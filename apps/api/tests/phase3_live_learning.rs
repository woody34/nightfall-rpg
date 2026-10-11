//! Paid learning through a real experimental actor and atomic memory/Postgres lanes.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api::application::checkpoint::CheckpointService;
use nightfall_api::application::ports::*;
use nightfall_api::application::replay_log::{
    start_epoch, DurableTickGate, GateConfig, NoReplayMetrics,
};
use nightfall_api::application::session::{PlayerSpawn, SessionRegistry};
use nightfall_api::application::zone_actor::{
    manual_ticks, ManualTickDriver, TickOutcome, ZoneActor, ZoneHandle,
};
use nightfall_api::application::{AppError, IdempotencyKey};
use nightfall_api::domain::zone::skill::{SkillId, SkillRef};
use nightfall_api::domain::zone::*;
use nightfall_api::domain::{
    AccountId, Character, CharacterId, CharacterName, DomainEvent, Position, Race, SessionId,
};
use nightfall_api::infrastructure::class_data::{load_classes, ClassSource};
use nightfall_api::infrastructure::eventlog::InMemoryEventLog;
use nightfall_api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource};
use nightfall_api::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
use nightfall_api::infrastructure::telemetry::Metrics;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn rules() -> Arc<Phase3Rules> {
    Arc::new(load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap())
}
async fn seed<R: CharacterRepository>(repo: &R) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("PaidLearner").unwrap(),
        Race::Human,
    );
    c.level = 10;
    c.xp = load_rules(&RulesSource::embedded())
        .unwrap()
        .rules
        .xp_to_level(10)
        .unwrap();
    c.class_state.sp = 2000;
    c.position = Position { x: 126.0, y: 128.0 };
    repo.create_idempotent(&IdempotencyKey::new(), "live-learning", &c)
        .await
        .unwrap();
    c
}
async fn actor<R: CharacterRepository + LearningCheckpointRepository + 'static>(
    repo: Arc<R>,
) -> (ZoneHandle, ManualTickDriver, Arc<InMemoryEventLog>) {
    let state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(88),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        load_rules(&RulesSource::embedded()).unwrap().rules,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        (*rules()).clone(),
    )
    .unwrap();
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, gate);
    zone.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_learning(repo),
    );
    (zone, driver, log)
}
async fn admit<R: CharacterRepository + 'static>(
    repo: Arc<R>,
    zone: &ZoneHandle,
    driver: &ManualTickDriver,
    c: &Character,
) -> (SessionRegistry, SessionId) {
    let registry = SessionRegistry::with_characters(repo);
    let session = SessionId::new();
    registry
        .admit(
            zone,
            session,
            SessionGeneration(1),
            &PlayerSpawn {
                entity: EntityId::from_uuid(c.id.as_uuid()),
                name: c.name.to_string(),
                pos: Vec2Fixed::from_tiles(126, 128),
                speed: Speed::DEFAULT,
                load: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    (registry, session)
}
fn input(c: &Character, key: IdempotencyKey, rank: u32, generation: u64) -> ZoneInput {
    let entity = EntityId::from_uuid(c.id.as_uuid());
    ZoneInput {
        source: CommandSource::Session {
            entity,
            generation: SessionGeneration(generation),
        },
        seq: None,
        command: ZoneCommand::LearnSkill {
            entity,
            account: c.account_id,
            request_key: key.as_uuid(),
            skill_id: SkillId(3),
            skill_level: rank,
        },
    }
}
async fn parity<R: CharacterRepository + LearningCheckpointRepository + 'static>(repo: Arc<R>) {
    let c = seed(repo.as_ref()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let (registry, session) = admit(repo.clone(), &zone, &driver, &c).await;
    let mut output = zone.subscribe();
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let failed_key = IdempotencyKey::new();
    let wrong = zone
        .enqueue_learn_skill(input(&c, failed_key, 2, 1))
        .unwrap();
    driver.step().await.unwrap();
    assert!(matches!(wrong.await.unwrap(), Err(AppError::FailedPrecondition(_))));
    assert_eq!(
        repo.learn_receipt_lookup(
            c.account_id,
            &failed_key,
            &learn_skill_fingerprint(
                c.id,
                SkillRef {
                    id: SkillId(3),
                    rank: 2
                }
            )
        )
        .await
        .unwrap(),
        LearnReceiptLookup::Unknown
    );
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before);
    let key = IdempotencyKey::new();
    let first = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    let duplicate = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    let other = zone
        .enqueue_learn_skill(input(&c, IdempotencyKey::new(), 1, 1))
        .unwrap();
    driver.step().await.unwrap();
    let result = first.await.unwrap().unwrap();
    assert_eq!(
        result.skill,
        SkillRef {
            id: SkillId(3),
            rank: 1
        }
    );
    assert_eq!(result.sp_remaining, 2000u64.checked_sub(result.sp_spent).unwrap());
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(saved.revision, before.revision.saturating_add(1));
    assert_eq!(saved.class_state.sp, result.sp_remaining);
    assert_eq!(saved.hp, Some(result.hp));
    assert_eq!(saved.mp, Some(result.mp));
    assert_eq!(saved.class_state.cp, result.cp);
    driver.step().await.unwrap();
    assert_eq!(duplicate.await.unwrap().unwrap(), result);
    assert!(matches!(other.await.unwrap(), Err(AppError::FailedPrecondition(_))));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), saved);
    let conflict = zone.enqueue_learn_skill(input(&c, key, 2, 1)).unwrap();
    driver.step().await.unwrap();
    assert!(matches!(conflict.await.unwrap(), Err(AppError::IdempotencyConflict)));
    let stale = zone
        .enqueue_learn_skill(input(&c, IdempotencyKey::new(), 2, 0))
        .unwrap();
    driver.step().await.unwrap();
    assert!(matches!(stale.await.unwrap(), Err(AppError::FailedPrecondition(_))));
    // Receipt preflight precedes stale generation and offline eligibility.
    let leave =
        registry.leave(&zone, session, EntityId::from_uuid(c.id.as_uuid()), SessionGeneration(1));
    tokio::pin!(leave);
    assert!(tokio::time::timeout(Duration::from_millis(1), &mut leave)
        .await
        .is_err());
    driver.step().await.unwrap();
    leave.await.unwrap();
    let retry = zone.enqueue_learn_skill(input(&c, key, 1, 0)).unwrap();
    driver.step().await.unwrap();
    assert_eq!(retry.await.unwrap().unwrap(), result);
    let final_saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_learning(repo.clone());
    recovery.recover(log.as_ref(), ZoneId(88), 1).await.unwrap();
    recovery.recover(log.as_ref(), ZoneId(88), 1).await.unwrap();
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), final_saved);
    let mut purchases = 0_usize;
    while let Ok(tick) = output.try_recv() {
        purchases = purchases.saturating_add(
            tick.events
                .iter()
                .filter(|e| matches!(e, ZoneEvent::SkillLearned { .. }))
                .count(),
        );
    }
    assert_eq!(purchases, 1);
    drop(driver);
    zone.stopped().await;
}
#[tokio::test]
async fn memory_real_purchase_duplicate_conflict_offline_and_recovery() {
    parity(Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()))).await;
}
#[tokio::test]
async fn postgres_real_purchase_duplicate_conflict_offline_and_recovery() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    parity(Arc::new(PgCharacterRepository::new(pool).with_phase3_rules(rules()))).await;
}
struct Probe {
    inner: InMemoryCharacterRepository,
    hold: AtomicBool,
    lose: AtomicBool,
    reject: AtomicBool,
    entered: Notify,
    release: Notify,
    requests:
        parking_lot::Mutex<Vec<(CharacterCheckpoint, Vec<DomainEvent>, SuccessfulLearnReceipt)>>,
}
#[async_trait::async_trait]
impl CharacterRepository for Probe {
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
        self.inner.checkpoint(cp, events).await
    }
}
#[async_trait::async_trait]
impl LearnReceiptRepository for Probe {
    async fn learn_receipt_lookup(
        &self,
        a: AccountId,
        key: &IdempotencyKey,
        fp: &str,
    ) -> anyhow::Result<LearnReceiptLookup> {
        self.inner.learn_receipt_lookup(a, key, fp).await
    }
}
#[async_trait::async_trait]
impl LearningCheckpointRepository for Probe {
    async fn checkpoint_learning(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        r: &SuccessfulLearnReceipt,
    ) -> Result<LearningCheckpointOutcome, CheckpointError> {
        self.requests
            .lock()
            .push((cp.clone(), events.to_vec(), r.clone()));
        if self.hold.load(Ordering::Acquire) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.reject.load(Ordering::Acquire) {
            return Err(CheckpointError::Constraint("injected_crash_before_commit".into()));
        }
        let result = self.inner.checkpoint_learning(cp, events, r).await?;
        if self.lose.swap(false, Ordering::AcqRel) {
            return Err(anyhow::anyhow!("lost learning commit acknowledgement").into());
        }
        Ok(result)
    }
}
#[tokio::test]
async fn reply_and_output_wait_for_ack_with_identical_frozen_retry() {
    let repo = Arc::new(Probe {
        inner: InMemoryCharacterRepository::default().with_phase3_rules(rules()),
        hold: AtomicBool::new(true),
        lose: AtomicBool::new(true),
        reject: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
        requests: parking_lot::Mutex::default(),
    });
    let c = seed(repo.as_ref()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let (_registry, _session) = admit(repo.clone(), &zone, &driver, &c).await;
    let mut output = zone.subscribe();
    let mut reply = zone
        .enqueue_learn_skill(input(&c, IdempotencyKey::new(), 1, 1))
        .unwrap();
    let step_driver = driver.clone();
    let step = tokio::spawn(async move { step_driver.step().await });
    repo.entered.notified().await;
    assert!(log.subjects().iter().any(|(_, s)| s.ends_with(".applied")));
    assert!(reply.try_recv().is_err());
    assert!(output.try_recv().is_err());
    assert!(tokio::time::timeout(Duration::from_millis(10), zone.snapshot())
        .await
        .is_err());
    repo.hold.store(false, Ordering::Release);
    repo.release.notify_one();
    assert!(matches!(step.await.unwrap().unwrap(), TickOutcome::Ran(_)));
    let result = reply.await.unwrap().unwrap();
    {
        let requests = repo.requests.lock();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[0].2.result, result);
        assert!(requests[0].1.iter().any(|event| matches!(
            event,
            DomainEvent::CharacterSkillLearned {
                skill_id: 3,
                skill_level: 1,
                ..
            }
        )));
    }
    assert_eq!(
        repo.inner
            .staged_events()
            .iter()
            .filter(|e| matches!(e, DomainEvent::CharacterSkillLearned { .. }))
            .count(),
        1
    );
    drop(driver);
    zone.stopped().await;
}

