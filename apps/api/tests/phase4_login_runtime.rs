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

/// The documented client contract: UNAVAILABLE after the bounded wait means "retry the same
/// key"; the retry returns the original frozen receipt (slow shared Postgres under load).
async fn grant(app: &common::TestApp, request: pb::GrantItemRequest) -> pb::GrantItemResponse {
    let mut attempts = 0_u8;
    loop {
        let result = app.grpc.clone().grant_item(request.clone()).await;
        match result {
            Err(status) if status.code() == tonic::Code::Unavailable && attempts < 5 => {
                attempts = attempts.saturating_add(1);
            },
            other => return other.expect("grant").into_inner(),
        }
    }
}

fn inventory_changed(m: &pb::ServerMessage) -> Option<&pb::InventoryChanged> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::InventoryChanged(changed)),
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

/// A durable V6 zone (epoch 1) with every item port and the dev grant enabled.
async fn v6_app<R>(repo: Arc<R>, log: Arc<dyn EventLog>) -> (common::TestApp, ZoneSnapshot, ZoneId)
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
    (app, baseline, zone)
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
    let account = common::account();
    let character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new("LoginGrant").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "login-grant", &character)
        .await
        .unwrap();
    let (app, baseline, zone) = v6_app(repo.clone(), log.clone()).await;
    // Offline dev grant: materializes the starter kit and 10 Adena before login.
    let granted = grant(
        &app,
        pb::GrantItemRequest {
            character_id: character.id.as_uuid().as_bytes().to_vec(),
            template_id: 57,
            count: 10,
            idempotency_key: Uuid::now_v7().to_string(),
            enchant_level: None,
        },
    )
    .await;
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

/// T11: the client's required-live sequence. `CreateCharacter`, first V6 login (the kit
/// materializes at admission), dev grants while online (potions, soulshots), then `UseItem` on
/// the potion and `Unequip`/`Equip`. The zone must
/// keep ticking (no "retained item mutation without private checkpoint" fence) and the epoch
/// replays byte-exact.
async fn client_sequence<R>(repo: Arc<R>, log: Arc<dyn EventLog>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    let account = common::account();
    let (app, baseline, zone) = v6_app(repo.clone(), log.clone()).await;
    let created = app
        .grpc
        .clone()
        .create_character(pb::CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: "ClientSequence".into(),
            race: pb::Race::Human.into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let id = api::domain::CharacterId::from_uuid(Uuid::parse_str(&created.id).unwrap());
    let player = ws::Player {
        account,
        character: id,
        name: "ClientSequence".into(),
    };
    let mut socket = ws::join(&app, &player).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|sync| sync.baseline))
        .await;
    for (template, count) in [(1060, 5), (1835, 200)] {
        grant(
            &app,
            pb::GrantItemRequest {
                character_id: id.as_uuid().as_bytes().to_vec(),
                template_id: template,
                count,
                idempotency_key: Uuid::now_v7().to_string(),
                enchant_level: None,
            },
        )
        .await;
    }
    // The grants land online (the owner is in the world): InventoryChanged per grant.
    let changes = socket
        .until(|m| {
            inventory_changed(m)
                .is_some_and(|c| c.added.iter().any(|item| item.template_id == 1835))
        })
        .await;
    let sync = inventory_sync(seen.last().unwrap()).unwrap().clone();
    let added: Vec<pb::OwnedItem> = changes
        .iter()
        .filter_map(inventory_changed)
        .flat_map(|c| c.added.iter().copied())
        .collect();
    let instance = |template| {
        sync.items
            .iter()
            .chain(added.iter())
            .find(|item| item.template_id == template)
            .unwrap()
            .instance_id
    };
    let potion = instance(1060);
    let weapon = sync
        .paperdoll
        .iter()
        .find(|entry| entry.slot == pb::EquipSlot::Rhand as i32)
        .unwrap()
        .instance_id;
    let intents = [
        pb::client_message::Intent::UseItem(pb::UseItemRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            instance_id: potion,
            target: None,
        }),
        pb::client_message::Intent::UnequipItem(pb::UnequipItemRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            slot: pb::EquipSlot::Rhand as i32,
        }),
        pb::client_message::Intent::EquipItem(pb::EquipItemRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            instance_id: weapon,
            preferred_slot: pb::EquipSlot::Rhand as i32,
        }),
    ];
    for (seq, intent) in (1_u32..).zip(intents) {
        socket
            .send(&pb::ClientMessage {
                seq,
                intent: Some(intent),
            })
            .await;
        let results = socket
            .until(|m| item_result(m).is_some() || ws::rejected(m).is_some())
            .await;
        assert!(results.iter().all(|m| ws::rejected(m).is_none()), "{results:?}");
        if seq == 1 {
            // The potion is an item skill: its effect lands after the paid use tick and
            // extends the provider install order without any item mutation (T11's trigger).
            socket
                .until(|m| {
                    matches!(
                        &m.payload,
                        Some(server_message::Payload::Event(pb::WorldEvent {
                            event: Some(world_event::Event::EffectApplied(_)),
                        }))
                    )
                })
                .await;
        }
    }
    // Let the potion's effect and reuse run for a while: the zone must keep ticking.
    let after = app.zone.stats().borrow().tick;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    app.zone.snapshot().await.expect("zone stays up");
    let later = app.zone.stats().borrow().tick;
    assert!(later > after, "the zone keeps ticking after the item intents");
    assert!(!app.zone.persistence_failed());
    socket.close().await;
    let mut replay = restore_snapshot(baseline).unwrap();
    let mut records = log.read_epoch(zone, 1).await.unwrap();
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
    }
}

