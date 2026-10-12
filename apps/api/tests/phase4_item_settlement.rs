//! Isolated acknowledged-projection storage contracts; live kit/fold/gameplay gates are separate.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod common;
use nightfall_api::application::ports::*;
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::zone::phase4_checkpoint::StarterKitReceipt;
use nightfall_api::domain::zone::phase4_projection::*;
use nightfall_api::domain::zone::{
    EntityId, ItemInstance, ItemInstanceRange, ItemLocation, Phase3Checkpoint, Phase4Checkpoint,
    Phase4Projection, Tick, ZoneId,
};
use nightfall_api::domain::{
    AccountId, Character, CharacterId, CharacterName, DomainEvent, EventMetadata, ItemEventFact,
    Position, Race,
};
use nightfall_api::infrastructure::{
    item_data::{embedded_phase4_bundle, Phase4RuleBundle},
    memory::InMemoryCharacterRepository,
    postgres::PgCharacterRepository,
};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[allow(clippy::too_many_lines)] // One complete immutable materialization fixture.
async fn fixture<R: CharacterRepository + ItemAdmissionRepository>(
    repo: &R,
    bundle: &Phase4RuleBundle,
    name: &str,
) -> (Character, CharacterCheckpoint, ItemCheckpointWork, Vec<DomainEvent>) {
    let character = Character::create(
        AccountId::from_uuid(Uuid::from_u128(1)),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), name, &character)
        .await
        .unwrap();
    let pending = repo
        .pending_item_requests(character.id)
        .await
        .unwrap()
        .remove(0);
    let PendingItemManifest::Kit(kit) = pending.manifest.clone();
    let reserved = repo.reserve_item_range(ZoneId(1), 1).await.unwrap();
    let mut allocation = ItemInstanceRange::new(
        reserved.range_start,
        reserved.next_reserved,
        reserved.range_end_exclusive,
    )
    .unwrap();
    // Full manifest counts are retained; slot/stat execution is deliberately not this adapter fixture.
    let instances: Vec<_> = kit
        .items
        .iter()
        .map(|row| ItemInstance {
            id: allocation.allocate().unwrap(),
            template_id: row.template_id,
            count: row.count,
            location: ItemLocation::Inventory,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: Tick(73),
            revision: 1,
        })
        .collect();
    let checkpoint = Phase4Checkpoint {
        schema_version: 1,
        timer_policy_version: 1,
        rules_hash: bundle.items.hash(),
        inventory_revision: 1,
        instances: instances.clone(),
        paperdoll: BTreeMap::new(),
        auto_shots: BTreeSet::new(),
        item_reuse_remainders: Vec::new(),
        equipment_install_order: Vec::new(),
        active_sets: BTreeMap::new(),
        weight: 0,
        max_load: 100_000,
        ordinary_capacity: 80,
        quest_capacity: 100,
        weight_penalty_bonus: 1,
        overload_level: 0,
        overloaded: false,
        expertise: 0,
        weapon_penalty: 0,
        armor_penalty: 0,
        starter_kit_receipt: Some(StarterKitReceipt {
            manifest_version: pending.manifest_version,
            manifest_hash: pending.manifest_hash,
            source_class: kit.source_class_id,
            request_id: pending.request_id,
            materialized_revision: 1,
        }),
    };
    let cp = CharacterCheckpoint {
        character_id: character.id,
        revision_seen: 0,
        level: character.level,
        xp: character.xp,
        hp: 20,
        mp: 10,
        alive: true,
        position: Position::default(),
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(character.class_state.clone()),
        phase3: Some(Phase3Checkpoint::new()),
        phase4: Some(checkpoint.clone()),
    };
    let work = ItemCheckpointWork {
        zone: ZoneId(1),
        epoch: 1,
        allocation,
        projection: Phase4Projection {
            enchant_attempts: Vec::new(),
            paused_core: None,
            tick: Tick(73),
            entity: EntityId::from_uuid(character.id.as_uuid()),
            account: character.account_id,
            checkpoint,
            ledger: instances
                .iter()
                .map(|item| ItemDeltaPlan {
                    operation: ItemOperation::Starter,
                    operation_ordinal: 0,
                    instance: item.id,
                    before: None,
                    after: Some(item.clone()),
                    count_delta: i64::try_from(item.count).unwrap(),
                    receipt: Some(ItemReceiptRef {
                        operation: ItemOperation::Starter,
                        key: pending.request_id,
                    }),
                    causes: Vec::new(),
                })
                .collect(),
            receipts: vec![ItemReceiptFact {
                request_id: pending.request_id,
                fingerprint: pending.fingerprint,
                result: FrozenItemResult {
                    entity: EntityId::from_uuid(character.id.as_uuid()),
                    incarnation: 4,
                    tick: Tick(73),
                    inventory_revision: 1,
                    receipt: pending.request_id,
                    detail: ItemResultDetail::Operation {
                        operation: ItemOperation::Starter,
                        affected: instances.iter().map(|item| item.id).collect(),
                        recipe: None,
                    },
                },
            }],
            materializations: vec![ItemMaterializationFact {
                request_id: pending.request_id,
                manifest_version: pending.manifest_version,
                manifest_hash: pending.manifest_hash,
                inventory_revision: 1,
                receipt: ItemReceiptRef {
                    operation: ItemOperation::Starter,
                    key: pending.request_id,
                },
            }],
            evidence: Vec::new(),
        },
    };
    work.projection.validate_structure(0).unwrap();
    let events = events(&cp, &work);
    (character, cp, work, events)
}
fn events(cp: &CharacterCheckpoint, work: &ItemCheckpointWork) -> Vec<DomainEvent> {
    work.projection
        .ledger
        .iter()
        .enumerate()
        .map(|(ordinal, delta)| {
            let fact = ItemEventFact {
                metadata: EventMetadata {
                    event_id: Uuid::from_u128(
                        1000_u128
                            .checked_add(u128::try_from(ordinal).unwrap())
                            .unwrap(),
                    ),
                    sequence: (
                        cp.revision_seen.checked_add(1).unwrap(),
                        u64::try_from(ordinal).unwrap(),
                    ),
                },
                character_id: cp.character_id,
                instance_id: delta.instance,
                template_id: delta.template().unwrap(),
                count_delta: delta.count_delta,
                before: delta.before.clone(),
                after: delta.after.clone(),
                causes: delta
                    .causes
                    .iter()
                    .map(|cause| serde_json::to_value(cause).unwrap())
                    .collect(),
            };
            match delta.operation {
                ItemOperation::Starter | ItemOperation::Backfill | ItemOperation::Grant => {
                    DomainEvent::ItemGranted { fact }
                },
                ItemOperation::Consume => DomainEvent::ItemConsumed { fact },
                ItemOperation::Destroy | ItemOperation::Crystallize => {
                    DomainEvent::ItemDestroyed { fact }
                },
                ItemOperation::Equip => DomainEvent::ItemEquipped { fact },
                ItemOperation::Unequip => DomainEvent::ItemUnequipped { fact },
                ItemOperation::Enchant => DomainEvent::ItemEnchanted { fact },
                ItemOperation::Refine | ItemOperation::CancelRefine => {
                    DomainEvent::ItemRefined { fact }
                },
                ItemOperation::Exchange => DomainEvent::ItemExchanged { fact },
                ItemOperation::Use | ItemOperation::ToggleAutoShot => {
                    panic!("owner-only operations cannot be fixture ledger rows")
                },
            }
        })
        .collect()
}
fn interleaved(
    first: &CharacterCheckpoint,
    first_work: &ItemCheckpointWork,
) -> (CharacterCheckpoint, ItemCheckpointWork, Vec<DomainEvent>) {
    let mut cp = first.clone();
    cp.revision_seen = 1;
    cp.idempotency.1 = IdempotencyKey::new();
    let retained = cp.phase4.as_mut().unwrap();
    retained.inventory_revision = 4;
    let target = retained
        .instances
        .iter_mut()
        .max_by_key(|item| item.count)
        .unwrap();
    let before = target.clone();
    let mut grant_before = before.clone();
    grant_before.count = grant_before.count.checked_sub(2).unwrap();
    grant_before.revision = 2;
    let mut grant_after = grant_before.clone();
    grant_after.count = grant_after.count.checked_add(5).unwrap();
    grant_after.revision = 3;
    target.count = target.count.checked_sub(1).unwrap();
    target.revision = 4;
    let after = target.clone();
    let target_id = target.id;
    let request = Uuid::now_v7();
    let cause = |ordinal, count| ItemAuditEvidence {
        family: "consume".into(),
        fields: BTreeMap::from([
            ("ordinal".into(), ItemEvidenceValue::Unsigned(ordinal)),
            ("count".into(), ItemEvidenceValue::Unsigned(count)),
        ]),
    };
    let mut work = first_work.clone();
    work.projection.tick = Tick(74);
    work.projection.checkpoint = retained.clone();
    work.projection.ledger = vec![
        ItemDeltaPlan {
            operation: ItemOperation::Grant,
            operation_ordinal: 1,
            instance: target_id,
            before: Some(grant_before),
            after: Some(grant_after),
            count_delta: 5,
            receipt: Some(ItemReceiptRef {
                operation: ItemOperation::Grant,
                key: request,
            }),
            causes: Vec::new(),
        },
        ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: target_id,
            before: Some(before),
            after: Some(after),
            count_delta: -6,
            receipt: Some(ItemReceiptRef {
                operation: ItemOperation::Use,
                key: request,
            }),
            causes: vec![cause(0, 2), cause(2, 4)],
        },
    ];
    work.projection.receipts = vec![ItemReceiptFact {
        request_id: request,
        fingerprint: "grant-body-five".into(),
        result: FrozenItemResult {
            entity: work.projection.entity,
            incarnation: 4,
            tick: Tick(74),
            inventory_revision: 3,
            receipt: request,
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::Grant,
                affected: vec![target_id],
                recipe: None,
            },
        },
    }];
    let mut use_receipt = work.projection.receipts.first().unwrap().clone();
    use_receipt.fingerprint = "use-body-six".into();
    use_receipt.result.inventory_revision = 4;
    use_receipt.result.detail = ItemResultDetail::Operation {
        operation: ItemOperation::Use,
        affected: vec![target_id],
        recipe: None,
    };
    work.projection.receipts.push(use_receipt);
    work.projection.materializations.clear();
    let facts = events(&cp, &work);
    (cp, work, facts)
}
fn destroy(
    previous: &CharacterCheckpoint,
    previous_work: &ItemCheckpointWork,
) -> (CharacterCheckpoint, ItemCheckpointWork, Vec<DomainEvent>) {
    let mut cp = previous.clone();
    cp.revision_seen = 2;
    cp.idempotency.1 = IdempotencyKey::new();
    let checkpoint = cp.phase4.as_mut().unwrap();
    checkpoint.inventory_revision = 5;
    let target = previous_work.projection.ledger.first().unwrap().instance;
    let index = checkpoint
        .instances
        .iter()
        .position(|item| item.id == target)
        .unwrap();
    let before = checkpoint.instances.remove(index);
    let request = Uuid::now_v7();
    let mut work = previous_work.clone();
    work.projection.tick = Tick(75);
    work.projection.checkpoint = checkpoint.clone();
    work.projection.ledger = vec![ItemDeltaPlan {
        operation: ItemOperation::Destroy,
        operation_ordinal: 0,
        instance: target,
        before: Some(before.clone()),
        after: None,
        count_delta: i64::try_from(before.count).unwrap().checked_neg().unwrap(),
        receipt: Some(ItemReceiptRef {
            operation: ItemOperation::Destroy,
            key: request,
        }),
        causes: Vec::new(),
    }];
    work.projection.receipts = vec![ItemReceiptFact {
        request_id: request,
        fingerprint: "destroy-body".into(),
        result: FrozenItemResult {
            entity: work.projection.entity,
            incarnation: 4,
            tick: Tick(75),
            inventory_revision: 5,
            receipt: request,
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::Destroy,
                affected: vec![target],
                recipe: None,
            },
        },
    }];
    let facts = events(&cp, &work);
    (cp, work, facts)
}
#[allow(clippy::too_many_lines)] // Same adapter contract including historical immutable retries.
async fn contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemInventoryRepository
        + ItemCheckpointRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) -> (Character, ItemCheckpointWork) {
    let (character, cp, work, facts) = fixture(repo, bundle, "ItemSettlement").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(1)
    );
    assert!(repo
        .pending_item_requests(character.id)
        .await
        .unwrap()
        .iter()
        .all(|request| matches!(request.status, PendingItemStatus::Materialized { .. })));
    assert_eq!(
        repo.load_for_admission(character.id)
            .await
            .unwrap()
            .unwrap()
            .phase4
            .unwrap()
            .pending
            .len(),
        0
    );
    let (second, second_work, second_facts) = interleaved(&cp, &work);
    assert_eq!(
        repo.checkpoint_items(&second, &second_facts, &second_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    let receipt = second_work.projection.receipts.first().unwrap();
    let encoded = serde_json::to_value(receipt).unwrap();
    assert_eq!(
        encoded.get("result").unwrap(),
        &serde_json::to_value(receipt.result.payload().unwrap().as_bytes()).unwrap()
    );
    let key = IdempotencyKey::from_uuid(receipt.request_id);
    let use_receipt = second_work.projection.receipts.get(1).unwrap();
    let ItemReceiptLookup::Known(used) = repo
        .item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::UseItem,
            &key,
            &use_receipt.fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("same-tick Use receipt must coexist with Grant");
    };
    assert_eq!(used.result, use_receipt.result);
    let persisted = serde_json::to_value(&used).unwrap();
    assert_eq!(
        persisted["result"],
        serde_json::to_value(use_receipt.result.payload().unwrap().as_bytes()).unwrap()
    );
    assert_eq!(
        serde_json::from_value::<Box<nightfall_api::application::ports::ItemMutationReceipt>>(
            persisted
        )
        .unwrap(),
        used
    );
    let known = repo
        .item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::GrantItem,
            &key,
            &receipt.fingerprint,
        )
        .await
        .unwrap();
    let frozen = match &known {
        ItemReceiptLookup::Known(receipt) => Some(receipt),
        ItemReceiptLookup::Unknown | ItemReceiptLookup::Conflict { .. } => None,
    }
    .unwrap();
    assert_eq!(frozen.inventory.as_ref().unwrap().character_revision, 2);
    assert_eq!(
        frozen
            .inventory
            .as_ref()
            .unwrap()
            .checkpoint
            .inventory_revision,
        4
    );
    assert_eq!(frozen.result.inventory_revision, 3);
    let (third, third_work, third_facts) = destroy(&second, &second_work);
    assert_eq!(
        repo.checkpoint_items(&third, &third_facts, &third_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(3)
    );
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::GrantItem,
            &key,
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        known
    );
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(1)
    );
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::GrantItem,
            &key,
            "changed-body"
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Conflict {
            stored_fingerprint: receipt.fingerprint.clone()
        }
    );
    assert_eq!(
        repo.item_receipt_lookup(
            AccountId::from_uuid(Uuid::from_u128(99)),
            ItemMutationOperation::GrantItem,
            &key,
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Unknown
    );
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::EquipItem,
            &key,
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Unknown
    );
    let mut duplicate = second.clone();
    duplicate.idempotency.1 = IdempotencyKey::new();
    assert_eq!(
        repo.checkpoint_items(&duplicate, &second_facts, &second_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Stale
    );
    let mut conflicting = second_work.clone();
    conflicting
        .projection
        .receipts
        .first_mut()
        .unwrap()
        .fingerprint = "changed-body".into();
    assert!(matches!(
        repo.checkpoint_items(&duplicate, &second_facts, &conflicting, None)
            .await,
        Err(CheckpointError::KeyReused)
    ));
    let mut toggled = third.clone();
    toggled.revision_seen = 3;
    toggled.idempotency.1 = IdempotencyKey::new();
    let checkpoint = toggled.phase4.as_mut().unwrap();
    checkpoint.inventory_revision = 6;
    checkpoint.auto_shots.insert(1463);
    let mut toggle_work = third_work.clone();
    toggle_work.projection.tick = Tick(76);
    toggle_work.projection.checkpoint = checkpoint.clone();
    toggle_work.projection.ledger.clear();
    let request_id = Uuid::now_v7();
    toggle_work.projection.receipts = vec![ItemReceiptFact {
        request_id,
        fingerprint: "toggle-with-no-stack".into(),
        result: FrozenItemResult {
            entity: toggle_work.projection.entity,
            incarnation: 4,
            tick: Tick(76),
            inventory_revision: 6,
            receipt: request_id,
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::ToggleAutoShot,
                affected: vec![],
                recipe: None,
            },
        },
    }];
    assert_eq!(
        repo.checkpoint_items(&toggled, &[], &toggle_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(4)
    );
    assert_eq!(
        repo.checkpoint_items(&toggled, &[], &toggle_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(4)
    );
    assert!(matches!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::ToggleAutoShot,
            &IdempotencyKey::from_uuid(request_id),
            "toggle-with-no-stack"
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Known(_)
    ));
    let current = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.character_revision, 4);
    assert_eq!(current.checkpoint, toggled.phase4);
    let metadata = repo
        .get(character.id)
        .await
        .unwrap()
        .unwrap()
        .item_metadata
        .unwrap();
    assert_eq!(metadata.inventory_revision, 6);
    assert_eq!(metadata.rules_hash, bundle.items.hash());
    assert_eq!(
        repo.list_by_account(character.account_id).await.unwrap()[0].item_metadata,
        Some(metadata)
    );

    (character, second_work)
}
#[tokio::test]
async fn memory_atomic_item_projection_receipts_batching_and_revision_fences() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (_, work) = contract(&repo, &bundle).await;
    let consumed: Vec<_> = repo
        .item_ledger_for_test()
        .into_iter()
        .filter(|row| row.operation == ItemLedgerOperation::Consume)
        .collect();
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0].delta.count_delta, -6);
    assert_eq!(consumed[0].delta.causes.len(), 2);
    assert_eq!(consumed[0].delta.receipt.unwrap().operation, ItemOperation::Use);
    assert_eq!(consumed[0].instance_id, work.projection.ledger[0].instance);
}
#[tokio::test]
async fn postgres_atomic_item_projection_receipts_batching_and_revision_fences() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, _) = contract(&repo, &bundle).await;
    let rows: Vec<(i32, serde_json::Value)> = sqlx::query_as(
        "SELECT operation_ordinal,delta FROM item_ledger WHERE character_id=$1 AND op='consume'",
    )
    .bind(character.id.as_uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, 0);
    let delta: ItemLedgerDelta = serde_json::from_value(rows[0].1.clone()).unwrap();
    assert_eq!(delta.count_delta, -6);
    assert_eq!(delta.causes.len(), 2);
    assert_eq!(delta.receipt.unwrap().operation, ItemOperation::Use);
    let cursor: i64 =
        sqlx::query_scalar("SELECT next_reserved FROM item_id_ranges WHERE zone_id=1 AND epoch=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(cursor > 4_294_967_296);
}
#[tokio::test]
async fn postgres_outbox_failure_rolls_back_items_ledger_receipts_pending_range_and_parent() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, cp, work, facts) = fixture(&repo, &bundle, "ItemRollback").await;
    sqlx::raw_sql("CREATE FUNCTION reject_item_outbox() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.subject LIKE 'nightfall.item.%' THEN RAISE EXCEPTION 'injected outbox failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_item_outbox BEFORE INSERT ON outbox FOR EACH ROW EXECUTE FUNCTION reject_item_outbox();").execute(&pool).await.unwrap();
    assert!(repo
        .checkpoint_items(&cp, &facts, &work, None)
        .await
        .is_err());
    assert_eq!(
        repo.retained_inventory(character.id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        0
    );
    for (table, query) in [
        ("item_instances", "SELECT count(*) FROM item_instances"),
        ("item_ledger", "SELECT count(*) FROM item_ledger"),
        ("item_mutation_receipts", "SELECT count(*) FROM item_mutation_receipts"),
    ] {
        let count: i64 = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0, "{table} rolled back");
    }
    let pending = repo.pending_item_requests(character.id).await.unwrap();
    assert!(matches!(pending[0].status, PendingItemStatus::Pending));
    let receipt = work.projection.receipts.first().unwrap();
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::Starter,
            &IdempotencyKey::from_uuid(receipt.request_id),
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Unknown
    );
    let reserved = repo.reserve_item_range(ZoneId(1), 1).await.unwrap();
    assert_eq!(reserved.next_reserved, reserved.range_start);
    sqlx::query("DROP TRIGGER reject_item_outbox ON outbox")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::Starter,
            &IdempotencyKey::from_uuid(receipt.request_id),
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Known(Box::new(work.receipts(&cp).unwrap().remove(0)))
    );
}

