//! Token admission uses the same durable record and revision fence as live transfers.
use super::*;
use crate::application::checkpoint::CheckpointMetrics;
use crate::application::replay_log::{start_epoch, DurableTickGate, GateConfig, NoReplayMetrics};
use crate::domain::character_progression::TokenSource;
use crate::domain::zone::{StateDigestVersion, ZoneEvent};
use std::sync::atomic::AtomicUsize;

#[derive(Default)]
struct Grants(AtomicUsize);
impl CheckpointMetrics for Grants {
    fn failed(&self) {}
    fn lag(&self, _: Duration) {}
    fn token_granted(&self, _: u8, _: TokenSource) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

fn token_state(epoch: u64) -> ZoneState {
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(77),
            epoch,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
    )
    .with_rules(load_rules(&RulesSource::embedded()).unwrap().rules)
    .with_classes(load_classes(&ClassSource::embedded()).unwrap().registry)
    .unwrap()
}
fn character(level: u32, balance: u32, mask: u8) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::from_u128(7)),
        CharacterName::new("Tokenhero").unwrap(),
        Race::Human,
    );
    c.level = level;
    c.xp = load_rules(&RulesSource::embedded())
        .unwrap()
        .rules
        .xp_to_level(level)
        .unwrap();
    c.class_state.token_tier_1_count = balance;
    c.class_state.token_tier_2_count = balance;
    c.class_state.milestone_claimed_mask = mask;
    c
}
fn spawn(c: &Character, revision: Option<u64>) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: EntityId::from_uuid(c.id.as_uuid()),
        name: c.name.as_str().into(),
        pos: Vec2Fixed::from_tiles(10, 10),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(PlayerLoad {
            progression: Some(Box::new(PlayerProgression {
                identity: c.identity(),
                class_state: c.class_state.clone(),
                max_cp: 0,
            })),
            checkpoint_revision: revision,
            level: c.level,
            xp: c.xp,
            hp: Some(17),
            mp: Some(9),
            ..PlayerLoad::fresh("human_fighter")
        })),
    })
}
fn service(repo: Arc<Repo>, grants: Arc<Grants>) -> CheckpointService {
    CheckpointService::new(repo, Arc::new(InMemorySessionAudit::default()), grants)
}
async fn actor(
    state: ZoneState,
    repo: Arc<Repo>,
    grants: Arc<Grants>,
) -> (ZoneHandle, ManualTickDriver, Arc<InMemoryEventLog>) {
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated(state, ticks, gate);
    handle.checkpoints.install(service(repo, grants));
    (handle, driver, log)
}

#[tokio::test]
async fn admission_grant_and_mark_only_wait_for_commit_and_reconnect_is_idempotent() {
    for balance in [0, 7] {
        let c = character(40, balance, 0);
        let repo = Arc::new(Repo::new());
        repo.inner.insert_for_test(c.clone());
        repo.hold.store(true, Ordering::Release);
        let metrics = Arc::new(Grants::default());
        let (handle, driver, log) = actor(token_state(1), repo.clone(), metrics.clone()).await;
        let mut outputs = handle.subscribe();
        handle.send(spawn(&c, Some(0))).unwrap();
        let step = tokio::spawn(async move { driver.step().await });
        tokio::time::timeout(Duration::from_secs(5), repo.entered.notified())
            .await
            .unwrap();
        assert!(outputs.try_recv().is_err());
        assert_eq!(metrics.0.load(Ordering::Acquire), 0);
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        let snapshot_wait = tokio::spawn({
            let handle = handle.clone();
            async move { handle.snapshot().await }
        });
        tokio::task::yield_now().await;
        assert!(!snapshot_wait.is_finished());
        repo.hold.store(false, Ordering::Release);
        repo.release.notify_one();
        assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(0)));
        let batch = outputs.recv().await.unwrap();
        assert!(batch
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
        let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.class_state.milestone_claimed_mask, 3);
        assert_eq!((saved.hp, saved.mp), (Some(17), Some(9)));
        assert_eq!(saved.class_state.token_tier_1_count, balance.max(1));
        assert_eq!(metrics.0.load(Ordering::Acquire), if balance == 0 { 2 } else { 0 });
        let snapshot = snapshot_wait.await.unwrap().unwrap();
        assert_eq!(snapshot.checkpoints[0].revision, 1);
        let mut replay = service(repo.clone(), metrics.clone());
        replay.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        assert_eq!(metrics.0.load(Ordering::Acquire), if balance == 0 { 2 } else { 0 });
        // A fresh epoch reloads the persisted ledger; no second save/grant on admission.
        let mut next = c.clone();
        next.class_state = saved.class_state;
        let (next_handle, next_driver, _) =
            actor(token_state(2), repo.clone(), metrics.clone()).await;
        next_handle.send(spawn(&next, Some(1))).unwrap();
        assert_eq!(next_driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }
}

