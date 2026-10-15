#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
use super::*;
use crate::application::ports::{CheckpointError, RepositoryError};
use crate::application::{CreateOutcome, ProgressionState};
use crate::domain::zone::{
    DeathFact, PlayerLoad, SessionGeneration, Speed, Vec2Fixed, ZoneBounds, ZoneEvent, ZoneInput,
    ZoneSeed, ZoneState,
};
use crate::domain::{AccountId, Character, CharacterName, Race};
use crate::infrastructure::memory::{
    AuditedFrame, InMemoryCharacterRepository, InMemorySessionAudit,
};
use crate::infrastructure::{rules_data, telemetry::Metrics};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Harness {
    repo: Arc<InMemoryCharacterRepository>,
    audit: Arc<InMemorySessionAudit>,
    service: CheckpointService,
    state: ZoneState,
    id: EntityId,
    session: SessionId,
}
impl Harness {
    async fn new() -> Self {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let c = Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Tester").unwrap(),
            Race::Human,
        );
        repo.insert_for_test(c.clone());
        let audit = Arc::new(InMemorySessionAudit::default());
        let mut service =
            CheckpointService::new(repo.clone(), audit.clone(), Arc::new(Metrics::detached()));
        let id = EntityId::from_uuid(c.id.as_uuid());
        let session = SessionId::new();
        service.bind_session(id, session);
        let mut state = ZoneState::new(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(100, 100)).unwrap(),
            0,
        )
        .with_rules(
            rules_data::load_rules(&rules_data::RulesSource::embedded())
                .unwrap()
                .rules,
        );
        let tick = state
            .run_tick(state.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: id,
                name: "Tester".into(),
                pos: Vec2Fixed::from_tiles(10, 10),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(PlayerLoad {
                    phase4: None,
                    phase3: None,
                    checkpoint_revision: Some(0),
                    ..PlayerLoad::fresh("human_fighter")
                })),
            })]))
            .unwrap();
        service.admitted(&tick, &state.snapshot()).await.unwrap();
        Self {
            repo,
            audit,
            service,
            state,
            id,
            session,
        }
    }
    async fn step(&mut self) {
        let tick = self.state.run_tick(self.state.draft(vec![])).unwrap();
        self.service
            .admitted(&tick, &self.state.snapshot())
            .await
            .unwrap();
    }
    async fn stored(&self) -> ProgressionState {
        self.repo
            .load_for_admission(self.id.as_uuid().into())
            .await
            .unwrap()
            .unwrap()
    }
    async fn transition(&mut self, died: bool) -> anyhow::Result<AppliedTick> {
        let mut tick = self.state.run_tick(self.state.draft(vec![])).unwrap();
        let mut snapshot = self.state.snapshot();
        let e = snapshot
            .entities
            .iter_mut()
            .find(|e| e.id == self.id)
            .unwrap();
        let c = e.combat.as_mut().unwrap();
        c.hp = if died { 0 } else { 50 };
        e.targeting.dead = died;
        if let CombatRole::Player { xp, .. } = &mut c.role {
            *xp = if died { 41 } else { 70 };
        }
        let delta = ProgressionDelta {
            tick: tick.tick,
            entity: self.id,
            level_before: if died { 2 } else { 1 },
            xp_before: 60,
            level: if died { 1 } else { 2 },
            xp: if died { 41 } else { 70 },
            hp: c.hp,
            mp: c.mp,
            alive: !died,
            pos: e.pos,
            xp_gained: if died { 0 } else { 10 },
            levels_gained: if died { vec![] } else { vec![2] },
            died: died.then(|| DeathFact {
                killer: None,
                killer_template: Some("keltir".into()),
                xp_lost: 29,
            }),
            respawned: false,
        };
        // Projection consumes the full delta; the supplied boundary agrees on resources.
        tick.events.push(ZoneEvent::Progression(delta));
        self.service.admitted(&tick, &snapshot).await?;
        Ok(tick)
    }
}