async fn concurrent_contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) {
    let (character, cp, work, facts) = fixture(repo, bundle, "ItemConcurrent").await;
    let (left, right) = tokio::join!(
        repo.checkpoint_items(&cp, &facts, &work, None),
        repo.checkpoint_items(&cp, &facts, &work, None)
    );
    let mut outcomes = vec![left.unwrap(), right.unwrap()];
    assert!(outcomes.contains(&CheckpointOutcome::Applied(1)));
    assert!(outcomes.contains(&CheckpointOutcome::Replayed(1)));
    outcomes.clear();
    let (next, next_work, next_facts) = interleaved(&cp, &work);
    let mut other = next.clone();
    other.idempotency.1 = IdempotencyKey::new();
    let (left, right) = tokio::join!(
        repo.checkpoint_items(&next, &next_facts, &next_work, None),
        repo.checkpoint_items(&other, &next_facts, &next_work, None)
    );
    outcomes.extend([left.unwrap(), right.unwrap()]);
    assert!(outcomes.contains(&CheckpointOutcome::Applied(2)));
    assert!(outcomes.contains(&CheckpointOutcome::Stale));
    assert_eq!(
        repo.retained_inventory(character.id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        2
    );
}

#[tokio::test]
async fn memory_concurrent_item_keys_settle_once() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    concurrent_contract(&repo, &bundle).await;
}