#[tokio::test]
async fn admission_transient_and_lost_ack_retry_exact_requests_without_duplicate_grants() {
    for lost in [false, true] {
        let c = character(40, 0, 0);
        let repo = Arc::new(Repo::new());
        repo.inner.insert_for_test(c.clone());
        repo.lost_reply.store(lost, Ordering::Release);
        repo.transient_once.store(!lost, Ordering::Release);
        let metrics = Arc::new(Grants::default());
        let (handle, driver, log) = actor(token_state(1), repo.clone(), metrics.clone()).await;
        handle.send(spawn(&c, Some(0))).unwrap();
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
        {
            let requests = repo.requests.lock();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0], requests[1]);
        }
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        assert_eq!(
            repo.inner
                .staged_events()
                .iter()
                .filter(|e| matches!(e, DomainEvent::CharacterTokenGranted { .. }))
                .count(),
            2
        );
        assert_eq!(metrics.0.load(Ordering::Acquire), if lost { 0 } else { 2 }); // applied-only
        service(repo.clone(), metrics.clone())
            .recover(log.as_ref(), ZoneId(77), 1)
            .await
            .unwrap();
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            1
        );
    }
}

#[tokio::test]
async fn admission_permanent_failure_or_missing_revision_never_releases_and_recovers_once() {
    for missing in [false, true] {
        let c = character(40, 0, 0);
        let repo = Arc::new(Repo::new());
        repo.inner.insert_for_test(c.clone());
        repo.reject_constraint.store(!missing, Ordering::Release);
        let metrics = Arc::new(Grants::default());
        let (handle, driver, log) = actor(token_state(1), repo.clone(), metrics.clone()).await;
        let mut outputs = handle.subscribe();
        handle
            .send(spawn(&c, if missing { None } else { Some(0) }))
            .unwrap();
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(0)));
        handle.stopped().await;
        assert!(handle.persistence_failed());
        assert!(outputs.try_recv().is_err());
        assert_eq!(metrics.0.load(Ordering::Acquire), 0);
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        if !missing {
            repo.reject_constraint.store(false, Ordering::Release);
            let mut recovery = service(repo.clone(), metrics.clone());
            recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
            recovery.recover(log.as_ref(), ZoneId(77), 1).await.unwrap();
            assert_eq!(
                repo.load_for_admission(c.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .revision,
                1
            );
            assert_eq!(metrics.0.load(Ordering::Acquire), 2);
        }
    }
}