#[tokio::test]
async fn cadence_is_per_player_fifty_ticks_and_logout_flushes_resources_position_and_ack() {
    let mut h = Harness::new().await;
    for _ in 0..49 {
        h.step().await;
    }
    assert_eq!(h.stored().await.revision, 0);
    h.step().await;
    assert_eq!(h.stored().await.revision, 1);
    let tick = h
        .state
        .run_tick(h.state.draft(vec![ZoneInput::system(ZoneCommand::MoveTo {
            entity: h.id,
            dest: Vec2Fixed::from_tiles(11, 10),
        })]))
        .unwrap();
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    assert_eq!(h.stored().await.revision, 1);
    h.service.flush(h.id).await.unwrap();
    let saved = h.stored().await;
    assert_eq!(saved.revision, 2);
    assert!(saved.position.x > 10.0);
    assert!(saved.hp.is_some() && saved.mp.is_some() && saved.alive);
    h.service.flush(h.id).await.unwrap();
    assert_eq!(h.stored().await.revision, 2);
    assert_eq!(
        h.audit
            .frames(h.session)
            .iter()
            .filter(|f| matches!(f, AuditedFrame::Checkpoint(_)))
            .count(),
        2
    );
}

#[tokio::test]
async fn death_and_level_are_immediate_and_duplicate_batches_do_not_duplicate_events() {
    let mut h = Harness::new().await;
    let tick = h.transition(false).await.unwrap();
    assert_eq!(h.stored().await.revision, 1);
    h.service
        .admitted(&tick, &h.state.snapshot())
        .await
        .unwrap();
    assert_eq!(h.repo.staged_events().len(), 1);
    h.transition(true).await.unwrap();
    let p = h.stored().await;
    assert_eq!((p.revision, p.xp, p.hp, p.alive), (2, 41, Some(0), false));
    assert!(
        matches!(&h.repo.staged_events()[1], DomainEvent::CharacterDied { xp_lost: 29, level: 1, metadata, .. } if metadata.sequence == (2,0))
    );
}

#[tokio::test]
async fn stale_revision_never_overwrites_newer_state_or_stages_events() {
    let mut h = Harness::new().await;
    let mut newer = h.service.players[&h.id].latest.clone().unwrap();
    newer.xp = 999;
    newer.idempotency.1 = IdempotencyKey::from_uuid(Uuid::now_v7());
    h.repo.checkpoint(&newer, &[]).await.unwrap();
    assert!(h.transition(true).await.is_err());
    assert_eq!(h.stored().await.xp, 999);
    assert!(h.repo.staged_events().is_empty());
    assert!(h.service.players[&h.id].fenced);
}

struct Flaky {
    inner: Arc<InMemoryCharacterRepository>,
    attempts: AtomicUsize,
    after_commit: bool,
    gate: Option<(Arc<tokio::sync::Semaphore>, Arc<tokio::sync::Semaphore>)>,
}
#[async_trait::async_trait]
impl CharacterRepository for Flaky {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.inner.get(id).await
    }
    async fn list_by_account(&self, id: AccountId) -> anyhow::Result<Vec<Character>> {
        self.inner.list_by_account(id).await
    }
    async fn create_idempotent(
        &self,
        k: &IdempotencyKey,
        f: &str,
        c: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.inner.create_idempotent(k, f, c).await
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
        if let Some((entered, release)) = &self.gate {
            entered.add_permits(1);
            release.acquire().await.unwrap().forget();
        }
        let first = self.attempts.fetch_add(1, Ordering::SeqCst) == 0;
        if first && !self.after_commit {
            return Err(anyhow::anyhow!("before transaction").into());
        }
        let result = self.inner.checkpoint(cp, events).await;
        if first {
            return Err(anyhow::anyhow!("lost commit response").into());
        }
        result
    }
}

