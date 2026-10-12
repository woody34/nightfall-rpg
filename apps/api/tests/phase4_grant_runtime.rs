//! Actual socket -> lifecycle -> V6 recorded grant -> atomic checkpoint -> frozen reply.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::application::replay_log::{
    restore_snapshot, start_epoch, AppliedTickRecord, DurableTickGate, EventLog, GateConfig,
    NoReplayMetrics,
};
use api::application::use_cases::RegistryItemCatalogue;
use api::application::zone_actor::{IntervalTicks, ZoneActor};
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterName, Race};
use api::infrastructure::eventlog::{InMemoryEventLog, JetStreamEventLog};
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::postgres::PgCharacterRepository;
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::{class_data, item_data, rules_data};
use api::interface::grpc::pb;
use nightfall_api as api;
use std::fmt::Write as _;
use std::sync::Arc;
use tokio_stream::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tonic::Code;
use uuid::Uuid;

async fn contract<R>(
    repo: Arc<R>,
    bundle: item_data::Phase4RuleBundle,
    log: Arc<dyn EventLog>,
) -> api::domain::CharacterId
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + LearningCheckpointRepository
        + 'static,
{
    let character = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("OfflineGrant").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "offline-grant", &character)
        .await
        .unwrap();
    let mut expected_skill_hash = String::new();
    for byte in bundle.skills.skills().hash() {
        write!(&mut expected_skill_hash, "{byte:02x}").unwrap();
    }
    let pending = repo.pending_item_requests(character.id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert!(matches!(pending[0].status, PendingItemStatus::Pending));
    let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let reservation = repo.reserve_item_range(zone, 1).await.unwrap();
    let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
    let state = ZoneState::new_phase4(
        ZoneSeed { zone, epoch: 1 },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        classes.registry.clone(),
        (*bundle.skills).clone(),
        bundle.items.clone(),
        ItemInstanceRange::new(
            reservation.range_start,
            reservation.next_reserved,
            reservation.range_end_exclusive,
        )
        .unwrap(),
    )
    .unwrap();
    let baseline = state.snapshot();
    let started = start_epoch(log.as_ref(), &baseline).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let handle = ZoneActor::spawn_gated(state, IntervalTicks::new(), gate);
    handle.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_items(repo.clone())
        .with_learning(repo.clone()),
    );
    let catalogue = Arc::new(RegistryItemCatalogue::new(bundle.items, &bundle.skills).unwrap());
    let ports = repo.clone();
    let app = common::TestApp::spawn_with_handle(
        move |deps| {
            deps.dev_item_grants = true;
            deps.characters = ports.clone();
            deps.item_receipts = Some(ports.clone());
            deps.item_inventory = Some(ports.clone());
            deps.item_checkpoints = Some(ports);
            deps.item_catalogue = Some(catalogue);
            deps.classes = Some(classes);
        },
        handle,
    )
    .await;
    let mut client = app.grpc.clone();
    let skill_catalogue = client
        .get_skill_catalog(pb::GetSkillCatalogRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(skill_catalogue.data_version, expected_skill_hash);
    assert!(!skill_catalogue.skills.is_empty());

    let request = pb::GrantItemRequest {
        character_id: character.id.as_uuid().as_bytes().to_vec(),
        template_id: 57,
        count: 10,
        idempotency_key: Uuid::now_v7().to_string(),
    };
    assert_eq!(
        app.anon_grpc()
            .grant_item(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_as(Uuid::from_u128(77))
            .grant_item(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        client
            .get_inventory(pb::GetInventoryRequest {
                character_id: request.character_id.clone()
            })
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert!(repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .is_none());
    let first = client
        .grant_item(request.clone())
        .await
        .unwrap()
        .into_inner();
    let result = first.result.as_ref().unwrap();
    assert_eq!(result.entity_id, character.id.as_uuid().as_bytes());
    assert_eq!(result.receipt_id, request.idempotency_key);
    assert_eq!(result.incarnation, 0);
    assert_eq!(result.operation, pb::ItemOperation::Grant as i32);
    let inventory = first.inventory.as_ref().unwrap();
    assert!(inventory.baseline);
    assert_eq!((inventory.tick, inventory.baseline_tick, inventory.incarnation), (0, 0, 0));
    assert_eq!(inventory.inventory_revision, result.inventory_revision);
    let saved = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.character_revision, 1);
    assert_eq!(
        saved.checkpoint.as_ref().unwrap().inventory_revision,
        inventory.inventory_revision
    );
    assert_eq!(
        saved
            .checkpoint
            .as_ref()
            .unwrap()
            .instances
            .iter()
            .find(|item| item.template_id == 57)
            .unwrap()
            .count,
        10
    );
    assert!(matches!(
        repo.pending_item_requests(character.id).await.unwrap()[0].status,
        PendingItemStatus::Materialized { .. }
    ));
    let detached = client
        .get_inventory(pb::GetInventoryRequest {
            character_id: request.character_id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .inventory
        .unwrap();
    assert_eq!(detached, *inventory);
    let mut next = request.clone();
    next.count = 5;
    next.idempotency_key = Uuid::now_v7().to_string();
    let mut peer = client.clone();
    let (left, right) =
        tokio::join!(client.grant_item(next.clone()), peer.grant_item(next.clone()));
    assert_eq!(left.unwrap().into_inner(), right.unwrap().into_inner());
    let after = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.character_revision, 2);
    assert_eq!(
        after
            .checkpoint
            .unwrap()
            .instances
            .iter()
            .find(|item| item.template_id == 57)
            .unwrap()
            .count,
        15
    );
    assert_eq!(
        client
            .grant_item(request.clone())
            .await
            .unwrap()
            .into_inner(),
        first
    );
    let mut conflicting = request.clone();
    conflicting.count = 11;
    assert_eq!(
        client.grant_item(conflicting).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        repo.retained_inventory(character.id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        2
    );
    assert!(!app
        .zone
        .snapshot()
        .await
        .unwrap()
        .entities
        .iter()
        .any(|e| e.id.as_uuid() == character.id.as_uuid()));
    let mut replay = restore_snapshot(baseline).unwrap();
    let mut records = log.read_epoch(zone, 1).await.unwrap();
    let mut grants = 0_u32;
    let mut committed_grants = 0_usize;
    while let Some(record) = records.next().await {
        let record = record.unwrap();
        let tick = replay
            .run_tick(AppliedTickDraft {
                epoch: 1,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(zone, &tick)));
        assert!(tick.outputs.is_empty());
        committed_grants = committed_grants.checked_add(tick.events.iter().filter(|event| {
            matches!(event, ZoneEvent::ItemCheckpoint(projection) if projection.receipts.iter().any(|receipt| receipt.result.operation() == api::domain::zone::phase4_projection::ItemOperation::Grant))
        }).count()).unwrap();
        for command in &tick.commands {
            assert!(!matches!(command.command, ZoneCommand::SpawnPlayer { .. }));
            if matches!(command.command, ZoneCommand::MaterializePendingItems(_)) {
                grants = grants.checked_add(1_u32).unwrap();
            }
        }
    }
    assert!(grants >= 2);
    assert_eq!(committed_grants, 2);
    character.id
}

#[tokio::test]
async fn memory_actual_offline_grant_socket_retries_and_cold_replay() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    contract(repo.clone(), bundle, Arc::new(InMemoryEventLog::default())).await;
    assert_eq!(
        repo.staged_events()
            .iter()
            .filter(|e| matches!(e, api::domain::DomainEvent::ItemGranted { .. }))
            .count(),
        repo.item_ledger_for_test().len()
    );
}

#[tokio::test]
async fn owned_postgres_jetstream_actual_offline_grant_socket_is_atomic() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    common::pg::insert_account(&pool, common::account()).await;
    let url = std::env::var("NATS_URL").expect("owned NATS_URL required");
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let log = Arc::new(
        JetStreamEventLog::connect(async_nats::connect(&url).await.unwrap(), Metrics::detached())
            .await
            .unwrap(),
    );
    let id = contract(repo, bundle, log).await;
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM item_mutation_receipts WHERE character_id=$1 AND operation='grant_item'").bind(id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(receipts, 2);
    let grants: i64 =
        sqlx::query_scalar("SELECT count(*) FROM item_ledger WHERE character_id=$1 AND op='grant'")
            .bind(id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(grants, 2);
    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->'fact'->>'character_id'=$1 AND subject='nightfall.item.granted'").bind(id.to_string()).fetch_one(&pool).await.unwrap();
    let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM item_ledger WHERE character_id=$1")
        .bind(id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox, ledger);
}
