//! Transfer concurrency, release barriers and crash recovery with manual ticks and memory ports.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
use super::*;
use crate::application::checkpoint::CheckpointService;
use crate::application::ports::{transfer_fingerprint, MutationReceiptLookup, RepositoryError};
use crate::application::replay_log::{AppliedTickRecord, EventLog};
use crate::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use crate::domain::class::ClassId;
use crate::domain::zone::{
    PlayerLoad, PlayerProgression, SessionGeneration, Speed, Vec2Fixed, ZoneBounds, ZoneCommand,
    ZoneId, ZoneInput, ZoneSeed,
};
use crate::domain::{
    AccountId, Character, CharacterId, CharacterName, DomainEvent, Position, Race,
};
use crate::infrastructure::{
    class_data::{load_classes, ClassSource},
    eventlog::InMemoryEventLog,
    memory::{InMemoryCharacterRepository, InMemorySessionAudit},
    rules_data::{load_rules, RulesSource},
    telemetry::Metrics,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use uuid::Uuid;

struct Repo {
    inner: Arc<InMemoryCharacterRepository>,
    hold: AtomicBool,
    lost_reply: AtomicBool,
    entered: Notify,
    release: Notify,
    requests: parking_lot::Mutex<Vec<(CharacterCheckpoint, Vec<DomainEvent>)>>,
}
impl Repo {
    fn new() -> Self {
        Self {
            inner: Arc::new(InMemoryCharacterRepository::default()),
            hold: AtomicBool::new(false),
            lost_reply: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
            requests: parking_lot::Mutex::new(Vec::new()),
        }
    }
}
#[async_trait::async_trait]
impl CharacterRepository for Repo {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.inner.get(id).await
    }
    async fn list_by_account(&self, id: AccountId) -> anyhow::Result<Vec<Character>> {
        self.inner.list_by_account(id).await
    }
    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fp: &str,
        c: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.inner.create_idempotent(key, fp, c).await
    }
    async fn load_for_admission(
        &self,
        id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        self.inner.load_for_admission(id).await
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.requests.lock().push((cp.clone(), events.to_vec()));
        self.entered.notify_one();
        while self.hold.load(Ordering::Acquire) {
            self.release.notified().await;
        }
        let result = self.inner.checkpoint(cp, events).await?;
        if self.lost_reply.swap(false, Ordering::AcqRel) {
            return Err(anyhow::anyhow!("committed response lost").into());
        }
        Ok(result)
    }
}
struct Gate(Arc<AtomicBool>);
impl TickGate for Gate {
    fn admit(&self, _: &AppliedTick) -> impl Future<Output = Result<(), GateError>> {
        std::future::ready(if self.0.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(GateError("held".into()))
        })
    }
}
struct Harness {
    zone: ZoneHandle,
    driver: ManualTickDriver,
    repo: Arc<Repo>,
    character: Character,
    gate: Arc<AtomicBool>,
}
impl Harness {
    async fn new() -> Self {
        let rules = load_rules(&RulesSource::embedded()).unwrap().rules;
        let registry = load_classes(&ClassSource::embedded()).unwrap().registry;
        let mut character = Character::create(
            AccountId::from_uuid(Uuid::from_u128(7)),
            CharacterName::new("Tester").unwrap(),
            Race::Human,
        );
        character.level = 40;
        character.xp = rules.xp_to_level(40).unwrap();
        character.position = Position { x: 126.0, y: 126.0 };
        character.class_state.token_tier_1_count = 1;
        character.class_state.token_tier_2_count = 1;
        let repo = Arc::new(Repo::new());
        repo.inner.insert_for_test(character.clone());
        let state = ZoneState::new(
            ZoneSeed {
                zone: ZoneId(77),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
        )
        .with_rules(rules)
        .with_classes(registry)
        .unwrap();
        let (ticks, driver) = manual_ticks();
        let gate = Arc::new(AtomicBool::new(true));
        let zone = ZoneActor::spawn_gated(state, ticks, Gate(gate.clone()));
        zone.checkpoints.install(CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        ));
        let h = Self {
            zone,
            driver,
            repo,
            character,
            gate,
        };
        h.spawn_character(&h.character, 1).await;
        h
    }
    async fn spawn_character(&self, c: &Character, generation: u64) {
        let load = PlayerLoad {
            progression: Some(Box::new(PlayerProgression {
                identity: c.identity(),
                class_state: c.class_state.clone(),
                max_cp: 0,
            })),
            checkpoint_revision: Some(0),
            level: c.level,
            xp: c.xp,
            ..PlayerLoad::fresh("human_fighter")
        };
        self.zone
            .send(ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: EntityId::from_uuid(c.id.as_uuid()),
                name: c.name.as_str().into(),
                pos: Vec2Fixed::from_tiles(126, 126),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(generation),
                load: Some(Box::new(load)),
            }))
            .unwrap();
        self.driver.step().await.unwrap();
    }
    fn request(
        &self,
        key: u128,
        target: u32,
    ) -> oneshot::Receiver<
        Result<crate::domain::character_progression::FrozenTransferResult, super::super::AppError>,
    > {
        self.request_for(&self.character, key, target, 1)
    }
    fn request_for(
        &self,
        c: &Character,
        key: u128,
        target: u32,
        generation: u64,
    ) -> oneshot::Receiver<
        Result<crate::domain::character_progression::FrozenTransferResult, super::super::AppError>,
    > {
        let entity = EntityId::from_uuid(c.id.as_uuid());
        self.zone
            .enqueue_class_transfer(ZoneInput {
                source: CommandSource::Session {
                    entity,
                    generation: SessionGeneration(generation),
                },
                seq: None,
                command: ZoneCommand::ChangeClass {
                    entity,
                    account: c.account_id,
                    request_key: Uuid::from_u128(key),
                    target: ClassId(target),
                },
            })
            .unwrap()
    }
    fn event_count(&self) -> usize {
        self.repo
            .inner
            .staged_events()
            .iter()
            .filter(|e| matches!(e, DomainEvent::CharacterClassChanged { .. }))
            .count()
    }
}
#[tokio::test]
async fn concurrent_same_key_returns_one_frozen_success_and_different_body_conflicts() {
    let h = Harness::new().await;
    let one = h.request(1, 1);
    let two = h.request(1, 1);
    let conflict = h.request(1, 4);
    h.driver.step().await.unwrap();
    let first = one.await.unwrap().unwrap();
    h.driver.step().await.unwrap();
    assert_eq!(two.await.unwrap().unwrap(), first);
    assert!(matches!(
        conflict.await.unwrap(),
        Err(super::super::AppError::IdempotencyConflict)
    ));
    assert_eq!(h.event_count(), 1);
    let saved = h
        .repo
        .load_for_admission(h.character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.class_state.token_tier_1_count, 0);
    assert_eq!(saved.class_state.successful_transfer_receipts.len(), 1);
}
#[tokio::test]
async fn competing_branches_and_account_key_reuse_on_another_character_have_one_winner() {
    let h = Harness::new().await;
    let mut other = h.character.clone();
    other.id = CharacterId::from_uuid(Uuid::from_u128(123));
    other.name = CharacterName::new("Second").unwrap();
    h.repo.inner.insert_for_test(other.clone());
    h.spawn_character(&other, 1).await;
    let first = h.request(1, 1);
    let sibling = h.request(2, 4);
    let cross = h.request_for(&other, 1, 1, 1);
    h.driver.step().await.unwrap();
    first.await.unwrap().unwrap();
    h.driver.step().await.unwrap();
    assert!(sibling.await.unwrap().is_err());
    h.driver.step().await.unwrap();
    assert!(matches!(cross.await.unwrap(), Err(super::super::AppError::IdempotencyConflict)));
    assert_eq!(h.event_count(), 1);
}
#[tokio::test]
async fn log_gate_then_checkpoint_gate_withhold_reply_stream_and_snapshot() {
    let h = Harness::new().await;
    let mut output = h.zone.subscribe();
    let mut reply = h.request(1, 1);
    h.gate.store(false, Ordering::Release);
    assert!(matches!(h.driver.step().await.unwrap(), TickOutcome::Held(_)));
    assert!(reply.try_recv().is_err());
    assert!(output.try_recv().is_err());
    assert!(h.repo.requests.lock().is_empty());
    h.repo.hold.store(true, Ordering::Release);
    h.gate.store(true, Ordering::Release);
    let driver = h.driver.clone();
    let step = tokio::spawn(async move { driver.step().await });
    h.repo.entered.notified().await;
    assert!(reply.try_recv().is_err());
    assert!(output.try_recv().is_err());
    assert!(tokio::time::timeout(Duration::from_millis(20), h.zone.snapshot())
        .await
        .is_err());
    h.repo.hold.store(false, Ordering::Release);
    h.repo.release.notify_one();
    step.await.unwrap().unwrap();
    reply.await.unwrap().unwrap();
    assert!(output
        .recv()
        .await
        .unwrap()
        .events
        .iter()
        .any(|e| matches!(e, crate::domain::zone::ZoneEvent::ClassChanged { .. })));
    assert_eq!(h.event_count(), 1);
}
#[tokio::test]
async fn lost_commit_response_retries_identical_request_and_dropped_rpc_can_retry_offline() {
    let h = Harness::new().await;
    h.repo.lost_reply.store(true, Ordering::Release);
    drop(h.request(1, 1));
    h.driver.step().await.unwrap();
    {
        let requests = h.repo.requests.lock();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
    }
    assert_eq!(h.event_count(), 1);
    let entity = EntityId::from_uuid(h.character.id.as_uuid());
    h.zone
        .send(ZoneInput {
            source: CommandSource::Session {
                entity,
                generation: SessionGeneration(1),
            },
            seq: None,
            command: ZoneCommand::Despawn { entity },
        })
        .unwrap();
    h.driver.step().await.unwrap();
    let key = IdempotencyKey::from_uuid(Uuid::from_u128(1));
    let lookup = h
        .repo
        .mutation_receipt_lookup(
            h.character.account_id,
            &key,
            &transfer_fingerprint(h.character.id, ClassId(1)),
        )
        .await
        .unwrap();
    let MutationReceiptLookup::Known(frozen) = lookup else {
        panic!("offline success lost")
    };
    assert_eq!(frozen.current_class_id, ClassId(1));
    assert_eq!(frozen.position_millitiles, [126_000, 126_000]);
    assert_eq!(frozen.token_tier_1_count, 0);
}
#[tokio::test]
async fn replacement_and_despawn_cannot_bypass_generation_or_save_barriers() {
    let h = Harness::new().await;
    let entity = EntityId::from_uuid(h.character.id.as_uuid());
    h.zone
        .send(ZoneInput::system(ZoneCommand::ReplaceSession {
            entity,
            generation: SessionGeneration(2),
        }))
        .unwrap();
    let stale = h.request(1, 1);
    h.driver.step().await.unwrap();
    h.driver.step().await.unwrap();
    assert!(stale.await.unwrap().is_err());
    assert_eq!(h.event_count(), 0);
    let fresh = h.request_for(&h.character, 2, 1, 2);
    h.zone
        .send(ZoneInput {
            source: CommandSource::Session {
                entity,
                generation: SessionGeneration(2),
            },
            seq: None,
            command: ZoneCommand::Despawn { entity },
        })
        .unwrap();
    h.driver.step().await.unwrap();
    fresh.await.unwrap().unwrap();
    assert_eq!(h.event_count(), 1);
    h.driver.step().await.unwrap();
    assert!(h
        .zone
        .snapshot()
        .await
        .unwrap()
        .entities
        .iter()
        .all(|e| e.id != entity));
}
#[tokio::test]
async fn stale_checkpoint_fences_actor_and_never_releases_transfer() {
    let h = Harness::new().await;
    let snapshot = h.zone.snapshot().await.unwrap();
    let mut service = h.zone.checkpoints.service().unwrap().lock_owned().await;
    service
        .flush(EntityId::from_uuid(h.character.id.as_uuid()))
        .await
        .unwrap();
    drop(service);
    // Another writer advances the committed revision after the actor's last acknowledgement.
    let cp = CharacterCheckpoint {
        character_id: h.character.id,
        revision_seen: 1,
        level: 40,
        xp: h.character.xp,
        hp: 1,
        mp: 0,
        alive: true,
        position: h.character.position,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::from_uuid(Uuid::from_u128(99))),
        class_state: None,
    };
    h.repo.checkpoint(&cp, &[]).await.unwrap();
    let mut output = h.zone.subscribe();
    let reply = h.request(1, 1);
    assert!(matches!(h.driver.step().await.unwrap(), TickOutcome::Held(_)));
    assert!(reply.await.unwrap().is_err());
    h.zone.stopped().await;
    assert!(h.zone.persistence_failed());
    assert!(output.try_recv().is_err());
    assert_eq!(h.event_count(), 0);
    assert!(h.zone.final_snapshot().is_none());
    assert!(snapshot.classes.is_some());
}
#[tokio::test]
async fn durable_transfer_prefix_recovers_class_tokens_receipt_and_outbox_before_admission() {
    let h = Harness::new().await;
    let snapshot = h.zone.snapshot().await.unwrap();
    let log = InMemoryEventLog::default();
    log.write_snapshot(&snapshot).await.unwrap();
    let mut state = ZoneState::from_snapshot(snapshot.clone()).unwrap();
    let entity = EntityId::from_uuid(h.character.id.as_uuid());
    let tick = state
        .run_tick(state.draft(vec![ZoneInput {
            source: CommandSource::Session {
                entity,
                generation: SessionGeneration(1),
            },
            seq: None,
            command: ZoneCommand::ChangeClass {
                entity,
                account: h.character.account_id,
                request_key: Uuid::from_u128(1),
                target: ClassId(1),
            },
        }]))
        .unwrap();
    log.append_applied(&AppliedTickRecord::from_applied(ZoneId(77), &tick))
        .await
        .unwrap();
    let mut recovery = CheckpointService::new(
        h.repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    );
    recovery.recover(&log, ZoneId(77), 1).await.unwrap();
    assert_eq!(h.event_count(), 1);
    recovery.recover(&log, ZoneId(77), 1).await.unwrap();
    assert_eq!(h.event_count(), 1);
    let saved = h
        .repo
        .load_for_admission(h.character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.class_state.current_class_id, ClassId(1));
    assert_eq!(saved.class_state.token_tier_1_count, 0);
    assert_eq!(saved.class_state.successful_transfer_receipts.len(), 1);
}