#[tokio::test]
async fn failures_before_and_after_commit_retry_identical_body_once_and_count() {
    for after_commit in [false, true] {
        let mut h = Harness::new().await;
        let repo = Arc::new(Flaky {
            inner: h.repo.clone(),
            attempts: AtomicUsize::new(0),
            after_commit,
            gate: None,
        });
        let metrics = Metrics::detached();
        h.service.repo = repo.clone();
        h.service.metrics = Arc::new(metrics.clone());
        h.transition(false).await.unwrap();
        assert_eq!(repo.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(h.stored().await.revision, 1);
        assert_eq!(h.repo.staged_events().len(), 1);
        let exposition = metrics.render().unwrap();
        assert!(exposition
            .lines()
            .any(|line| line.starts_with("nightfall_checkpoint_failures_total")
                && line.ends_with(" 1")));
        assert!(exposition.lines().any(|line| line
            .starts_with("nightfall_checkpoint_lag_seconds")
            && line.ends_with(" 0")));
        assert!(format!("{:?}", h.audit.frames(h.session)).contains("Checkpoint"));
    }
}

#[test]
fn multiple_levels_and_death_keep_stable_ids_order_and_penalty() {
    let delta = ProgressionDelta {
        tick: Tick(9),
        entity: EntityId::from_uuid(Uuid::nil()),
        level_before: 1,
        xp_before: 60,
        level: 2,
        xp: 340,
        hp: 0,
        mp: 12,
        alive: false,
        pos: Vec2Fixed::from_tiles(1, 2),
        xp_gained: 330,
        levels_gained: vec![2, 3],
        died: Some(DeathFact {
            killer: None,
            killer_template: Some("keltir".into()),
            xp_lost: 50,
        }),
        respawned: false,
    };
    let events = domain_events(ZoneId(1), 2, 7, &delta);
    assert_eq!(events, domain_events(ZoneId(1), 2, 7, &delta));
    let metadata: Vec<_> = events
        .iter()
        .map(|e| match e {
            DomainEvent::CharacterLeveled { metadata, .. }
            | DomainEvent::CharacterDied { metadata, .. } => metadata,
            DomainEvent::ItemGranted { fact }
            | DomainEvent::ItemEquipped { fact }
            | DomainEvent::ItemUnequipped { fact }
            | DomainEvent::ItemConsumed { fact }
            | DomainEvent::ItemEnchanted { fact }
            | DomainEvent::ItemDestroyed { fact }
            | DomainEvent::ItemRefined { fact }
            | DomainEvent::ItemRefineCancelled { fact }
            | DomainEvent::ItemExchanged { fact } => &fact.metadata,
            DomainEvent::CharacterCreated { .. }
            | DomainEvent::CharacterClassChanged { .. }
            | DomainEvent::CharacterTokenGranted { .. }
            | DomainEvent::CharacterSkillLearned { .. } => {
                unreachable!()
            },
        })
        .collect();
    assert_eq!(
        metadata.iter().map(|m| m.sequence).collect::<Vec<_>>(),
        [(7, 0), (7, 1), (7, 2)]
    );
    assert_ne!(metadata[0].event_id, metadata[1].event_id);
    assert_ne!(events, domain_events(ZoneId(1), 3, 7, &delta));
}

#[tokio::test]
async fn held_ticks_never_enter_checkpoint_lane_and_lifecycle_flush_precedes_despawn() {
    use crate::application::zone_actor::{
        manual_ticks, GateError, TickGate, TickOutcome, ZoneActor,
    };
    struct Hold(Arc<std::sync::atomic::AtomicBool>);
    impl TickGate for Hold {
        fn admit(
            &self,
            _tick: &AppliedTick,
        ) -> impl std::future::Future<Output = Result<(), GateError>> + Send {
            std::future::ready(if self.0.load(Ordering::SeqCst) {
                Err(GateError("held".into()))
            } else {
                Ok(())
            })
        }
    }
    let h = Harness::new().await;
    let held = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(h.state, ticks, Hold(held.clone()));
    zone.checkpoints.install(h.service);
    for _ in 0..55 {
        assert!(matches!(driver.step().await.unwrap(), TickOutcome::Held(_)));
    }
    assert_eq!(
        h.repo
            .load_for_admission(h.id.as_uuid().into())
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    held.store(false, Ordering::SeqCst);
    driver.step().await.unwrap();
    zone.send(ZoneInput::system(ZoneCommand::Despawn { entity: h.id }))
        .unwrap();
    // Even if the despawn's log admission is held, its prior admitted state is saved first.
    held.store(true, Ordering::SeqCst);
    assert!(matches!(driver.step().await.unwrap(), TickOutcome::Held(_)));
    assert_eq!(
        h.repo
            .load_for_admission(h.id.as_uuid().into())
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    held.store(false, Ordering::SeqCst);
    driver.step().await.unwrap();
    assert!(zone.snapshot().await.unwrap().entities.is_empty());
}

#[tokio::test]
async fn intent_then_disconnect_saves_the_last_admitted_position() {
    use crate::application::zone_actor::{manual_ticks, ZoneActor};
    let h = Harness::new().await;
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(h.state, ticks);
    zone.checkpoints.install(h.service);
    zone.send(ZoneInput::system(ZoneCommand::MoveTo {
        entity: h.id,
        dest: Vec2Fixed::from_tiles(11, 10),
    }))
    .unwrap();
    zone.send(ZoneInput::system(ZoneCommand::Despawn { entity: h.id }))
        .unwrap();
    driver.step().await.unwrap();
    assert_eq!(zone.stats().borrow().commands_deferred, 1);
    driver.step().await.unwrap();
    let saved = h
        .repo
        .load_for_admission(h.id.as_uuid().into())
        .await
        .unwrap()
        .unwrap();
    assert!(saved.position.x > 10.0);
    assert_eq!(saved.revision, 1);
    assert!(zone.snapshot().await.unwrap().entities.is_empty());
}

#[tokio::test]
async fn mid_fight_actor_snapshot_restores_checkpoint_cadence_revision_and_pending_events() {
    use crate::application::zone_actor::{manual_ticks, ZoneActor};
    use crate::domain::zone::NpcCombat;
    let mut h = Harness::new().await;
    let def = crate::infrastructure::zone_data::parse_zone(
        crate::infrastructure::zone_data::TEST_ZONE_TOML,
    )
    .unwrap();
    let rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules;
    let npc = NpcCombat::from_template(&rules, &def.npc_templates[0]).unwrap();
    let batch = h
        .state
        .run_tick(h.state.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Keltir".into(),
            pos: Vec2Fixed::from_tiles(11, 10),
            speed: Speed::DEFAULT,
            combat: Some(Box::new(npc)),
        })]))
        .unwrap();
    h.service
        .admitted(&batch, &h.state.snapshot())
        .await
        .unwrap();
    let target = h
        .state
        .snapshot()
        .entities
        .iter()
        .find(|e| e.id != h.id)
        .unwrap()
        .id;
    let batch = h
        .state
        .run_tick(h.state.draft(vec![
            ZoneInput::system(ZoneCommand::SetTarget {
                entity: h.id,
                target: Some(target),
            }),
            ZoneInput::system(ZoneCommand::Attack { entity: h.id }),
        ]))
        .unwrap();
    h.service
        .admitted(&batch, &h.state.snapshot())
        .await
        .unwrap();
    assert!(h
        .state
        .snapshot()
        .entities
        .iter()
        .any(|e| e.combat.as_ref().is_some_and(|c| c.swing.is_some())));
    // Pending facts normally flush immediately; explicitly retain one to test the full lane payload.
    let pending = DomainEvent::CharacterLeveled {
        character_id: h.id.as_uuid().into(),
        level: 2,
        metadata: EventMetadata {
            event_id: Uuid::from_u128(19),
            sequence: (1, 0),
        },
    };
    h.service
        .players
        .get_mut(&h.id)
        .unwrap()
        .events
        .push(pending.clone());
    let (ticks, _driver) = manual_ticks();
    let actor = ZoneActor::spawn(h.state, ticks);
    actor.checkpoints.install(h.service);
    let snapshot = actor.snapshot().await.unwrap();
    assert!(snapshot.checkpoints[0].dirty);
    let snapshot: ZoneSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let (ticks, driver) = manual_ticks();
    let restored = ZoneActor::spawn(ZoneState::from_snapshot(snapshot.clone()).unwrap(), ticks);
    restored.checkpoints.install(CheckpointService::new(
        h.repo.clone(),
        h.audit,
        Arc::new(Metrics::detached()),
    ));
    assert_eq!(restored.snapshot().await.unwrap().checkpoints, snapshot.checkpoints);
    // Continue combat, then force the same lifecycle save as a disconnect.
    for _ in 0..5 {
        driver.step().await.unwrap();
    }
    restored
        .send(ZoneInput::system(ZoneCommand::Despawn { entity: h.id }))
        .unwrap();
    driver.step().await.unwrap();
    let loaded = h
        .repo
        .load_for_admission(h.id.as_uuid().into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.revision, 1);
    assert_eq!(h.repo.staged_events(), vec![pending]);
}

