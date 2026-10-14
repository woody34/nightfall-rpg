//! T9: a character provisioned by the dev `GrantItem` logs into a real V6 session. The canonical
//! V6 load (retained checkpoint) rides `SpawnPlayer` into the zone, the owner observes its own
//! spawn and its `InventorySync` baseline with the granted item, a WS item intent executes, and
//! the recorded V6 epoch replays byte-exact. A fresh, never-granted character (pending starter kit,
//! no retained checkpoint) logs in too, and an Equip intent publishes `EquipmentChanged`.
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
use api::interface::grpc::pb::{self, server_message, world_event};
use common::ws;
use futures_util::StreamExt as _;
use nightfall_api as api;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn inventory_sync(m: &pb::ServerMessage) -> Option<&pb::InventorySync> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::InventorySync(sync)),
        })) => Some(sync),
        _ => None,
    }
}

fn equipment_changed(m: &pb::ServerMessage) -> Option<&pb::EquipmentChanged> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::EquipmentChanged(changed)),
        })) => Some(changed),
        _ => None,
    }
}

fn item_result(m: &pb::ServerMessage) -> Option<&pb::ItemOperationResult> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::ItemOperationResult(result)),
        })) => Some(result),
        _ => None,
    }
}

async fn contract<R>(repo: Arc<R>, log: Arc<dyn EventLog>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let account = common::account();
    let character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new("LoginGrant").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "login-grant", &character)
        .await
        .unwrap();
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
    assert!(handle.phase4_enabled());
    handle.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(Metrics::detached()),
        )
        .with_items(repo.clone())
        .with_item_admission(repo.clone())
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
            deps.item_checkpoints = Some(ports.clone());
            deps.item_admission = Some(ports.clone());
            deps.learning = Some(ports);
            deps.item_catalogue = Some(catalogue);
            deps.classes = Some(classes);
        },
        handle,
    )
    .await;
    // Offline dev grant: materializes the starter kit and 10 Adena before login.
    let granted = app
        .grpc
        .clone()
        .grant_item(pb::GrantItemRequest {
            character_id: character.id.as_uuid().as_bytes().to_vec(),
            template_id: 57,
            count: 10,
            idempotency_key: Uuid::now_v7().to_string(),
            enchant_level: None,
        })
        .await
        .unwrap()
        .into_inner();
    let revision = granted.inventory.unwrap().inventory_revision;
    // Real session: ticket, WS, admission with the canonical V6 load.
    let player = ws::Player {
        account,
        character: character.id,
        name: character.name.as_str().to_owned(),
    };
    let mut socket = ws::join(&app, &player).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|sync| sync.baseline))
        .await;
    assert!(
        seen.iter()
            .filter_map(ws::spawn_of)
            .any(|spawn| spawn.entity_id == player.entity_id()),
        "in_world: own spawn precedes the inventory baseline"
    );
    let sync = inventory_sync(seen.last().unwrap()).unwrap();
    assert_eq!(sync.entity_id, character.id.as_uuid().as_bytes());
    assert_eq!(sync.inventory_revision, revision);
    assert!(sync
        .items
        .iter()
        .any(|item| item.template_id == 57 && item.count == 10));
    assert!(!sync.paperdoll.is_empty(), "the kit's equipped items are worn");
    // A WS item intent executes on V6 (no rollout refusal): unequip the kit weapon.
    let weapon = sync
        .paperdoll
        .iter()
        .find(|entry| entry.slot == pb::EquipSlot::Rhand as i32)
        .expect("kit weapon")
        .instance_id;
    let key = Uuid::now_v7();
    socket
        .send(&pb::ClientMessage {
            seq: 1,
            intent: Some(pb::client_message::Intent::UnequipItem(pb::UnequipItemRequest {
                idempotency_key: key.to_string(),
                slot: pb::EquipSlot::Rhand as i32,
            })),
        })
        .await;
    let results = socket.until(|m| item_result(m).is_some()).await;
    assert!(results.iter().all(|m| ws::rejected(m).is_none()), "{results:?}");
    let result = item_result(results.last().unwrap()).unwrap();
    assert_eq!(result.receipt_id, key.to_string());
    assert_eq!(result.operation, pb::ItemOperation::Unequip as i32);
    assert_eq!(result.affected_instance_ids, vec![weapon]);
    // Equip it again: the owner gets the receipt and the public slot projection.
    let key = Uuid::now_v7();
    socket
        .send(&pb::ClientMessage {
            seq: 2,
            intent: Some(pb::client_message::Intent::EquipItem(pb::EquipItemRequest {
                idempotency_key: key.to_string(),
                instance_id: weapon,
                preferred_slot: pb::EquipSlot::Rhand as i32,
            })),
        })
        .await;
    let results = socket.until(|m| item_result(m).is_some()).await;
    assert!(results.iter().all(|m| ws::rejected(m).is_none()), "{results:?}");
    let result = item_result(results.last().unwrap()).unwrap();
    assert_eq!(result.receipt_id, key.to_string());
    assert_eq!(result.operation, pb::ItemOperation::Equip as i32);
    assert_eq!(result.affected_instance_ids, vec![weapon]);
    let mut seen = results;
    if !seen.iter().any(|m| equipment_changed(m).is_some()) {
        seen.extend(socket.until(|m| equipment_changed(m).is_some()).await);
    }
    let weapon_template = sync
        .items
        .iter()
        .find(|item| item.instance_id == weapon)
        .unwrap()
        .template_id;
    assert!(
        seen.iter().filter_map(equipment_changed).any(|changed| {
            changed.entity_id == character.id.as_uuid().as_bytes()
                && changed.visible.iter().any(|v| {
                    v.slot == pb::EquipSlot::Rhand as i32 && v.template_id == weapon_template
                })
        }),
        "ItemEquipped: the RHAND slot shows the weapon again"
    );
    socket.close().await;
    // A fresh, never-granted character: its pending starter kit rides the V6 load with no
    // retained checkpoint and materializes at admission, before the inventory baseline.
    // Created through the production CreateCharacter RPC: a Human Mystic whose level-1
    // auto-get and racial skills are learned metadata, with no retained skill checkpoint.
    let created = app
        .grpc
        .clone()
        .create_character(pb::CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: "LoginFresh".into(),
            race: pb::Race::Human.into(),
            base_class_id: Some(10),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let fresh = repo
        .get(api::domain::CharacterId::from_uuid(Uuid::parse_str(&created.id).unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert!(!fresh.class_state.learned_skills.is_empty());
    let admission = repo.load_for_admission(fresh.id).await.unwrap().unwrap();
    assert!(admission.phase3.is_none());
    assert!(admission.phase4.as_ref().unwrap().checkpoint.is_none());
    let newcomer = ws::Player {
        account,
        character: fresh.id,
        name: fresh.name.as_str().to_owned(),
    };
    let mut socket = ws::join(&app, &newcomer).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|sync| sync.baseline))
        .await;
    assert!(seen
        .iter()
        .filter_map(ws::spawn_of)
        .any(|spawn| spawn.entity_id == newcomer.entity_id()));
    let kit = inventory_sync(seen.last().unwrap()).unwrap();
    assert_eq!(kit.entity_id, fresh.id.as_uuid().as_bytes());
    assert!(!kit.items.is_empty() && !kit.paperdoll.is_empty());
    assert!(!kit.items.iter().any(|item| item.template_id == 57));
    socket.close().await;
    let retained = repo.retained_inventory(fresh.id).await.unwrap().unwrap();
    assert_eq!(retained.checkpoint.unwrap().inventory_revision, kit.inventory_revision);
    assert!(repo
        .pending_item_requests(fresh.id)
        .await
        .unwrap()
        .iter()
        .all(|request| matches!(request.status, PendingItemStatus::Materialized { .. })));
    // The recorded V6 epoch, including SpawnPlayer with its V6 load, replays byte-exact.
    let mut replay = restore_snapshot(baseline).unwrap();
    let mut records = log.read_epoch(zone, 1).await.unwrap();
    let (mut spawned, mut pending_spawned) = (false, false);
    while let Some(record) = records.next().await {
        let record = record.unwrap();
        let encoded = record.encode();
        assert_eq!(AppliedTickRecord::decode(&encoded).unwrap(), record);
        let tick = replay
            .run_tick(AppliedTickDraft {
                epoch: 1,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(zone, &tick)));
        for command in &record.commands {
            if let ZoneCommand::SpawnPlayer {
                load: Some(load), ..
            } = &command.command
            {
                let items = load
                    .phase4
                    .as_ref()
                    .expect("every V6 spawn carries its item load");
                spawned |= items.checkpoint.is_some();
                pending_spawned |= items.checkpoint.is_none();
            }
        }
    }
    assert!(spawned, "the recorded SpawnPlayer carries the retained V6 checkpoint");
    assert!(pending_spawned, "the fresh SpawnPlayer carries its pending kit");
}

#[tokio::test]
async fn memory_dev_granted_character_logs_into_v6_and_replays() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    contract(repo, Arc::new(InMemoryEventLog::default())).await;
}

#[tokio::test]
async fn postgres_jetstream_dev_granted_character_logs_into_v6_and_replays() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let url = std::env::var("NATS_URL").expect("owned NATS_URL required");
    common::pg::insert_account(&pool, common::account()).await;
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool)
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let log = Arc::new(
        JetStreamEventLog::connect(async_nats::connect(&url).await.unwrap(), Metrics::detached())
            .await
            .unwrap(),
    );
    contract(repo, log).await;
}