#[tokio::test]
async fn missing_admitted_lane_fences_before_draft_but_absent_actor_is_safe_rejection() {
    let h = Harness::new().await;
    let mut boundary = h.zone.snapshot().await.unwrap();
    boundary.checkpoints.clear();
    h.zone
        .checkpoints
        .service()
        .unwrap()
        .lock()
        .await
        .restore(&boundary);
    let mut output = h.zone.subscribe();
    let reply = h.request(1, 1);
    assert_eq!(h.driver.step().await.unwrap(), TickOutcome::Held(boundary.tick));
    assert!(reply.await.unwrap().is_err());
    h.zone.stopped().await;
    assert!(h.zone.persistence_failed());
    assert!(output.try_recv().is_err());
    assert!(h.repo.requests.lock().is_empty());
    assert_eq!(h.event_count(), 0);

    let h = Harness::new().await;
    let entity = EntityId::from_uuid(h.character.id.as_uuid());
    h.zone
        .send(ZoneInput::system(ZoneCommand::Despawn { entity }))
        .unwrap();
    h.driver.step().await.unwrap();
    let reply = h.request(1, 1);
    assert!(matches!(h.driver.step().await.unwrap(), TickOutcome::Ran(_)));
    assert!(reply.await.unwrap().is_err());
    assert!(!h.zone.persistence_failed());
    assert_eq!(h.event_count(), 0);
}