#[tokio::test]
async fn refreshed_baseline_preserves_replay_start_and_shutdown_closes_durable_index() {
    use crate::application::replay_log::{EventLog, ZoneSnapshotStore};
    use crate::infrastructure::eventlog::{InMemoryEventLog, InMemoryZoneSnapshotStore};
    let mut h = Harness::new().await;
    let log = Arc::new(InMemoryEventLog::default());
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    let initial = h.state.snapshot();
    log.write_snapshot(&initial).await.unwrap();
    h.service = h.service.with_durability(log.clone(), store.clone());
    h.service.refreshed = Instant::now() - Duration::from_hours(24);
    h.step().await;
    let fresh = log
        .read_recovery_snapshot(initial.seed.zone, initial.seed.epoch)
        .await
        .unwrap()
        .unwrap();
    assert!(fresh.snapshot.tick > initial.tick);
    assert!(!fresh.snapshot.checkpoints.is_empty());
    assert_eq!(
        log.read_snapshot(initial.seed.zone, initial.seed.epoch)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .tick,
        initial.tick
    );
    assert_eq!(store.unresolved(initial.seed.zone).await.unwrap().len(), 1);
    store
        .set_first_seq(
            initial.seed.zone,
            initial.seed.epoch,
            crate::application::replay_log::Seq(2),
        )
        .await
        .unwrap();
    h.service.shutdown(&h.state.snapshot()).await.unwrap();
    let final_snapshot = log
        .read_recovery_snapshot(initial.seed.zone, initial.seed.epoch)
        .await
        .unwrap()
        .unwrap();
    assert!(
        final_snapshot.seq > fresh.seq,
        "a same-tick shutdown flush must not dedupe against the periodic snapshot"
    );
    assert!(!final_snapshot.snapshot.checkpoints[0].dirty);
    assert_eq!(final_snapshot.snapshot.checkpoints[0].revision, 1);
    let row = store
        .get(initial.seed.zone, initial.seed.epoch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.snapshot,
        crate::application::replay_log::encode_snapshot(&final_snapshot.snapshot).unwrap()
    );
    assert_eq!(row.first_seq, Some(crate::application::replay_log::Seq(2)));
    assert!(store
        .unresolved(initial.seed.zone)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(h.stored().await.revision, 1);
}

#[tokio::test]
async fn restored_cadence_saves_on_the_original_fiftieth_tick() {
    let mut h = Harness::new().await;
    for _ in 0..49 {
        h.step().await;
    }
    let mut snapshot = h.state.snapshot();
    h.service.snapshot(&mut snapshot);
    let mut service =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()));
    service.restore(&snapshot);
    h.service = service;
    h.state = ZoneState::from_snapshot(snapshot).unwrap();
    assert_eq!(h.stored().await.revision, 0);
    h.step().await;
    assert_eq!(h.stored().await.revision, 1);
}

