//! Replay/upgrade boundaries on real Postgres and default-payload `JetStream`.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;
use common::token::{balances, character, definition, grants};
use nightfall_api::application::checkpoint::CheckpointService;
use nightfall_api::application::replay_log::{
    encode_snapshot, AppliedTickRecord, EventLog, NoReplayMetrics, WatermarkReason,
    ZoneSnapshotRow, ZoneSnapshotStore,
};
use nightfall_api::application::zone_actor::{manual_ticks, TickOutcome};
use nightfall_api::application::zone_bootstrap::ZoneBootstrap;
use nightfall_api::application::{CharacterRepository, IdempotencyKey, ProgressionState};
use nightfall_api::domain::zone::{
    EntityId, PlayerLoad, PlayerProgression, SessionGeneration, Speed, StateDigestVersion, Tick,
    Vec2Fixed, ZoneCommand, ZoneEvent, ZoneInput, ZoneSeed, ZoneState,
};
use nightfall_api::infrastructure::class_data::{load_classes, ClassSource};
use nightfall_api::infrastructure::eventlog::{JetStreamEventLog, ZONES_STREAM};
use nightfall_api::infrastructure::memory::InMemorySessionAudit;
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgZoneSnapshotStore};
use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource};
use nightfall_api::infrastructure::telemetry::Metrics;
use sqlx::{Executor, PgPool};
use std::sync::Arc;

async fn services() -> (
    PgPool,
    Arc<PgCharacterRepository>,
    Arc<JetStreamEventLog>,
    Arc<PgZoneSnapshotStore>,
) {
    let pool = common::pg::migrated_pool()
        .await
        .expect("real token recovery requires DATABASE_URL; no skipped pass");
    let client = async_nats::connect(
        std::env::var("NATS_URL").expect("real token recovery requires NATS_URL; no skipped pass"),
    )
    .await
    .unwrap();
    assert_eq!(client.server_info().max_payload, 1_048_576);
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
    let log = Arc::new(
        JetStreamEventLog::connect(client, Metrics::detached())
            .await
            .unwrap(),
    );
    let store = Arc::new(PgZoneSnapshotStore::new(pool.clone()));
    (pool, repo, log, store)
}
fn lane(repo: Arc<PgCharacterRepository>) -> CheckpointService {
    CheckpointService::new(
        repo,
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
}
fn spawn(id: nightfall_api::domain::CharacterId, p: ProgressionState) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: EntityId::from_uuid(id.as_uuid()),
        name: p.name.as_str().into(),
        pos: Vec2Fixed::from_tiles(126, 126),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(PlayerLoad {
            progression: Some(Box::new(PlayerProgression {
                identity: p.identity,
                class_state: p.class_state,
                max_cp: 0,
            })),
            checkpoint_revision: Some(p.revision),
            class: p.class_profile,
            level: p.level,
            xp: p.xp,
            hp: p.hp,
            mp: p.mp,
            alive: p.alive,
        })),
    })
}
async fn baseline(log: &dyn EventLog, store: &dyn ZoneSnapshotStore, state: &ZoneState) {
    let snapshot = state.snapshot();
    let seq = log.write_snapshot(&snapshot).await.unwrap();
    store
        .insert(&ZoneSnapshotRow {
            zone: snapshot.seed.zone,
            epoch: snapshot.seed.epoch,
            snapshot: encode_snapshot(&snapshot).unwrap(),
            snapshot_seq: seq,
            first_seq: None,
            time_origin_ms: snapshot.time_origin_ms,
            build_id: snapshot.meta.build_id.clone(),
            config_hash: snapshot.meta.config_hash.clone(),
            schema_version: snapshot.meta.schema_version,
        })
        .await
        .unwrap();
}
fn state(def: &nightfall_api::application::zone_bootstrap::ZoneDefinition) -> ZoneState {
    ZoneState::new(
        ZoneSeed {
            zone: def.zone,
            epoch: 1,
        },
        def.bounds,
        0,
    )
    .with_rules(common::rules())
    .with_classes(load_classes(&ClassSource::embedded()).unwrap().registry)
    .unwrap()
}