#[tokio::test]
async fn memory_client_sequence_create_grant_login_use_equip() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    client_sequence(repo, Arc::new(InMemoryEventLog::default())).await;
}

#[tokio::test]
async fn postgres_jetstream_client_sequence_create_grant_login_use_equip() {
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
    client_sequence(repo, log).await;
}

/// T12: an item intent that reaches the zone before its session's player is admitted is
/// refused for that session and never fences the zone. Covers both the session guard (an
/// intent sent right after the handshake) and the actor pre-check (an item input for an entity
/// with no admitted player); afterwards the same owner logs in and equips normally.
async fn early_intent<R>(repo: Arc<R>, log: Arc<dyn EventLog>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    let account = common::account();
    let (app, baseline, zone) = v6_app(repo.clone(), log.clone()).await;
    let character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new("EarlyIntent").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "early-intent", &character)
        .await
        .unwrap();
    let unequip = |seq| pb::ClientMessage {
        seq,
        intent: Some(pb::client_message::Intent::UnequipItem(pb::UnequipItemRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            slot: pb::EquipSlot::Rhand as i32,
        })),
    };
    // Actor pre-check: an item input for an entity with no admitted player.
    let entity = api::domain::zone::EntityId::from_uuid(character.id.as_uuid());
    let command = api::interface::zone_mapping::item_command_from_pb(
        entity,
        unequip(1).intent.as_ref().unwrap(),
    )
    .unwrap()
    .unwrap();
    app.zone
        .send(ZoneInput::session(
            entity,
            SessionGeneration(1),
            1,
            ZoneCommand::Item(Box::new(command)),
        ))
        .unwrap();
    // Session guard: the intent right after the handshake, before the admission tick.
    let player = ws::Player {
        account,
        character: character.id,
        name: "EarlyIntent".into(),
    };
    let mut socket = ws::join(&app, &player).await;
    socket.send(&unequip(2)).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|sync| sync.baseline))
        .await;
    assert!(!app.zone.persistence_failed(), "no zone fence");
    for m in &seen {
        if let Some(rejected) = ws::rejected(m) {
            assert_eq!(rejected.seq, 2);
            assert_eq!(rejected.detail, "player not admitted yet");
        }
    }
    // Once admitted, item intents work on the same session.
    socket.send(&unequip(3)).await;
    let results = socket
        .until(|m| item_result(m).is_some() || ws::rejected(m).is_some_and(|r| r.seq == 3))
        .await;
    assert!(item_result(results.last().unwrap()).is_some(), "{results:?}");
    socket.close().await;
    // The zone kept running and the epoch replays byte-exact.
    let after = app.zone.stats().borrow().tick;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(app.zone.stats().borrow().tick > after);
    let mut replay = restore_snapshot(baseline).unwrap();
    let mut records = log.read_epoch(zone, 1).await.unwrap();
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
    }
}

#[tokio::test]
async fn memory_item_intent_before_admission_is_refused_without_a_fence() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    early_intent(repo, Arc::new(InMemoryEventLog::default())).await;
}

