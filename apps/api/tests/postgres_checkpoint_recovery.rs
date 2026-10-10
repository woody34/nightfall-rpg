//! E4.2/E4.3 crash recovery of the admitted durable prefix, with real checkpoint transactions.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;

use nightfall_api::application::checkpoint::CheckpointService;
use nightfall_api::application::replay_log::{
    encode_snapshot, AppliedTickRecord, EpochStatus, EventLog, ZoneSnapshotRow, ZoneSnapshotStore,
};
use nightfall_api::application::{CharacterRepository, IdempotencyKey};
use nightfall_api::domain::zone::{
    EntityId, EntityKind, PlayerLoad, SessionGeneration, Speed, Vec2Fixed, ZoneCommand, ZoneInput,
    ZoneState,
};
use nightfall_api::domain::{AccountId, Character, CharacterName, DomainEvent, Race};
use nightfall_api::infrastructure::eventlog::InMemoryEventLog;
use nightfall_api::infrastructure::memory::InMemorySessionAudit;
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgZoneSnapshotStore};
use nightfall_api::infrastructure::telemetry::Metrics;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[allow(clippy::too_many_lines)] // One end-to-end crash matrix shares the same combat setup.
async fn crash_before_or_after_transaction_and_missing_audit_ack_recovers_identical_facts() {
    for (commit_before_crash, expire_snapshot) in [(false, false), (true, false), (false, true)] {
        let Some(pool) = common::pg::migrated_pool().await else {
            return;
        };
        let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
        let c = Character::create(
            AccountId::from_uuid(Uuid::now_v7()),
            CharacterName::new("Recoverer").unwrap(),
            Race::Human,
        );
        repo.create_idempotent(&IdempotencyKey::from_uuid(Uuid::now_v7()), "create", &c)
            .await
            .unwrap();
        let id = EntityId::from_uuid(c.id.as_uuid());
        let log = InMemoryEventLog::default();
        let mut state = common::fixture_zone();
        let mut npc = common::keltir();
        npc.xp_reward = 400;
        npc.stats.max_hp = 1;
        state
            .run_tick(state.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "Target".into(),
                pos: Vec2Fixed::from_tiles(11, 10),
                speed: Speed::DEFAULT,
                combat: Some(Box::new(npc)),
            })]))
            .unwrap();
        // An epoch-start snapshot may already contain its fixture population.
        let mut snap = state.snapshot();
        snap.tick = nightfall_api::domain::zone::Tick(0);
        state = ZoneState::from_snapshot(snap.clone()).unwrap();
        let target = snap
            .entities
            .iter()
            .find(|e| e.kind == EntityKind::Npc)
            .unwrap()
            .id;
        let snapshot_seq = log.write_snapshot(&snap).await.unwrap();
        let store = PgZoneSnapshotStore::new(pool.clone());
        store
            .insert(&ZoneSnapshotRow {
                zone: snap.seed.zone,
                epoch: snap.seed.epoch,
                snapshot: encode_snapshot(&snap).unwrap(),
                snapshot_seq,
                first_seq: None,
                time_origin_ms: snap.time_origin_ms,
                build_id: snap.meta.build_id.clone(),
                config_hash: snap.meta.config_hash.clone(),
                schema_version: snap.meta.schema_version,
            })
            .await
            .unwrap();
        let audit = Arc::new(InMemorySessionAudit::default());
        let metrics = Arc::new(Metrics::detached());
        let mut live = CheckpointService::new(repo.clone(), audit.clone(), metrics.clone());
        let mut inputs = vec![
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: id,
                name: "Recoverer".into(),
                pos: Vec2Fixed::from_tiles(10, 10),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(PlayerLoad {
                    checkpoint_revision: Some(0),
                    ..PlayerLoad::fresh("human_fighter")
                })),
            }),
            ZoneInput::system(ZoneCommand::SetTarget {
                entity: id,
                target: Some(target),
            }),
            ZoneInput::system(ZoneCommand::Attack { entity: id }),
        ];
        let mut leveled = false;
        for _ in 0..100 {
            let applied = state
                .run_tick(state.draft(std::mem::take(&mut inputs)))
                .unwrap();
            store
                .recording(snap.seed.zone, snap.seed.epoch, applied.tick)
                .await
                .unwrap();
            log.append_applied(&AppliedTickRecord::from_applied(snap.seed.zone, &applied))
                .await
                .unwrap();
            if commit_before_crash {
                live.admitted(&applied, &state.snapshot()).await.unwrap();
            }
            if applied.progression().any(|p| !p.levels_gained.is_empty()) {
                leveled = true;
                break;
            }
        }
        assert!(leveled);
        assert!(matches!(
            log.epoch_status(snap.seed.zone, snap.seed.epoch)
                .await
                .unwrap(),
            EpochStatus::Incomplete { .. }
        ));
        // No completion watermark or save audit ack: only the admitted prefix survives.
        drop(live);
        if expire_snapshot {
            log.delete_message(snapshot_seq);
            assert_eq!(log.latest_epoch(snap.seed.zone).await.unwrap(), None);
            let mut restarted =
                CheckpointService::new(repo.clone(), audit.clone(), metrics.clone());
            let error = restarted
                .recover_indexed(&log, &store, snap.seed.zone)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("snapshot missing"));
            assert_eq!(store.unresolved(snap.seed.zone).await.unwrap().len(), 1);
            assert_eq!(
                repo.load_for_admission(c.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .revision,
                0
            );
            continue;
        }
        for _ in 0..2 {
            let mut restarted =
                CheckpointService::new(repo.clone(), audit.clone(), metrics.clone());
            restarted
                .recover_indexed(&log, &store, snap.seed.zone)
                .await
                .unwrap();
        }
        let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((loaded.level, loaded.xp, loaded.revision), (3, 400, 1));
        let payloads: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM outbox WHERE subject = 'nightfall.character.leveled' ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let events: Vec<DomainEvent> = payloads
            .into_iter()
            .map(|p| serde_json::from_value(p).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0], DomainEvent::CharacterLeveled { level:2, metadata, .. } if metadata.sequence == (1,0))
        );
        assert!(
            matches!(&events[1], DomainEvent::CharacterLeveled { level:3, metadata, .. } if metadata.sequence == (1,1))
        );
    }
}