#[tokio::test]
async fn logged_purchase_with_stale_checkpoint_remains_fenced_during_recovery() {
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.as_ref()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let (_registry, _session) = admit(repo.clone(), &zone, &driver, &c).await;
    let mut output = zone.subscribe();
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let newer = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: before.revision,
        level: before.level,
        xp: before.xp,
        hp: before.hp.unwrap(),
        mp: before.mp.unwrap(),
        alive: true,
        position: before.position,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(before.class_state.clone()),
        phase3: before.phase3.clone(),
    };
    repo.checkpoint(&newer, &[]).await.unwrap();
    let expected = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let key = IdempotencyKey::new();
    let reply = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(1)));
    assert!(matches!(reply.await.unwrap(), Err(AppError::Unavailable(_))));
    zone.stopped().await;
    assert!(output.try_recv().is_err());
    assert!(zone.snapshot().await.is_err());
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), expected);
    assert_eq!(
        repo.learn_receipt_lookup(
            c.account_id,
            &key,
            &learn_skill_fingerprint(
                c.id,
                SkillRef {
                    id: SkillId(3),
                    rank: 1
                }
            )
        )
        .await
        .unwrap(),
        LearnReceiptLookup::Unknown
    );
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_learning(repo.clone());
    assert!(
        recovery.recover(log.as_ref(), ZoneId(88), 1).await.is_err(),
        "newer revision remains fenced during recovery"
    );
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), expected);
}