#[tokio::test]
async fn postgres_jetstream_item_intent_before_admission_is_refused_without_a_fence() {
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
    early_intent(repo, log).await;
}

/// T19: every Phase 2/3 RPC that reads the live zone (class transfer options, the session
/// generation behind `LearnSkill`, the learnable-skill catalogue) and the transfer itself work
/// for an online player of a V6 zone. They used to rebuild the zone with the V5-only
/// `ZoneState::from_snapshot` and answered UNAVAILABLE "invalid zone snapshot".
async fn phase2_phase3_rpcs<R>(repo: Arc<R>, log: Arc<dyn EventLog>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    use api::domain::subclass::LearnedSkill;
    let account = common::account();
    let mut character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new("SixTransfer").unwrap(),
        Race::Human,
    );
    let rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules;
    character.level = 40;
    character.xp = rules.xp_to_level(40).unwrap();
    // At the trainer, with SP to buy a rank and one token per tier (milestones claimed).
    character.position = api::domain::Position { x: 126.0, y: 128.0 };
    character.class_state.sp = 1_000_000;
    character.class_state.token_tier_1_count = 1;
    character.class_state.token_tier_2_count = 1;
    character.class_state.milestone_claimed_mask = 3;
    character.class_state.learned_skills = vec![LearnedSkill {
        key: "racial.adaptable".into(),
        level: 1,
    }];
    repo.create_idempotent(&IdempotencyKey::new(), "v6-transfer", &character)
        .await
        .unwrap();
    let (app, _, _) = v6_app(repo.clone(), log).await;
    let player = ws::Player {
        account,
        character: character.id,
        name: character.name.as_str().to_owned(),
    };
    let mut socket = ws::join(&app, &player).await;
    socket
        .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
        .await;
    let mut client = app.game_as(account);
    let id = character.id.to_string();
    let options = client
        .transfer_options(pb::TransferOptionsRequest {
            character_id: id.clone(),
        })
        .await
        .expect("TransferOptions on V6")
        .into_inner();
    assert_eq!(options.current_class_id, 0);
    assert_eq!(
        options
            .options
            .iter()
            .map(|o| o.class_id)
            .collect::<Vec<_>>(),
        vec![1, 4, 7]
    );
    assert!(options.options.iter().all(|o| o.eligible));
    let learnable = client
        .list_learnable_skills(pb::ListLearnableSkillsRequest {
            character_id: id.clone(),
        })
        .await
        .expect("ListLearnableSkills on V6")
        .into_inner();
    assert_eq!(learnable.current_class_id, 0);
    let rank = learnable
        .skills
        .iter()
        .filter(|s| s.eligible)
        .find_map(|s| s.skill.clone().filter(|i| i.required_items.is_empty()))
        .expect("an eligible rank at the trainer");
    let learned = client
        .learn_skill(pb::LearnSkillRequest {
            character_id: id.clone(),
            skill_id: rank.skill_id,
            skill_level: rank.skill_level,
            idempotency_key: Uuid::now_v7().to_string(),
        })
        .await
        .expect("LearnSkill on V6")
        .into_inner();
    assert_eq!((learned.skill_id, learned.skill_level), (rank.skill_id, rank.skill_level));
    let changed = client
        .change_class(pb::ChangeClassRequest {
            character_id: id.clone(),
            target_class_id: 1,
            idempotency_key: Uuid::now_v7().to_string(),
        })
        .await
        .expect("ChangeClass on V6")
        .into_inner();
    assert_eq!(changed.character.as_ref().unwrap().class_id, 1);
    let after = client
        .transfer_options(pb::TransferOptionsRequest { character_id: id })
        .await
        .expect("TransferOptions after the transfer")
        .into_inner();
    assert_eq!(after.current_class_id, 1);
    assert_eq!(after.options.iter().map(|o| o.class_id).collect::<Vec<_>>(), vec![2, 3]);
    socket.close().await;
}

#[tokio::test]
async fn memory_phase2_phase3_rpcs_work_on_a_v6_zone() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    phase2_phase3_rpcs(repo, Arc::new(InMemoryEventLog::default())).await;
}

#[tokio::test]
async fn postgres_jetstream_phase2_phase3_rpcs_work_on_a_v6_zone() {
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
    phase2_phase3_rpcs(repo, log).await;
}

