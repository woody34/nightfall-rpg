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
        phase5: None,
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
        position: Vec2Fixed::from_tiles(
            bundle.items.parts().capacities.blacksmith.x,
            bundle.items.parts().capacities.blacksmith.y,
        ),
        load: player_load(loaded),
    };
    let entity = EntityId::from_uuid(c.id.as_uuid());
    let recipe = bundle
        .items
        .parts()
        .sa_recipes
        .values()
        .find(|recipe| {
            recipe.base_template == 71
                && recipe
                    .products
                    .iter()
                    .any(|product| product.template_id == 4711)
        })
        .unwrap()
        .clone();
    let mut grant_items = vec![(71, 3), (22227, 2)];
    grant_items.extend(
        recipe
            .ingredients
            .iter()
            .filter(|row| row.template_id != recipe.base_template)
            .map(|row| (row.template_id, row.count.checked_mul(2).unwrap())),
    );
    let grants = grant_items
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
                enchant: 0,
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
        .run_tick(state.draft(vec![
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity,
                name: c.name.as_str().to_owned(),
                pos: core.position,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(player_load(loaded))),
            }),
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: bundle.items.parts().capacities.blacksmith.template.clone(),
                pos: core.position,
                speed: Speed::from_milli_tiles_per_tick(0),
                combat: None,
            }),
        ]))
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
    let snapshot = state.snapshot();
    let npc = effects::TargetBinding {
        entity: snapshot
            .entities
            .iter()
            .find(|npc| npc.name == bundle.items.parts().capacities.blacksmith.template)
            .unwrap()
            .id,
        incarnation: 0,
    };
    let bases: Vec<_> = saved
        .instances
        .iter()
        .filter(|item| item.template_id == recipe.base_template)
        .map(|item| item.id)
        .collect();
    assert_eq!(bases.len(), 2);
    let commands: Vec<_> = bases
        .iter()
        .map(|base| {
            ItemCommand::new(
                entity,
                Uuid::now_v7(),
                ItemCommandKind::ExchangeSpecialAbility {
                    npc,
                    target: *base,
                    recipe: recipe.recipe_id,
                },
            )
            .unwrap()
            .with_account(c.account_id)
            .unwrap()
        })
        .collect();
    let draft = state.draft(vec![
        intent(5, commands[0].clone()),
        intent(6, commands[0].clone()),
        intent(7, commands[1].clone()),
    ]);
    let before_exchange = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let mut cold = restore_snapshot(snapshot.clone()).unwrap();
    let exchanged = state.run_tick(draft.clone()).unwrap();
    assert_eq!(exchanged, cold.run_tick(draft).unwrap());
    assert!(exchanged.dispositions.is_empty(), "{:?}", exchanged.dispositions);
    assert_eq!(state.snapshot().rng.word_pos, snapshot.rng.word_pos);
    let record = AppliedTickRecord::from_applied(zone, &exchanged);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    log.append_applied(&record).await.unwrap();
    let projection = exchanged
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
    assert_eq!(projection.exchanges.len(), 2);
    let references: Vec<_> = projection
        .exchanges
        .iter()
        .map(|exchange| exchange.receipt)
        .collect();
    let shared: Vec<_> = projection
        .ledger
        .iter()
        .filter(|row| row.receipt_refs.len() == 2)
        .collect();
    assert_eq!(shared.len(), 2);
    assert!(shared
        .iter()
        .all(|row| row.receipt.is_none() && row.receipt_refs == references));
    let work = ItemCheckpointWork {
        zone,
        epoch: 1,
        allocation: state.snapshot().phase4.unwrap().allocator,
        projection: projection.clone(),
    };
    let request = api::application::CharacterCheckpoint {
        character_id: c.id,
        revision_seen: before_exchange.revision,
        level: before_exchange.level,
        xp: before_exchange.xp,
        hp: before_exchange.hp.unwrap(),
        mp: before_exchange.mp.unwrap(),
        alive: before_exchange.alive,
        position: before_exchange.position,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(before_exchange.class_state.clone()),
        phase3: before_exchange.phase3.clone(),
        phase4: Some(projection.checkpoint.clone()),
    };
    let audits: Vec<_> = projection
        .exchanges
        .iter()
        .enumerate()
        .map(|(index, exchange)| DomainEvent::ItemExchanged {
            fact: api::domain::ItemEventFact {
                metadata: api::domain::EventMetadata {
                    event_id: Uuid::now_v7(),
                    sequence: (
                        before_exchange.revision.checked_add(1).unwrap(),
                        u64::try_from(index).unwrap(),
                    ),
                },
                character_id: c.id,
                instance_id: exchange.before.id,
                template_id: exchange.before.template_id,
                count_delta: -1,
                before: Some(exchange.before.clone()),
                after: None,
                causes: vec![serde_json::to_value(exchange).unwrap()],
            },
        })
        .collect();
    work.validate_events(&request, &audits).unwrap();
    let mut duplicate = audits.clone();
    duplicate.push(audits[0].clone());
    let mut altered = audits.clone();
    let DomainEvent::ItemExchanged { fact } = &mut altered[0] else {
        unreachable!()
    };
    fact.causes[0]["after"]["id"] = serde_json::json!(projection.exchanges[0].before.id);
    for invalid in [Vec::new(), duplicate, altered] {
        assert!(matches!(
            repo.checkpoint_items(&request, &invalid, &work, None).await,
            Err(api::application::CheckpointError::Constraint(_))
        ));
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before_exchange);
    }
    service
        .admitted(&exchanged, &state.snapshot())
        .await
        .unwrap();
    for (index, command) in commands.iter().enumerate() {
        let ItemReceiptLookup::Known(receipt) = repo
            .item_receipt_lookup(
                c.account_id,
                ItemMutationOperation::ExchangeSpecialAbility,
                &IdempotencyKey::from_uuid(command.request_id),
                &command.fingerprint_hex().unwrap(),
            )
            .await
            .unwrap()
        else {
            panic!("missing exchange receipt")
        };
        let audit = &projection.exchanges[index];
        assert_ne!(audit.before.id, audit.after.id);
        assert_eq!(receipt.result.accepted_sequence, Some(if index == 0 { 5 } else { 7 }));
        assert_eq!(audit.after.enchant, audit.before.enchant);
    }
    let retained = repo
        .retained_inventory(c.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert_eq!(retained, projection.checkpoint);
    assert!(!retained
        .instances
        .iter()
        .any(|item| bases.contains(&item.id)));
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
    let exchanged: Vec<_> = repo
        .staged_events()
        .into_iter()
        .filter_map(|event| {
            if let DomainEvent::ItemExchanged { fact } = event {
                Some(fact)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(exchanged.len(), 2);
    assert!(exchanged.iter().all(|fact| fact.after.is_none()
        && fact.causes[0]["before"]["id"] != fact.causes[0]["after"]["id"]));
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
    let exchanged: Vec<serde_json::Value> = sqlx::query_scalar("SELECT payload->'fact' FROM outbox WHERE subject='nightfall.item.exchanged' AND payload->'fact'->>'character_id'=$1").bind(id.to_string()).fetch_all(&pool).await.unwrap();
    assert_eq!(exchanged.len(), 2);
    assert!(exchanged
        .iter()
        .all(|fact| fact["causes"][0]["before"]["id"] != fact["causes"][0]["after"]["id"]));
    let shared: Vec<serde_json::Value> = sqlx::query_scalar("SELECT delta FROM item_ledger WHERE character_id=$1 AND op='consume' AND jsonb_array_length(delta->'receipt_refs')=2")
        .bind(id.as_uuid()).fetch_all(&pool).await.unwrap();
    assert_eq!(shared.len(), 2);
    assert!(shared.iter().all(
        |row| row.get("receipt").is_none() && row["receipt_refs"][0] != row["receipt_refs"][1]
    ));
}