#[tokio::test]
#[ignore = "manual Phase2 digest/tick budget measurement with the complete immutable registry"]
#[allow(clippy::print_stderr)]
async fn full_registry_tick_and_digest_measurement() {
    let h = Harness::new().await;
    let mut state = ZoneState::from_snapshot(h.zone.snapshot().await.unwrap()).unwrap();
    let first = state.entities().next().unwrap().combat.as_ref().unwrap();
    let crate::domain::zone::CombatRole::Player { progression, .. } = &first.role else {
        panic!("player")
    };
    let progression = progression.clone();
    let inputs = (2_u128..=200)
        .map(|n| {
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: EntityId::from_uuid(Uuid::from_u128(n + 1000)),
                name: format!("Player{n}"),
                pos: Vec2Fixed::from_tiles(126, 126),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(PlayerLoad {
                    progression: progression.clone(),
                    level: 40,
                    xp: h.character.xp,
                    ..PlayerLoad::fresh("human_fighter")
                })),
            })
        })
        .collect();
    let spawned = state.run_tick(state.draft(inputs)).unwrap();
    assert!(spawned.dispositions.is_empty());
    assert_eq!(state.entity_count(), 200);
    let mut ticks = Vec::new();
    let mut digests = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        std::hint::black_box(state.state_digest());
        digests.push(start.elapsed().as_micros());
        let start = Instant::now();
        state.run_tick(state.draft(vec![])).unwrap();
        std::hint::black_box(state.snapshot());
        ticks.push(start.elapsed().as_micros());
    }
    ticks.sort_unstable();
    digests.sort_unstable();
    eprintln!("PHASE2_DEBUG_PERF players=200 samples=100 digest_p50_us={} digest_p99_us={} idle_tick_plus_checkpoint_projection_p50_us={} p99_us={}", digests[49], digests[98], ticks[49], ticks[98]);
}