#[tokio::test]
async fn missing_tail_is_refused_even_when_a_fresh_baseline_survives() {
    use crate::application::replay_log::ZoneSnapshotStore;
    use crate::infrastructure::eventlog::{InMemoryEventLog, InMemoryZoneSnapshotStore};
    let mut h = Harness::new().await;
    let log = Arc::new(InMemoryEventLog::default());
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    let snapshot = h.state.snapshot();
    h.service = h.service.with_durability(log.clone(), store.clone());
    h.service.refresh(&snapshot).await.unwrap();
    // D134: the index is written only after the log acknowledged the tick, so an indexed tick
    // missing from the log is log loss and fails closed.
    store
        .recording(snapshot.seed.zone, snapshot.seed.epoch, snapshot.tick)
        .await
        .unwrap();
    let mut recovery = CheckpointService::new(h.repo, h.audit, Arc::new(Metrics::detached()));
    let error = recovery
        .recover_indexed(log.as_ref(), store.as_ref(), snapshot.seed.zone)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ends before indexed tick"));
    assert_eq!(store.unresolved(snapshot.seed.zone).await.unwrap().len(), 1);
}

#[tokio::test]
async fn recovery_commits_critical_facts_retained_in_a_baseline_without_later_records() {
    use crate::infrastructure::eventlog::{InMemoryEventLog, InMemoryZoneSnapshotStore};
    let mut h = Harness::new().await;
    let event = DomainEvent::CharacterLeveled {
        character_id: h.id.as_uuid().into(),
        level: 2,
        metadata: EventMetadata {
            event_id: Uuid::from_u128(20),
            sequence: (1, 0),
        },
    };
    h.service
        .players
        .get_mut(&h.id)
        .unwrap()
        .events
        .push(event.clone());
    let log = Arc::new(InMemoryEventLog::default());
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    log.write_snapshot(&h.state.snapshot()).await.unwrap();
    h.service = h.service.with_durability(log.clone(), store.clone());
    h.service.refresh(&h.state.snapshot()).await.unwrap();
    let mut recovery =
        CheckpointService::new(h.repo.clone(), h.audit.clone(), Arc::new(Metrics::detached()));
    recovery
        .recover_indexed(log.as_ref(), store.as_ref(), h.state.seed().zone)
        .await
        .unwrap();
    assert_eq!(h.stored().await.revision, 1);
    assert_eq!(h.repo.staged_events(), vec![event]);
}

