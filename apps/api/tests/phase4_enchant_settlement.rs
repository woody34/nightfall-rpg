//! Actual recorded enchanting settles its captured audit before later same-tick destruction.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::application::replay_log::{restore_snapshot, start_epoch, AppliedTickRecord, EventLog};
use api::domain::zone::item_grant::grant_fingerprint;
use api::domain::zone::phase4_projection::{ItemEnchantOutcome, ItemResultDetail};
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterId, CharacterName, DomainEvent, Race};
use api::infrastructure::eventlog::{InMemoryEventLog, JetStreamEventLog};
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::postgres::PgCharacterRepository;
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::{class_data, item_data, rules_data};
use nightfall_api as api;
use std::sync::Arc;
use uuid::Uuid;

fn player_load(loaded: api::application::ProgressionState) -> PlayerLoad {
    PlayerLoad {
        phase3: loaded.phase3.or_else(|| Some(Phase3Checkpoint::new())),
        phase4: loaded.phase4,
        progression: Some(Box::new(PlayerProgression {
            identity: loaded.identity,
            class_state: loaded.class_state,
            max_cp: 0,
        })),
        checkpoint_revision: Some(loaded.revision),
        class: loaded.class_profile,
        level: loaded.level,
        xp: loaded.xp,
        hp: loaded.hp,
        mp: loaded.mp,
        alive: loaded.alive,
    }
}
async fn contract<R>(
    repo: Arc<R>,
    bundle: item_data::Phase4RuleBundle,
    log: Arc<dyn EventLog>,
) -> CharacterId
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + 'static,
{
    let c = Character::create(
        AccountId::from_uuid(Uuid::from_u128(1)),
        CharacterName::new("EnchantAudit").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "enchant-owner", &c)
        .await
        .unwrap();
    let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let range = repo.reserve_item_range(zone, 1).await.unwrap();
    let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
    let mut state = ZoneState::new_phase4(
        ZoneSeed { zone, epoch: 1 },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        classes.registry,
        (*bundle.skills).clone(),
        bundle.items.clone(),
        ItemInstanceRange::new(range.range_start, range.next_reserved, range.range_end_exclusive)
            .unwrap(),
    )
    .unwrap();
    start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let mut service = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_items(repo.clone());
    let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let core = ItemPausedCore {
        name: c.name.as_str().to_owned(),
        position: Vec2Fixed::from_tiles(0, 0),
        load: player_load(loaded),
    };
    let entity = EntityId::from_uuid(c.id.as_uuid());
    let grants = [(71, 1), (22227, 2)]
        .into_iter()
        .map(|(template, count)| {
            ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(ItemGrantCommand {
                entity,
                account: c.account_id,
                request_id: Uuid::now_v7(),
                template_id: template,
                count,
                fingerprint: grant_fingerprint(c.id, template, count),
                rules_hash: bundle.items.hash(),
                paused: Some(Box::new(core.clone())),
                known: None,
                conflict: None,
            })))
        })
        .collect();
    let grant = state.run_tick(state.draft(grants)).unwrap();
    assert!(grant.dispositions.is_empty());
    log.append_applied(&AppliedTickRecord::from_applied(zone, &grant))
        .await
        .unwrap();
    service.admitted(&grant, &state.snapshot()).await.unwrap();
    let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let admitted = state
        .run_tick(state.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity,
            name: c.name.as_str().to_owned(),
            pos: core.position,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(player_load(loaded))),
        })]))
        .unwrap();
    assert!(admitted.dispositions.is_empty());
    log.append_applied(&AppliedTickRecord::from_applied(zone, &admitted))
        .await
        .unwrap();
    service
        .admitted(&admitted, &state.snapshot())
        .await
        .unwrap();
    let before = state.snapshot();
    let cp = before
        .phase4
        .as_ref()
        .unwrap()
        .checkpoint(entity, admitted.tick, before.phase3.as_ref().unwrap())
        .unwrap();
    let target = cp
        .instances
        .iter()
        .find(|item| item.template_id == 71)
        .unwrap()
        .id;
    let scroll = cp
        .instances
        .iter()
        .find(|item| item.template_id == 22227)
        .unwrap()
        .id;
    let key = Uuid::now_v7();
    let enchant = ItemCommand::new(
        entity,
        key,
        ItemCommandKind::Enchant {
            target,
            scroll,
            support: None,
        },
    )
    .unwrap()
    .with_account(c.account_id)
    .unwrap();
    let intent = |seq, command: ItemCommand| {
        ZoneInput::session(entity, SessionGeneration(1), seq, ZoneCommand::Item(Box::new(command)))
    };
    let draft = state.draft(vec![
        intent(
            1,
            ItemCommand::new(
                entity,
                Uuid::now_v7(),
                ItemCommandKind::Equip {
                    instance: target,
                    preferred_slot: Some(EquipSlot::Rhand),
                },
            )
            .unwrap()
            .with_account(c.account_id)
            .unwrap(),
        ),
        intent(2, enchant.clone()),
        intent(3, enchant.clone()),
        intent(
            4,
            ItemCommand::new(
                entity,
                Uuid::now_v7(),
                ItemCommandKind::Destroy {
                    instance: target,
                    count: 1,
                },
            )
            .unwrap()
            .with_account(c.account_id)
            .unwrap(),
        ),
    ]);
    let word_before = before.rng.word_pos;
    let mut cold = restore_snapshot(before).unwrap();
    let tick = state.run_tick(draft.clone()).unwrap();
    assert_eq!(tick, cold.run_tick(draft).unwrap());
    assert!(tick.dispositions.is_empty());
    assert_eq!(state.snapshot().rng.word_pos, word_before.checked_add(1).unwrap());
    let record = AppliedTickRecord::from_applied(zone, &tick);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    log.append_applied(&record).await.unwrap();
    service.admitted(&tick, &state.snapshot()).await.unwrap();
    let projection = tick
        .events
        .iter()
        .find_map(|event| {
            if let ZoneEvent::ItemCheckpoint(p) = event {
                Some(p.as_ref())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(projection.enchant_attempts.len(), 1);
    let attempt = &projection.enchant_attempts[0];
    assert_eq!(attempt.outcome, ItemEnchantOutcome::Success);
    assert_eq!((attempt.old_enchant, attempt.new_enchant), (0, 1));
    assert_eq!(attempt.before.slot, Some(EquipSlot::Rhand));
    assert_eq!(attempt.after.as_ref().unwrap().enchant, 1);
    assert!(!projection
        .checkpoint
        .instances
        .iter()
        .any(|item| item.id == target));
    assert_eq!(
        projection
            .checkpoint
            .instances
            .iter()
            .find(|item| item.id == scroll)
            .unwrap()
            .count,
        1
    );
    let ItemReceiptLookup::Known(receipt) = repo
        .item_receipt_lookup(
            c.account_id,
            ItemMutationOperation::EnchantItem,
            &IdempotencyKey::from_uuid(key),
            &enchant.fingerprint_hex().unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("missing original enchant receipt");
    };
    assert!(matches!(
        receipt.result.detail,
        ItemResultDetail::Enchant {
            old_enchant: 0,
            new_enchant: 1,
            outcome: ItemEnchantOutcome::Success,
            ..
        }
    ));
    let saved = repo
        .retained_inventory(c.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert_eq!(saved, projection.checkpoint);
    c.id
}

#[tokio::test]
async fn memory_actual_enchant_audit_survives_later_target_destruction_and_duplicate() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    contract(repo.clone(), bundle, Arc::new(InMemoryEventLog::default())).await;
    let attempts: Vec<_> = repo
        .staged_events()
        .into_iter()
        .filter_map(|event| {
            if let DomainEvent::ItemEnchanted { fact } = event {
                Some(fact)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].after.as_ref().unwrap().enchant, 1);
    assert!(attempts[0].causes[0].get("scroll").is_some());
}
#[tokio::test]
async fn owned_postgres_jetstream_actual_enchant_audit_survives_target_destruction() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
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
    let facts: Vec<serde_json::Value> = sqlx::query_scalar("SELECT payload->'fact' FROM outbox WHERE subject='nightfall.item.enchanted' AND payload->'fact'->>'character_id'=$1").bind(id.to_string()).fetch_all(&pool).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["after"]["enchant"], 1);
    assert_eq!(facts[0]["causes"][0]["old_enchant"], 0);
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM item_mutation_receipts WHERE character_id=$1 AND operation='enchant_item'").bind(id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(receipts, 1);
}