#[tokio::test]
async fn crash_after_record_ack_before_or_after_real_commit_recovers_identical_once_only_supply() {
    for committed in [false, true] {
        let (pool, repo, log, store) = services().await;
        let c = character("Crashrecover", 40, [0, 0], 0);
        repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
            .await
            .unwrap();
        let def = definition();
        let mut state = state(&def);
        baseline(log.as_ref(), store.as_ref(), &state).await;
        let applied = state
            .run_tick(state.draft(vec![spawn(
                c.id,
                repo.load_for_admission(c.id).await.unwrap().unwrap(),
            )]))
            .unwrap();
        assert!(applied
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
        store.recording(def.zone, 1, applied.tick).await.unwrap();
        log.append_applied(&AppliedTickRecord::from_applied(def.zone, &applied))
            .await
            .unwrap();
        if committed {
            lane(repo.clone())
                .admitted(&applied, &state.snapshot())
                .await
                .unwrap();
        } else {
            assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
        }
        // The lost process released no outputs. Recovery consumes exactly the durable
        // prefix before any fresh-epoch admission, replaying the same transaction key.
        let mut recovered = lane(repo.clone());
        recovered
            .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
            .await
            .unwrap();
        let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([1, 1], 3));
        let facts = grants(&pool, &c).await;
        assert_eq!(facts.len(), 2);
        assert!(store.unresolved(def.zone).await.unwrap().is_empty());
        recovered
            .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
            .await
            .unwrap();
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), saved);
        assert_eq!(grants(&pool, &c).await, facts);
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // The actual recovery -> failed baseline -> new epoch -> admission boundary.
async fn old_policy_recovers_without_supply_then_new_policy_baseline_must_commit_before_admission()
{
    let (pool, repo, log, store) = services().await;
    let c = character("Upgrade", 40, [0, 0], 0);
    repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
        .await
        .unwrap();
    let mut def = definition();
    def.npcs.clear();
    def.spawn_slots.clear();
    let mut old = state(&def).snapshot();
    old.meta.schema_version = 7;
    old.meta.digest_version = StateDigestVersion::BinaryV3;
    let mut old_state = ZoneState::from_snapshot(old).unwrap();
    baseline(log.as_ref(), store.as_ref(), &old_state).await;
    let old_join = old_state
        .run_tick(old_state.draft(vec![spawn(
            c.id,
            repo.load_for_admission(c.id).await.unwrap().unwrap(),
        )]))
        .unwrap();
    assert!(!old_join
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::TokensReconciled { .. })));
    store.recording(def.zone, 1, old_join.tick).await.unwrap();
    log.append_applied(&AppliedTickRecord::from_applied(def.zone, &old_join))
        .await
        .unwrap();
    let mut recovery = lane(repo.clone());
    recovery
        .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
        .await
        .unwrap();
    assert!(store.unresolved(def.zone).await.unwrap().is_empty());
    assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
    assert!(grants(&pool, &c).await.is_empty());
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    let classes = load_classes(&ClassSource::embedded()).unwrap();
    let bootstrap = ZoneBootstrap::new(
        log.clone(),
        Some(store.clone()),
        Arc::new(nightfall_api::infrastructure::SystemClock),
        Arc::new(NoReplayMetrics),
    )
    .with_rules(load_rules(&RulesSource::embedded()).unwrap())
    .with_classes(classes.registry, classes.config_hash);
    pool.execute(
        "ALTER TABLE zone_snapshots ADD CONSTRAINT reject_new_epoch CHECK (epoch=1) NOT VALID",
    )
    .await
    .unwrap();
    // Real baseline transaction fails after the new snapshot was acknowledged by NATS.
    assert!(bootstrap.start(&def, manual_ticks().0).await.is_err());
    assert_eq!(log.latest_epoch(def.zone).await.unwrap(), Some(2));
    assert!(store.get(def.zone, 2).await.unwrap().is_none());
    assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
    assert!(grants(&pool, &c).await.is_empty());
    pool.execute("ALTER TABLE zone_snapshots DROP CONSTRAINT reject_new_epoch")
        .await
        .unwrap();
    recovery
        .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
        .await
        .unwrap();
    let (ticks, driver) = manual_ticks();
    let running = bootstrap.start(&def, ticks).await.unwrap();
    assert_eq!(running.epoch(), 3);
    let snapshot = log
        .read_snapshot(def.zone, 3)
        .await
        .unwrap()
        .unwrap()
        .snapshot;
    assert_eq!(
        (snapshot.meta.schema_version, snapshot.meta.digest_version),
        (8, StateDigestVersion::BinaryV4)
    );
    assert_eq!(
        store.get(def.zone, 3).await.unwrap().unwrap().snapshot,
        encode_snapshot(&snapshot).unwrap()
    );
    running
        .handle()
        .checkpoints
        .install(recovery.with_durability(log.clone(), store.clone()));
    let mut visible = running.handle().subscribe();
    running
        .handle()
        .send(spawn(c.id, repo.load_for_admission(c.id).await.unwrap().unwrap()))
        .unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    visible.recv().await.unwrap();
    assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([1, 1], 3));
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(grants(&pool, &c).await.len(), 2);
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