#[tokio::test]
async fn recovery_refuses_to_close_an_epoch_without_its_original_prefix() {
    use crate::application::replay_log::ZoneSnapshotStore;
    use crate::infrastructure::eventlog::{InMemoryEventLog, InMemoryZoneSnapshotStore};
    let mut h = Harness::new().await;
    let log = Arc::new(InMemoryEventLog::default());
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    h.service = h.service.with_durability(log.clone(), store.clone());
    h.service.refresh(&h.state.snapshot()).await.unwrap();
    let mut recovery = CheckpointService::new(h.repo, h.audit, Arc::new(Metrics::detached()));
    let error = recovery
        .recover_indexed(log.as_ref(), store.as_ref(), h.state.seed().zone)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("epoch-start snapshot missing"));
    assert_eq!(store.unresolved(h.state.seed().zone).await.unwrap().len(), 1);
    assert!(!matches!(
        log.epoch_status(h.state.seed().zone, h.state.seed().epoch)
            .await
            .unwrap(),
        crate::application::replay_log::EpochStatus::Complete(_)
    ));
}

#[tokio::test]
async fn independent_saves_overlap_but_the_entire_batch_still_gates_output() {
    use crate::application::zone_actor::{manual_ticks, TickOutcome, ZoneActor};
    let mut h = Harness::new().await;
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Second").unwrap(),
        Race::Human,
    );
    h.repo.insert_for_test(c.clone());
    let second = EntityId::from_uuid(c.id.as_uuid());
    let record = h
        .state
        .run_tick(
            h.state
                .draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                    entity: second,
                    name: "Second".into(),
                    pos: Vec2Fixed::from_tiles(10, 10),
                    speed: Speed::DEFAULT,
                    generation: SessionGeneration(1),
                    load: Some(Box::new(PlayerLoad {
                        checkpoint_revision: Some(0),
                        ..PlayerLoad::fresh("human_fighter")
                    })),
                })]),
        )
        .unwrap();
    h.service
        .admitted(&record, &h.state.snapshot())
        .await
        .unwrap();
    h.service.players.get_mut(&second).unwrap().last_saved = Tick(0);
    while h.state.next_tick() < Tick(50) {
        h.step().await;
    }
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    h.service.repo = Arc::new(Flaky {
        inner: h.repo.clone(),
        attempts: AtomicUsize::new(1),
        after_commit: false,
        gate: Some((entered.clone(), release.clone())),
    });
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(h.state, ticks);
    zone.checkpoints.install(h.service);
    let mut outputs = zone.subscribe();
    for entity in [h.id, second] {
        zone.send(ZoneInput::system(ZoneCommand::MoveTo {
            entity,
            dest: Vec2Fixed::from_tiles(11, 10),
        }))
        .unwrap();
    }
    let step = tokio::spawn(async move { driver.step().await });
    tokio::time::timeout(Duration::from_secs(5), entered.acquire_many(2))
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(outputs.try_recv().is_err());
    release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let first = h
                .repo
                .load_for_admission(h.id.as_uuid().into())
                .await
                .unwrap()
                .unwrap();
            let other = h.repo.load_for_admission(c.id).await.unwrap().unwrap();
            if first.revision + other.revision == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(outputs.try_recv().is_err(), "one commit cannot release the tick");
    release.add_permits(1);
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(50)));
    assert_eq!(outputs.try_recv().unwrap().tick, Tick(50));
    for id in [h.id, second] {
        assert_eq!(
            h.repo
                .load_for_admission(id.as_uuid().into())
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }
}