#[tokio::test]
async fn purchase_and_queued_departure_use_separate_committed_boundaries() {
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.as_ref()).await;
    let (zone, driver, _log) = actor(repo.clone()).await;
    let (_registry, _session) = admit(repo.clone(), &zone, &driver, &c).await;
    let key = IdempotencyKey::new();
    let reply = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    let entity = EntityId::from_uuid(c.id.as_uuid());
    zone.send(ZoneInput {
        source: CommandSource::Session {
            entity,
            generation: SessionGeneration(1),
        },
        seq: None,
        command: ZoneCommand::Despawn { entity },
    })
    .unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(1)));
    let result = reply.await.unwrap().unwrap();
    assert_eq!(
        repo.learn_receipt_lookup(c.account_id, &key, &learn_skill_fingerprint(c.id, result.skill))
            .await
            .unwrap(),
        LearnReceiptLookup::Known(result)
    );
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(2)));
    assert!(zone.snapshot().await.unwrap().entities.is_empty());
    drop(driver);
    zone.stopped().await;
}

#[tokio::test]
async fn crash_after_log_before_purchase_commit_recovers_receipt_and_outbox_once() {
    let repo = Arc::new(Probe {
        inner: InMemoryCharacterRepository::default().with_phase3_rules(rules()),
        hold: AtomicBool::new(false),
        lose: AtomicBool::new(false),
        reject: AtomicBool::new(true),
        entered: Notify::new(),
        release: Notify::new(),
        requests: parking_lot::Mutex::default(),
    });
    let c = seed(repo.as_ref()).await;
    let (zone, driver, log) = actor(repo.clone()).await;
    let (_registry, _session) = admit(repo.clone(), &zone, &driver, &c).await;
    let mut output = zone.subscribe();
    let key = IdempotencyKey::new();
    let reply = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(1)));
    assert!(matches!(reply.await.unwrap(), Err(AppError::Unavailable(_))));
    zone.stopped().await;
    assert!(output.try_recv().is_err());
    assert_eq!(
        repo.inner
            .staged_events()
            .iter()
            .filter(|e| matches!(e, DomainEvent::CharacterSkillLearned { .. }))
            .count(),
        0
    );
    repo.reject.store(false, Ordering::Release);
    let mut recovery = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_learning(repo.clone());
    recovery.recover(log.as_ref(), ZoneId(88), 1).await.unwrap();
    let fp = learn_skill_fingerprint(
        c.id,
        SkillRef {
            id: SkillId(3),
            rank: 1,
        },
    );
    let result = repo
        .learn_receipt_lookup(c.account_id, &key, &fp)
        .await
        .unwrap();
    assert!(matches!(result, LearnReceiptLookup::Known(_)));
    recovery.recover(log.as_ref(), ZoneId(88), 1).await.unwrap();
    assert_eq!(
        repo.learn_receipt_lookup(c.account_id, &key, &fp)
            .await
            .unwrap(),
        result
    );
    assert_eq!(
        repo.inner
            .staged_events()
            .iter()
            .filter(|e| matches!(e, DomainEvent::CharacterSkillLearned { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn typed_learning_lane_cannot_bypass_the_durable_log_requirement() {
    let repo = Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(rules()));
    let c = seed(repo.as_ref()).await;
    let (durable, driver, _log) = actor(repo.clone()).await;
    let (_registry, _session) = admit(repo.clone(), &durable, &driver, &c).await;
    let snapshot = durable.snapshot().await.unwrap();
    drop(driver);
    durable.stopped().await;
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(ZoneState::from_snapshot(snapshot).unwrap(), ticks);
    zone.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_learning(repo.clone()),
    );
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let mut output = zone.subscribe();
    let key = IdempotencyKey::new();
    let reply = zone.enqueue_learn_skill(input(&c, key, 1, 1)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(1)));
    assert!(matches!(reply.await.unwrap(), Err(AppError::Unavailable(_))));
    zone.stopped().await;
    assert!(output.try_recv().is_err());
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before);
    assert_eq!(
        repo.learn_receipt_lookup(
            c.account_id,
            &key,
            &learn_skill_fingerprint(
                c.id,
                SkillRef {
                    id: SkillId(3),
                    rank: 1
                }
            )
        )
        .await
        .unwrap(),
        LearnReceiptLookup::Unknown
    );
}