#[tokio::test]
async fn admission_without_log_or_db_lane_fails_closed() {
    for missing_log in [false, true] {
        let c = character(40, 0, 0);
        let repo = Arc::new(Repo::new());
        repo.inner.insert_for_test(c.clone());
        let metrics = Arc::new(Grants::default());
        let (handle, driver) = if missing_log {
            let (ticks, driver) = manual_ticks();
            let handle = ZoneActor::spawn(token_state(1), ticks);
            handle
                .checkpoints
                .install(service(repo.clone(), metrics.clone()));
            (handle, driver)
        } else {
            let log = Arc::new(InMemoryEventLog::default());
            let state = token_state(1);
            let start = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
            let (ticks, driver) = manual_ticks();
            (
                ZoneActor::spawn_gated(
                    state,
                    ticks,
                    DurableTickGate::new(
                        &start,
                        log,
                        Arc::new(NoReplayMetrics),
                        GateConfig::default(),
                        tokio_util::sync::CancellationToken::new(),
                    ),
                ),
                driver,
            )
        };
        let mut outputs = handle.subscribe();
        handle.send(spawn(&c, Some(0))).unwrap();
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(0)));
        handle.stopped().await;
        assert!(handle.persistence_failed());
        assert!(outputs.try_recv().is_err());
        assert!(repo.requests.lock().is_empty());
        assert_eq!(metrics.0.load(Ordering::Acquire), 0);
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One ordered crash/retry sequence documents the admission boundary.
async fn old_recovery_then_failed_upgrade_retries_new_durable_policy_before_join() {
    use crate::application::replay_log::{encode_snapshot, WatermarkReason};
    use crate::application::zone_bootstrap::ZoneBootstrap;
    let c = character(40, 0, 0);
    let repo = Arc::new(Repo::new());
    repo.inner.insert_for_test(c.clone());
    let metrics = Arc::new(Grants::default());
    let log = Arc::new(InMemoryEventLog::default());
    let index = Arc::new(Index::default());
    let mut old = token_state(1).snapshot();
    old.meta.schema_version = 7;
    old.meta.digest_version = StateDigestVersion::BinaryV3;
    let seq = log.write_snapshot(&old).await.unwrap();
    index
        .insert(&ZoneSnapshotRow {
            zone: ZoneId(77),
            epoch: 1,
            snapshot: encode_snapshot(&old).unwrap(),
            snapshot_seq: seq,
            first_seq: None,
            time_origin_ms: 0,
            build_id: old.meta.build_id.clone(),
            config_hash: String::new(),
            schema_version: 7,
        })
        .await
        .unwrap();
    let mut state = ZoneState::from_snapshot(old).unwrap();
    let batch = state
        .run_tick(state.draft(vec![spawn(&c, Some(0))]))
        .unwrap();
    assert!(!batch
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
    index.recording(ZoneId(77), 1, batch.tick).await.unwrap();
    log.append_applied(&AppliedTickRecord::from_applied(ZoneId(77), &batch))
        .await
        .unwrap();
    let mut recovery = service(repo.clone(), metrics.clone());
    recovery
        .recover_indexed(log.as_ref(), index.as_ref(), ZoneId(77))
        .await
        .unwrap();
    assert!(index.unresolved(ZoneId(77)).await.unwrap().is_empty());
    assert!(repo.requests.lock().is_empty());
    assert_eq!(metrics.0.load(Ordering::Acquire), 0);
    let rules = load_rules(&RulesSource::embedded()).unwrap();
    let classes = load_classes(&ClassSource::embedded()).unwrap();
    let bootstrap = Arc::new(
        ZoneBootstrap::new(
            log.clone(),
            Some(index.clone()),
            Arc::new(crate::infrastructure::SystemClock),
            Arc::new(NoReplayMetrics),
        )
        .with_rules(rules)
        .with_classes(classes.registry, classes.config_hash),
    );
    let mut def = crate::infrastructure::zone_data::parse_zone(
        crate::infrastructure::zone_data::TEST_ZONE_TOML,
    )
    .unwrap();
    def.zone = ZoneId(77);
    index.fail_insert.store(true, Ordering::Release);
    assert!(bootstrap.start(&def, manual_ticks().0).await.is_err());
    assert_eq!(log.latest_epoch(ZoneId(77)).await.unwrap(), Some(2));
    assert!(index.get(ZoneId(77), 2).await.unwrap().is_none());
    assert!(repo.requests.lock().is_empty());
    // Process retry: prior recovery is already closed; no hidden old-policy live epoch.
    recovery
        .recover_indexed(log.as_ref(), index.as_ref(), ZoneId(77))
        .await
        .unwrap();
    index.fail_insert.store(false, Ordering::Release);
    index.hold_insert.store(true, Ordering::Release);
    // Consume any notification from the failed attempt before awaiting the new one.
    index.insert_entered.notified().await;
    let (ticks, driver) = manual_ticks();
    let start = tokio::spawn({
        let bootstrap = bootstrap.clone();
        async move { bootstrap.start(&def, ticks).await }
    });
    tokio::time::timeout(Duration::from_secs(5), index.insert_entered.notified())
        .await
        .unwrap();
    assert!(!start.is_finished()); // no handle, no admission success before baseline DB ACK
    assert!(repo.requests.lock().is_empty());
    index.hold_insert.store(false, Ordering::Release);
    index.insert_release.notify_one();
    let running = start.await.unwrap().unwrap();
    assert_eq!(running.epoch(), 3);
    let snapshot = log
        .read_snapshot(ZoneId(77), 3)
        .await
        .unwrap()
        .unwrap()
        .snapshot;
    assert_eq!(
        (snapshot.meta.schema_version, snapshot.meta.digest_version),
        (8, StateDigestVersion::BinaryV4)
    );
    assert_eq!(
        index.get(ZoneId(77), 3).await.unwrap().unwrap().snapshot,
        encode_snapshot(&snapshot).unwrap()
    );
    running
        .handle()
        .checkpoints
        .install(recovery.with_durability(log.clone(), index.clone()));
    let mut outputs = running.handle().subscribe();
    running.handle().send(spawn(&c, Some(0))).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    outputs.recv().await.unwrap();
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(
        (
            saved.class_state.milestone_claimed_mask,
            saved.class_state.token_tier_1_count,
            saved.class_state.token_tier_2_count
        ),
        (3, 1, 1)
    );
    assert_eq!(saved.revision, 1);
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

struct TokenLogGate {
    allow: Arc<AtomicBool>,
    log: Arc<InMemoryEventLog>,
}
impl TickGate for TokenLogGate {
    fn durable(&self) -> bool {
        true
    }
    async fn admit(&self, tick: &AppliedTick) -> Result<(), GateError> {
        if !self.allow.load(Ordering::Acquire) {
            return Err(GateError("log ACK withheld".into()));
        }
        self.log
            .append_applied(&AppliedTickRecord::from_applied(ZoneId(77), tick))
            .await
            .map_err(|e| GateError(e.to_string()))?;
        Ok(())
    }
}

#[tokio::test]
async fn held_log_never_reaches_db_or_outputs_and_retries_one_immutable_admission() {
    use tokio_stream::StreamExt as _;
    let state = token_state(1);
    let c = character(40, 0, 0);
    let log = Arc::new(InMemoryEventLog::default());
    log.write_snapshot(&state.snapshot()).await.unwrap();
    let repo = Arc::new(Repo::new());
    repo.inner.insert_for_test(c.clone());
    let metrics = Arc::new(Grants::default());
    let allow = Arc::new(AtomicBool::new(false));
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated(
        state,
        ticks,
        TokenLogGate {
            allow: allow.clone(),
            log: log.clone(),
        },
    );
    handle
        .checkpoints
        .install(service(repo.clone(), metrics.clone()));
    let mut outputs = handle.subscribe();
    handle.send(spawn(&c, Some(0))).unwrap();
    for _ in 0..2 {
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(0)));
        assert!(repo.requests.lock().is_empty());
        assert!(outputs.try_recv().is_err());
        assert_eq!(metrics.0.load(Ordering::Acquire), 0);
    }
    assert!(log
        .read_epoch(ZoneId(77), 1)
        .await
        .unwrap()
        .next()
        .await
        .is_none());
    allow.store(true, Ordering::Release);
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    assert_eq!(outputs.recv().await.unwrap().tick, Tick(0));
    assert_eq!(repo.requests.lock().len(), 1);
    assert_eq!(metrics.0.load(Ordering::Acquire), 2);
    let mut records = log.read_epoch(ZoneId(77), 1).await.unwrap();
    assert!(records.next().await.unwrap().is_ok());
    assert!(records.next().await.is_none());
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Real command/impact/held-commit sequence is deliberately contiguous.
async fn actual_combat_level_grant_is_in_critical_checkpoint_before_owner_output() {
    let mut c = character(19, 0, 0);
    c.xp = 835_861;
    let repo = Arc::new(Repo::new());
    repo.inner.insert_for_test(c.clone());
    let metrics = Arc::new(Grants::default());
    let state = token_state(1);
    let def = crate::infrastructure::zone_data::parse_zone(
        crate::infrastructure::zone_data::TEST_ZONE_TOML,
    )
    .unwrap();
    let mut npc = crate::domain::zone::NpcCombat::from_template(
        state.rules().unwrap(),
        &def.npc_templates[0],
    )
    .unwrap();
    npc.stats.max_hp = 1;
    npc.xp_reward = 10_235;
    let (handle, driver, _) = actor(state, repo.clone(), metrics.clone()).await;
    let mut outputs = handle.subscribe();
    handle.send(spawn(&c, Some(0))).unwrap();
    handle
        .send(ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Milestonevictim".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            speed: Speed::DEFAULT,
            combat: Some(Box::new(npc)),
        }))
        .unwrap();
    driver.step().await.unwrap();
    let batch = outputs.recv().await.unwrap();
    let npc = batch
        .events
        .iter()
        .find_map(|event| {
            if let ZoneEvent::EntitySpawn {
                entity,
                kind: crate::domain::zone::EntityKind::Npc,
                ..
            } = event
            {
                Some(*entity)
            } else {
                None
            }
        })
        .unwrap();
    let player = EntityId::from_uuid(c.id.as_uuid());
    handle
        .send(ZoneInput::session(
            player,
            SessionGeneration(1),
            1,
            ZoneCommand::SetTarget {
                entity: player,
                target: Some(npc),
            },
        ))
        .unwrap();
    handle
        .send(ZoneInput::session(
            player,
            SessionGeneration(1),
            2,
            ZoneCommand::Attack { entity: player },
        ))
        .unwrap();
    repo.hold.store(true, Ordering::Release);
    let driving = tokio::spawn(async move {
        for _ in 0..100 {
            driver.step().await.unwrap();
        }
    });
    tokio::time::timeout(Duration::from_secs(5), repo.entered.notified())
        .await
        .unwrap();
    while let Ok(batch) = outputs.try_recv() {
        assert!(!batch.events.iter().any(|event| matches!(
            event,
            ZoneEvent::TokensReconciled { .. } | ZoneEvent::LevelUp { .. }
        )));
    }
    assert_eq!(metrics.0.load(Ordering::Acquire), 0);
    {
        let requests = repo.requests.lock();
        let cp = &requests[0].0;
        assert_eq!((cp.level, cp.xp), (20, 846_607));
        assert_eq!(cp.class_state.as_ref().unwrap().token_tier_1_count, 1);
        assert_eq!(cp.class_state.as_ref().unwrap().milestone_claimed_mask, 1);
        assert!(requests[0].1.iter().any(|e| matches!(
            e,
            DomainEvent::CharacterTokenGranted {
                tier: 1,
                source: TokenSource::LevelUp,
                ..
            }
        )));
    }
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    repo.hold.store(false, Ordering::Release);
    repo.release.notify_one();
    let committed = outputs.recv().await.unwrap();
    assert!(committed.events.iter().any(|e| matches!(
        e,
        ZoneEvent::TokensReconciled {
            source: TokenSource::LevelUp,
            ..
        }
    )));
    assert_eq!(metrics.0.load(Ordering::Acquire), 1);
    driving.abort();
    let _ = driving.await;
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .class_state
            .token_tier_1_count,
        1
    );
}