// This adapter records immutable requests, including exact replays, without inventing DB retries.
#[derive(Default)]
struct CountCheckpoints {
    requests: parking_lot::Mutex<Vec<CharacterCheckpoint>>,
    keys: parking_lot::Mutex<BTreeMap<Uuid, (String, u64)>>,
}
#[async_trait::async_trait]
impl CharacterRepository for CountCheckpoints {
    async fn load_for_admission(&self, _: CharacterId) -> anyhow::Result<Option<ProgressionState>> {
        anyhow::bail!("analysis only")
    }
    async fn get(&self, _: CharacterId) -> anyhow::Result<Option<Character>> {
        anyhow::bail!("analysis only")
    }
    async fn list_by_account(&self, _: AccountId) -> anyhow::Result<Vec<Character>> {
        anyhow::bail!("analysis only")
    }
    async fn create_idempotent(
        &self,
        _: &IdempotencyKey,
        _: &str,
        _: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        Err(RepositoryError::Other(anyhow::anyhow!("analysis only")))
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.requests.lock().push(cp.clone());
        let mut keys = self.keys.lock();
        let fp = cp.fingerprint(events);
        if let Some((prior, rev)) = keys.get(&cp.idempotency.1.as_uuid()) {
            assert_eq!(prior, &fp);
            return Ok(CheckpointOutcome::Replayed(*rev));
        }
        let rev = cp.revision_seen + 1;
        keys.insert(cp.idempotency.1.as_uuid(), (fp, rev));
        Ok(CheckpointOutcome::Applied(rev))
    }
}