#[tokio::test]
async fn postgres_concurrent_item_keys_settle_once() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    concurrent_contract(&repo, &bundle).await;
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM item_mutation_receipts WHERE operation='grant_item'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 1);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM item_ledger WHERE op='consume'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
}

async fn rejected_contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) {
    let (character, cp, work, facts) = fixture(repo, bundle, "ItemRejected").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let (next, valid, events) = interleaved(&cp, &work);
    let before = repo.retained_inventory(character.id).await.unwrap();
    let mut corrupt = valid.clone();
    corrupt.projection.ledger.last_mut().unwrap().count_delta = -7;
    assert!(
        matches!(repo.checkpoint_items(&next, &events, &corrupt, None).await, Err(CheckpointError::Constraint(name)) if name == "item_ledger_count_conservation")
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    let mut corrupt = valid.clone();
    corrupt
        .projection
        .ledger
        .last_mut()
        .unwrap()
        .before
        .as_mut()
        .unwrap()
        .created_tick = Tick(72);
    assert!(
        matches!(repo.checkpoint_items(&next, &events, &corrupt, None).await, Err(CheckpointError::Constraint(name)) if name == "item_ledger_initial_scalar")
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    let mut corrupt = valid.clone();
    corrupt.projection.account = AccountId::from_uuid(Uuid::from_u128(7));
    assert!(
        matches!(repo.checkpoint_items(&next, &events, &corrupt, None).await, Err(CheckpointError::Constraint(name)) if name == "item_checkpoint_identity")
    );
    let receipt = &valid.projection.receipts[0];
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::GrantItem,
            &IdempotencyKey::from_uuid(receipt.request_id),
            &receipt.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Unknown
    );
    let mut transient = valid.clone();
    let id = transient.allocation.allocate().unwrap();
    let created = ItemInstance {
        id,
        template_id: 57,
        count: 5,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(74),
        revision: 4,
    };
    let reference = transient.projection.receipts[0].reference();
    transient.projection.ledger.extend([
        ItemDeltaPlan {
            operation: ItemOperation::Grant,
            operation_ordinal: 3,
            instance: id,
            before: None,
            after: Some(created.clone()),
            count_delta: 5,
            receipt: Some(reference),
            causes: vec![],
        },
        ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: id,
            before: Some(created),
            after: None,
            count_delta: -6,
            receipt: None,
            causes: vec![ItemAuditEvidence {
                family: "consume".into(),
                fields: BTreeMap::new(),
            }],
        },
    ]);
    assert!(matches!(repo.checkpoint_items(&next, &events, &transient, None).await,
        Err(CheckpointError::Constraint(name)) if name == "item_ledger_count_conservation"));
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    let mut stale = next.clone();
    stale.revision_seen = 0;
    let stale_events = self::events(&stale, &valid);
    assert_eq!(
        repo.checkpoint_items(&stale, &stale_events, &valid, None)
            .await
            .unwrap(),
        CheckpointOutcome::Stale
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    assert_eq!(
        repo.checkpoint_items(&next, &events, &valid, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
}

#[tokio::test]
async fn memory_rejects_unconserved_counts_wrong_initial_scalar_owner_and_stale_fence() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    rejected_contract(&repo, &bundle).await;
}

#[tokio::test]
async fn postgres_rejects_unconserved_counts_wrong_initial_scalar_owner_and_stale_fence() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool)
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    rejected_contract(&repo, &bundle).await;
}