/// T25/D124: a harness-seeded class-transferred owner (level-40 Treasure Hunter, class 8,
/// written straight into `characters` without its transfer receipts) is refused by the dev
/// grant with the named `FAILED_PRECONDITION` `invalid_class_transfer_receipt_history`, never
/// `INTERNAL`. With the receipt history every class change writes (Rogue 7 then Treasure Hunter
/// 8: `character_transfer_receipts` ordinals 0..1, each bound to its `change_class`
/// `idempotency_keys` row holding the frozen transfer result) the same owner is granted and
/// admitted into the V6 zone.
#[tokio::test]
async fn postgres_seeded_class_transferred_owner_needs_its_receipt_history() {
    use api::domain::character_progression::FrozenTransferResult;
    use api::domain::class::ClassId;
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let url = std::env::var("NATS_URL").expect("owned NATS_URL required");
    // The dev grant client authenticates as the shared test account.
    let account = common::account();
    common::pg::insert_account(&pool, account).await;
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let mut character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new("SeededHunter").unwrap(),
        Race::Human,
    );
    character.level = 40;
    character.xp = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules
        .xp_to_level(40)
        .unwrap();
    repo.create_idempotent(&IdempotencyKey::new(), "seeded-hunter", &character)
        .await
        .unwrap();
    let log = Arc::new(
        JetStreamEventLog::connect(async_nats::connect(&url).await.unwrap(), Metrics::detached())
            .await
            .unwrap(),
    );
    let (app, _, _) = v6_app(repo.clone(), log).await;
    let seeded = ws::Player {
        account,
        character: character.id,
        name: character.name.as_str().to_owned(),
    };
    // IssuePlayTicket refuses the corrupt history itself, so the WebSocket refusal needs a
    // ticket issued while the stored state was still valid.
    let early_ticket = ws::ticket(&app, &seeded).await;
    // The harness seed shape (seed-phase3/4-fixture.py): the class columns only.
    for statement in [
        "UPDATE characters SET current_class_id = 8 WHERE id = $1",
        "UPDATE character_class_slots SET class_id = 8 WHERE character_id = $1 AND slot = 0",
    ] {
        sqlx::query(statement)
            .bind(character.id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
    let request = || pb::GrantItemRequest {
        character_id: character.id.as_uuid().as_bytes().to_vec(),
        template_id: 57,
        count: 10,
        idempotency_key: Uuid::now_v7().to_string(),
        enchant_level: None,
    };
    let refused = app.grpc.clone().grant_item(request()).await.unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition, "{refused:?}");
    assert_eq!(refused.message(), "invalid_class_transfer_receipt_history");
    // api-review-14 P2-2: the WebSocket login of the same owner, with a ticket issued while the
    // history was still valid, is refused by name before the upgrade: the `/ws` handler's
    // character read maps the stored-state precondition to HTTP 409 (`{"error": code}`),
    // never 500. (The session-path AdmissionRefused branch behind it needs the history to break
    // between the handler's read and admission; it is defence in depth.)
    assert_eq!(ws::connect(&app, &early_ticket).await.err(), Some(409));
    let ticket = app
        .session_as(account)
        .issue_play_ticket(pb::IssuePlayTicketRequest {
            character_id: character.id.to_string(),
            idempotency_key: Uuid::now_v7().to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(ticket.code(), tonic::Code::FailedPrecondition, "{ticket:?}");
    assert_eq!(ticket.message(), "invalid_class_transfer_receipt_history");
    // The required history: one receipt per tier, parent to child, each frozen at a level the
    // target allows, with the base-class stats and the owner's identity.
    for (ordinal, target, level) in [(0_i16, 7_u32, 20_u32), (1, 8, 40)] {
        let key = Uuid::now_v7();
        let result = FrozenTransferResult {
            character_id: character.id,
            identity: character.identity(),
            name: character.name.clone(),
            current_class_id: ClassId(target),
            level,
            xp: 0,
            sp: 0,
            stats: character.stats,
            position_millitiles: [0, 0],
            hp: 1,
            mp: 1,
            cp: 0,
            max_hp: 1,
            max_mp: 1,
            max_cp: 0,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            granted_skill_keys: Vec::new(),
        };
        sqlx::query(
            "INSERT INTO idempotency_keys (key, fingerprint, account_id, operation, response) \
             VALUES ($1, $2, $3, 'change_class', $4)",
        )
        .bind(key)
        .bind(format!("v1|{}|{target}", character.id))
        .bind(account)
        .bind(serde_json::to_value(&result).unwrap())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO character_transfer_receipts \
             (character_id, ordinal, account_id, key, target_class_id) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(character.id.as_uuid())
        .bind(ordinal)
        .bind(account)
        .bind(key)
        .bind(i32::try_from(target).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    // T25 (i3c): a seed that writes the current profession into `class_profile` is refused by
    // the zone's load check, and the refusal names the field.
    sqlx::query("UPDATE characters SET class_profile = 'treasure_hunter' WHERE id = $1")
        .bind(character.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let named = app.grpc.clone().grant_item(request()).await.unwrap_err();
    assert_eq!(named.code(), tonic::Code::FailedPrecondition, "{named:?}");
    assert!(
        named
            .message()
            .starts_with("character state does not match the zone's rules: ")
            && named.message().contains("class_profile `treasure_hunter`"),
        "{named:?}"
    );
    sqlx::query("UPDATE characters SET class_profile = 'human_fighter' WHERE id = $1")
        .bind(character.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let granted = grant(&app, request()).await;
    assert!(granted.inventory.is_some());
    let player = ws::Player {
        account,
        character: character.id,
        name: character.name.as_str().to_owned(),
    };
    let mut socket = ws::join(&app, &player).await;
    socket
        .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
        .await;
    let options = app
        .game_as(account)
        .transfer_options(pb::TransferOptionsRequest {
            character_id: character.id.to_string(),
        })
        .await
        .expect("the seeded Treasure Hunter is online")
        .into_inner();
    assert_eq!(options.current_class_id, 8);
    socket.close().await;
}

/// D125: the dev `GrantItem` (admin item creation) may exceed the owner's weight ceiling, so a
/// fixture can provision overload; the owner then retains an overload rank. Slot capacity still
/// binds it (`InventoryFull`), and in-game additions keep the ceiling (domain unit test
/// `admin_grant_exceeds_weight_but_never_slot_capacity`). Idempotent retry is unchanged.
async fn overweight_grant<R>(repo: Arc<R>, log: Arc<dyn EventLog>)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    let character = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("HeavyGrant").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "heavy-grant", &character)
        .await
        .unwrap();
    let (app, _, _) = v6_app(repo.clone(), log).await;
    let id = character.id.as_uuid().as_bytes().to_vec();
    let request = |template_id, count| pb::GrantItemRequest {
        character_id: id.clone(),
        template_id,
        count,
        idempotency_key: Uuid::now_v7().to_string(),
        enchant_level: None,
    };
    // The kit first, so the retained ceiling is known.
    grant(&app, request(57, 1)).await;
    let before = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert!(before.weight <= before.max_load && before.overload_level == 0);
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let parts = bundle.items.parts();
    let (template, weight) = parts
        .items
        .iter()
        .filter(|(t, d)| {
            d.stackable
                && **t != 57
                && d.weight > 0
                && parts.capabilities.get(t).is_some_and(|c| c.storable)
        })
        .map(|(t, d)| (*t, u64::from(d.weight)))
        .max_by_key(|(t, w)| (*w, *t))
        .unwrap();
    let count = u64::try_from(before.max_load)
        .unwrap()
        .checked_mul(2)
        .and_then(|load| load.checked_div(weight))
        .and_then(|units| units.checked_add(1))
        .unwrap();
    let heavy = request(template, count);
    let first = grant(&app, heavy.clone()).await;
    assert_eq!(grant(&app, heavy).await, first, "idempotent retry unchanged");
    let after = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert!(after.weight > after.max_load, "{} <= {}", after.weight, after.max_load);
    assert!(after.overloaded && after.overload_level >= 1, "{after:?}");
    // Slot capacity still binds the admin grant.
    let filler = parts
        .items
        .iter()
        .filter(|(t, d)| {
            !d.stackable
                && d.flags.get("is_questitem") == Some(&false)
                && parts.capabilities.get(t).is_some_and(|c| c.storable)
        })
        .map(|(t, _)| *t)
        .next()
        .unwrap();
    let refused = app
        .grpc
        .clone()
        .grant_item(request(filler, u64::from(after.ordinary_capacity).checked_add(1).unwrap()))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), tonic::Code::FailedPrecondition, "{refused:?}");
    assert_eq!(refused.message(), "InventoryFull");
    // api-review-14 P2-3: the overloaded owner logs into V6 and is admitted (4-overload).
    let player = ws::Player {
        account: common::account(),
        character: character.id,
        name: character.name.as_str().to_owned(),
    };
    let mut socket = ws::join(&app, &player).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
        .await;
    let baseline = inventory_sync(seen.last().unwrap()).unwrap();
    assert_eq!(baseline.inventory_revision, after.inventory_revision);
    socket.close().await;
}

#[tokio::test]
async fn memory_dev_grant_may_overload_but_not_overfill() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    overweight_grant(repo, Arc::new(InMemoryEventLog::default())).await;
}

#[tokio::test]
async fn postgres_jetstream_dev_grant_may_overload_but_not_overfill() {
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
    overweight_grant(repo, log).await;
}

/// T36/D142: owners with item activity (an online grant, then an automatic soulshot that
/// charges the equipped weapon) disconnect together while another owner stays online, then
/// log in again. The new incarnation drops the transient charges and toggles (D71); that reset
/// must settle through the item lane, so the zone never fences and every re-login gets its
/// inventory baseline.
async fn relogin_after_item_activity<R>(
    repo: Arc<R>,
    log: Arc<dyn EventLog>,
    pool: Option<&sqlx::PgPool>,
) where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + ItemReceiptRepository
        + LearningCheckpointRepository
        + 'static,
{
    const LEAVING: usize = 8;
    let mut players = Vec::new();
    for index in 0..=LEAVING {
        let account = Uuid::now_v7();
        if let Some(pool) = pool {
            common::pg::insert_account(pool, account).await;
        }
        let name = format!(
            "Relogin{}",
            char::from(b'a'.checked_add(u8::try_from(index).unwrap()).unwrap())
        );
        let character = Character::create(
            AccountId::from_uuid(account),
            CharacterName::new(name.clone()).unwrap(),
            Race::Human,
        );
        repo.create_idempotent(&IdempotencyKey::new(), &name, &character)
            .await
            .unwrap();
        players.push(ws::Player {
            account,
            character: character.id,
            name,
        });
    }
    let (app, _, _) = v6_app(repo.clone(), log).await;
    let mut sockets = Vec::new();
    for player in &players {
        let mut socket = ws::join(&app, player).await;
        socket
            .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
            .await;
        let granted = app
            .game_as(player.account)
            .grant_item(pb::GrantItemRequest {
                character_id: player.character.as_uuid().as_bytes().to_vec(),
                template_id: 1835,
                count: 200,
                idempotency_key: Uuid::now_v7().to_string(),
                enchant_level: None,
            })
            .await
            .expect("online grant");
        assert!(granted.into_inner().inventory.is_some());
        socket
            .send(&pb::ClientMessage {
                seq: 1,
                intent: Some(pb::client_message::Intent::ToggleAutoShot(
                    pb::ToggleAutoShotRequest {
                        idempotency_key: Uuid::now_v7().to_string(),
                        template_id: 1835,
                        enabled: true,
                    },
                )),
            })
            .await;
        let results = socket
            .until(|m| item_result(m).is_some() || ws::rejected(m).is_some())
            .await;
        assert!(results.iter().all(|m| ws::rejected(m).is_none()), "{results:?}");
        sockets.push(socket);
    }
    let stays = sockets.pop().unwrap();
    // Eight owners leave together.
    for socket in sockets {
        socket.close().await;
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert!(!app.zone.persistence_failed(), "the departures must not fence the zone");
    for player in players.iter().take(LEAVING) {
        let mut socket = ws::join(&app, player).await;
        let seen = socket
            .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
            .await;
        let baseline = inventory_sync(seen.last().unwrap()).unwrap();
        assert!(baseline.items.iter().any(|item| item.template_id == 1835));
        socket.close().await;
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert!(!app.zone.persistence_failed(), "the re-logins must not fence the zone");
    app.zone.snapshot().await.expect("zone stays up");
    stays.close().await;
}

#[tokio::test]
#[ignore = "D142 zone half pending"]
async fn memory_relogin_after_item_activity_keeps_the_zone_up() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    relogin_after_item_activity(repo, Arc::new(InMemoryEventLog::default()), None).await;
}

#[tokio::test]
#[ignore = "D142 zone half pending"]
async fn postgres_jetstream_relogin_after_item_activity_keeps_the_zone_up() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
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
    relogin_after_item_activity(repo, log, Some(&pool)).await;
}