#[tokio::test]
#[ignore = "offline full checkpoint accounting; requires PERF_RECORDING"]
#[allow(clippy::print_stderr)] // Manual evidence report.
async fn recorded_checkpoint_accounting() {
    use crate::infrastructure::eventlog::Recording;
    let recording =
        Recording::read(std::path::Path::new(&std::env::var("PERF_RECORDING").unwrap())).unwrap();
    let mut state = ZoneState::from_snapshot(recording.snapshot.clone()).unwrap();
    let old_repo = Arc::new(CountCheckpoints::default());
    let new_repo = Arc::new(CountCheckpoints::default());
    let mut lanes: Vec<_> = [old_repo.clone(), new_repo.clone()]
        .into_iter()
        .map(|repo| {
            let mut lane = CheckpointService::new(
                repo,
                Arc::new(InMemorySessionAudit::default()),
                Arc::new(Metrics::detached()),
            );
            lane.restore(&recording.snapshot);
            lane.internal_cadence = true;
            lane
        })
        .collect();
    let mut counts = [BTreeMap::<&str, usize>::new(), BTreeMap::new()];
    for record in recording.records {
        let applied = state
            .run_tick(AppliedTickDraft {
                epoch: record.epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(record.zone, &applied)));
        let snapshot = state.snapshot();
        for (index, (lane, repo)) in lanes.iter_mut().zip([&old_repo, &new_repo]).enumerate() {
            let start = repo.requests.lock().len();
            for cmd in &applied.commands {
                if let ZoneCommand::Despawn { entity }
                | ZoneCommand::ReplaceSession { entity, .. } = cmd.command
                {
                    lane.flush(entity).await.unwrap();
                }
            }
            let after_flush = repo.requests.lock().len();
            *counts[index].entry("lifecycle").or_default() += after_flush - start;
            let mut reasons = BTreeMap::new();
            for entity in &snapshot.entities {
                let side = snapshot
                    .phase3
                    .as_ref()
                    .and_then(|p| p.entities.get(&entity.id));
                let Some(player) = lane.players.get_mut(&entity.id) else {
                    reasons.insert(entity.id, "admission");
                    continue;
                };
                let retained = retained_phase3_changed(player.last_phase3.as_ref(), side);
                let class = match entity.combat.as_ref().map(|c| &c.role) {
                    Some(CombatRole::Player {
                        progression: Some(p),
                        ..
                    }) => {
                        player.latest.as_ref().and_then(|c| c.class_state.as_ref())
                            != Some(&p.class_state)
                    },
                    _ => false,
                };
                let progression = applied.progression().any(|d| {
                    d.entity == entity.id
                        && (d.died.is_some() || d.level_before != d.level || d.respawned)
                });
                reasons.insert(
                    entity.id,
                    if retained {
                        "retained"
                    } else if class {
                        "class_sp_cp"
                    } else if progression {
                        "death_level_respawn"
                    } else {
                        "cadence"
                    },
                );
                // Reproduce 995195c's exact old trigger using the real lane's bookkeeping.
                if index == 0 && retained {
                    player.last_phase3 = None;
                }
            }
            lane.admitted(&applied, &snapshot).await.unwrap();
            for cp in &repo.requests.lock()[after_flush..] {
                let id = EntityId::from_uuid(cp.character_id.as_uuid());
                *counts[index].entry(reasons[&id]).or_default() += 1;
            }
        }
    }
    for (index, repo) in [old_repo, new_repo].into_iter().enumerate() {
        let requests = repo.requests.lock().len();
        let distinct = repo.keys.lock().len();
        assert_eq!(counts[index].values().sum::<usize>(), requests);
        eprintln!("ACCOUNTING historical={} requests={requests} distinct={distinct} repeats={} causes={:?}", index == 0, requests - distinct, counts[index]);
    }
}

#[tokio::test]
async fn pending_lag_never_publishes_zero_while_a_peer_remains() {
    #[derive(Default)]
    struct LagHistory(parking_lot::Mutex<Vec<Duration>>);
    impl CheckpointMetrics for LagHistory {
        fn failed(&self) {}
        fn lag(&self, age: Duration) {
            self.0.lock().push(age);
        }
    }
    let metrics = LagHistory::default();
    let young = IdempotencyKey::from_uuid(Uuid::from_u128(1));
    let old = IdempotencyKey::from_uuid(Uuid::from_u128(2));
    let now = Instant::now();
    let pending = parking_lot::Mutex::new(BTreeMap::from([
        (young, now.checked_sub(Duration::from_secs(1)).unwrap()),
        (old, now.checked_sub(Duration::from_secs(2)).unwrap()),
    ]));
    let young_save = PendingSave {
        metrics: &metrics,
        pending: &pending,
        key: young,
    };
    let old_save = PendingSave {
        metrics: &metrics,
        pending: &pending,
        key: old,
    };
    young_save.lag(Duration::ZERO);
    assert!(metrics.0.lock().last().unwrap() >= &Duration::from_secs(2));
    // Even an explicit per-request zero and the oldest request's completion cannot
    // transiently publish "drained" before the remaining peer settles.
    old_save.lag(Duration::ZERO);
    drop(old_save);
    assert_eq!(pending.lock().len(), 1);
    assert!(metrics.0.lock().iter().all(|age| !age.is_zero()));
    drop(young_save);
    assert!(pending.lock().is_empty());
    assert!(metrics.0.lock().last().unwrap().is_zero());
}