#[allow(clippy::too_many_lines)] // Shared memory/Pg atomic unchanged-attempt contract.
async fn unchanged_enchant_contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) -> CharacterId {
    use nightfall_api::domain::zone::{EnchantAttempt, ItemInstanceId};
    let (character, mut cp, mut work, facts) = fixture(repo, bundle, "SafeAudit").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let retained = repo.retained_inventory(character.id).await.unwrap();
    cp.revision_seen = 1;
    cp.idempotency.1 = IdempotencyKey::new();
    cp.phase4.as_mut().unwrap().inventory_revision = 2;
    work.projection.tick = Tick(74);
    work.projection.checkpoint = cp.phase4.clone().unwrap();
    work.projection.ledger.clear();
    work.projection.materializations.clear();
    let target = work
        .projection
        .checkpoint
        .instances
        .iter()
        .find(|item| item.count == 1)
        .unwrap()
        .clone();
    let receipt = &mut work.projection.receipts[0];
    receipt.request_id = Uuid::now_v7();
    receipt.fingerprint = "b".repeat(64);
    receipt.result.receipt = receipt.request_id;
    receipt.result.tick = Tick(74);
    receipt.result.inventory_revision = 2;
    receipt.result.detail = ItemResultDetail::Enchant {
        target: target.id,
        old_enchant: target.enchant,
        new_enchant: target.enchant,
        outcome: ItemEnchantOutcome::SafeFailed,
        refunds: vec![],
        announcement_tier: 0,
    };
    let request = receipt.clone();
    let attempt = EnchantAttempt {
        operation_ordinal: 1,
        receipt: receipt.reference(),
        target: target.id,
        template: target.template_id,
        old_enchant: target.enchant,
        new_enchant: target.enchant,
        outcome: ItemEnchantOutcome::SafeFailed,
        before: target.clone(),
        after: Some(target.clone()),
        scroll: ItemInstanceId::new(99).unwrap(),
        scroll_template: 951,
        chance_1e5: 0,
        raw_draw: 199_999,
        draw: 99_999,
        cause: ItemAuditEvidence {
            family: "enchant".into(),
            fields: BTreeMap::new(),
        },
    };
    work.projection.enchant_attempts = vec![attempt.clone()];
    let fact = DomainEvent::ItemEnchanted {
        fact: ItemEventFact {
            metadata: EventMetadata {
                event_id: Uuid::now_v7(),
                sequence: (2, 0),
            },
            character_id: character.id,
            instance_id: target.id,
            template_id: target.template_id,
            count_delta: 0,
            before: Some(target.clone()),
            after: Some(target),
            causes: vec![serde_json::to_value(attempt).unwrap()],
        },
    };
    assert!(matches!(
        repo.checkpoint_items(&cp, &[], &work, None).await,
        Err(CheckpointError::Constraint(_))
    ));
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), retained);
    let key = IdempotencyKey::from_uuid(request.request_id);
    assert_eq!(
        repo.item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::EnchantItem,
            &key,
            &request.fingerprint
        )
        .await
        .unwrap(),
        ItemReceiptLookup::Unknown
    );
    let events = vec![fact];
    assert_eq!(
        repo.checkpoint_items(&cp, &events, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    assert_eq!(
        repo.checkpoint_items(&cp, &events, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(2)
    );
    let ItemReceiptLookup::Known(stored) = repo
        .item_receipt_lookup(
            character.account_id,
            ItemMutationOperation::EnchantItem,
            &key,
            &request.fingerprint,
        )
        .await
        .unwrap()
    else {
        panic!("missing enchant receipt");
    };
    assert_eq!(stored.result, request.result);
    let saved = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.checkpoint.unwrap().instances,
        retained.unwrap().checkpoint.unwrap().instances
    );
    character.id
}

#[tokio::test]
async fn memory_unchanged_enchant_audit_is_atomic_without_phantom_ledger() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    unchanged_enchant_contract(&repo, &bundle).await;
    assert!(repo
        .item_ledger_for_test()
        .iter()
        .all(|row| row.operation != ItemLedgerOperation::Enchant));
    assert_eq!(
        repo.staged_events()
            .iter()
            .filter(|event| matches!(event, DomainEvent::ItemEnchanted { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn postgres_unchanged_enchant_audit_is_atomic_without_phantom_ledger() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let id = unchanged_enchant_contract(&repo, &bundle).await;
    let ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM item_ledger WHERE character_id=$1 AND op='enchant'",
    )
    .bind(id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger, 0);
    let outbox: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE payload->'fact'->>'character_id'=$1 AND subject='nightfall.item.enchanted'",
    )
    .bind(id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox, 1);
}