#[tokio::test]
async fn missing_real_jetstream_snapshot_or_record_keeps_recovery_unresolved_and_database_unmodified(
) {
    for remove_snapshot in [false, true] {
        let (pool, repo, log, store) = services().await;
        let c = character("Missinghistory", 40, [0, 0], 0);
        repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
            .await
            .unwrap();
        let def = definition();
        let mut state = state(&def);
        baseline(log.as_ref(), store.as_ref(), &state).await;
        let batch = state
            .run_tick(state.draft(vec![spawn(
                c.id,
                repo.load_for_admission(c.id).await.unwrap().unwrap(),
            )]))
            .unwrap();
        store.recording(def.zone, 1, batch.tick).await.unwrap();
        if remove_snapshot {
            log.append_applied(&AppliedTickRecord::from_applied(def.zone, &batch))
                .await
                .unwrap();
            let seq = log.read_snapshot(def.zone, 1).await.unwrap().unwrap().seq;
            let client = async_nats::connect(std::env::var("NATS_URL").unwrap())
                .await
                .unwrap();
            let stream = async_nats::jetstream::new(client)
                .get_stream(ZONES_STREAM)
                .await
                .unwrap();
            assert!(stream.delete_message(seq.0).await.unwrap());
        }
        let error = lane(repo.clone())
            .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains(if remove_snapshot {
                "snapshot missing"
            } else {
                "history ends before indexed tick"
            }),
            "{error}"
        );
        assert_eq!(store.unresolved(def.zone).await.unwrap().len(), 1);
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
        assert!(grants(&pool, &c).await.is_empty());
    }
}

#[tokio::test]
async fn missing_checkpoint_lane_or_durable_gate_stops_real_database_admission_before_any_output() {
    use nightfall_api::application::replay_log::{start_epoch, DurableTickGate, GateConfig};
    use nightfall_api::application::zone_actor::ZoneActor;
    use std::time::Duration;
    for missing_log in [false, true] {
        let (pool, repo, log, store) = services().await;
        let c = character("Missinglane", 40, [0, 0], 0);
        repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
            .await
            .unwrap();
        let def = definition();
        let state = state(&def);
        baseline(log.as_ref(), store.as_ref(), &state).await;
        let (ticks, driver) = manual_ticks();
        let actor = if missing_log {
            let actor = ZoneActor::spawn(state, ticks);
            actor.checkpoints.install(lane(repo.clone()));
            actor
        } else {
            let start = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
            ZoneActor::spawn_gated(
                state,
                ticks,
                DurableTickGate::new(
                    &start,
                    log.clone(),
                    Arc::new(NoReplayMetrics),
                    GateConfig::default(),
                    tokio_util::sync::CancellationToken::new(),
                ),
            )
        };
        let mut output = actor.subscribe();
        actor
            .send(spawn(c.id, repo.load_for_admission(c.id).await.unwrap().unwrap()))
            .unwrap();
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(Tick(0)));
        tokio::time::timeout(Duration::from_secs(5), actor.stopped())
            .await
            .unwrap();
        assert!(actor.persistence_failed());
        assert!(output.try_recv().is_err());
        assert_eq!(balances(&repo.get(c.id).await.unwrap().unwrap()), ([0, 0], 0));
        assert_eq!(
            repo.load_for_admission(c.id)
                .await
                .unwrap()
                .unwrap()
                .revision,
            0
        );
        assert!(grants(&pool, &c).await.is_empty());
    }
}
