//! Isolated acknowledged-projection storage contracts; live kit/fold/gameplay gates are separate.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
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
use std::sync::Arc;
/// D77 read paths surface the typed permanent constraint through `anyhow`.
fn assert_constraint(error: &anyhow::Error, expected: &str) {
    assert!(
        matches!(error.downcast_ref::<CheckpointError>(), Some(CheckpointError::Constraint(name)) if name == expected),
        "expected {expected}: {error:?}"
    );
}
use uuid::Uuid;

// Storage-only: simultaneous paid rank and item consumption, not book eligibility/gameplay.
#[allow(clippy::too_many_lines)]
async fn unified_learning_contract<R>(
    repo: Arc<R>,
    bundle: &Phase4RuleBundle,
    pool: Option<&sqlx::PgPool>,
) where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + LearnReceiptRepository,
{
    use nightfall_api::domain::subclass::LearnedSkill;
    use nightfall_api::domain::zone::skill::{SkillId, SkillRef};
    let (c, mut baseline, baseline_work, baseline_events) =
        fixture(repo.as_ref(), bundle, "UnifiedLearn").await;
    baseline.level = 5;
    baseline.class_state.as_mut().unwrap().sp = 100;
    assert_eq!(
        repo.checkpoint_items(&baseline, &baseline_events, &baseline_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let before = repo.load_for_admission(c.id).await.unwrap();
    let mut cp = baseline.clone();
    cp.revision_seen = 1;
    cp.idempotency.1 = IdempotencyKey::new();
    let ledger = cp.class_state.as_mut().unwrap();
    ledger.sp = 50;
    ledger.merge_learned_skills([LearnedSkill {
        key: "l2.skill.3".into(),
        level: 1,
    }]);
    let retained = cp.phase4.as_mut().unwrap();
    retained.inventory_revision = 2;
    let scroll = retained
        .instances
        .iter_mut()
        .find(|item| item.count > 1)
        .unwrap();
    let old_scroll = scroll.clone();
    scroll.count = scroll.count.checked_sub(1).unwrap();
    scroll.revision = 2;
    let new_scroll = scroll.clone();
    let mut work = baseline_work.clone();
    work.projection.tick = Tick(74);
    work.projection.checkpoint = retained.clone();
    work.projection.receipts.clear();
    work.projection.materializations.clear();
    work.projection.ledger = vec![ItemDeltaPlan {
        receipt_refs: Vec::new(),
        operation: ItemOperation::Consume,
        operation_ordinal: 0,
        instance: old_scroll.id,
        before: Some(old_scroll),
        after: Some(new_scroll),
        count_delta: -1,
        receipt: None,
        causes: vec![ItemAuditEvidence {
            family: "skill_cast_payment".into(),
            fields: BTreeMap::from([
                ("cast_id".into(), ItemEvidenceValue::Unsigned(1)),
                ("count_delta".into(), ItemEvidenceValue::Signed(-1)),
            ]),
        }],
    }];
    let receipt = SuccessfulLearnReceipt {
        account_id: c.account_id,
        key: IdempotencyKey::new(),
        request_fingerprint: learn_skill_fingerprint(
            c.id,
            SkillRef {
                id: SkillId(3),
                rank: 1,
            },
        ),
        result: FrozenLearnResult {
            character_id: c.id,
            skill: SkillRef {
                id: SkillId(3),
                rank: 1,
            },
            sp_spent: 50,
            sp_remaining: 50,
            tick: Tick(74),
            hp: cp.hp,
            max_hp: cp.hp,
            mp: cp.mp,
            max_mp: cp.mp,
            cp: 0,
            max_cp: 0,
            effective_stats: LearnedEffectiveStats {
                p_atk_q: 0,
                p_def_q: 0,
                m_atk_q: 0,
                m_def_q: 0,
                attack_speed: 0,
                cast_speed: 0,
                speed_q: 0,
                accuracy_q: 0,
                evasion_q: 0,
                crit_permille: 0,
            },
            incarnation: 4,
        },
    };
    let mut outbox = events(&cp, &work);
    outbox.push(DomainEvent::CharacterSkillLearned {
        metadata: EventMetadata {
            event_id: Uuid::now_v7(),
            sequence: (2, 1),
        },
        character_id: c.id,
        skill_id: 3,
        skill_level: 1,
        sp_spent: 50,
        sp_remaining: 50,
        tick: 74,
        request_key: receipt.key.as_uuid(),
    });
    let lookup =
        || repo.learn_receipt_lookup(c.account_id, &receipt.key, &receipt.request_fingerprint);
    let mut stale = cp.clone();
    stale.revision_seen = 7;
    let stale_events = events(&stale, &work);
    assert_eq!(
        repo.checkpoint_items(&stale, &stale_events, &work, Some(&receipt))
            .await
            .unwrap(),
        CheckpointOutcome::Stale
    );
    let mut invalid = work.clone();
    invalid.projection.ledger[0].count_delta = -2;
    assert!(repo
        .checkpoint_items(&cp, &outbox, &invalid, Some(&receipt))
        .await
        .is_err());
    assert_eq!(lookup().await.unwrap(), LearnReceiptLookup::Unknown);
    assert_eq!(repo.load_for_admission(c.id).await.unwrap(), before);
    if let Some(pool) = pool {
        sqlx::query("ALTER TABLE outbox ADD CONSTRAINT fail_unified_learn CHECK (subject <> 'nightfall.character.skill_learned')")
            .execute(pool).await.unwrap();
        assert!(matches!(
            repo.checkpoint_items(&cp, &outbox, &work, Some(&receipt))
                .await,
            Err(CheckpointError::Constraint(_))
        ));
        assert_eq!(lookup().await.unwrap(), LearnReceiptLookup::Unknown);
        assert_eq!(repo.load_for_admission(c.id).await.unwrap(), before);
        sqlx::query("ALTER TABLE outbox DROP CONSTRAINT fail_unified_learn")
            .execute(pool)
            .await
            .unwrap();
    }
    let (a, b) = tokio::join!(
        repo.checkpoint_items(&cp, &outbox, &work, Some(&receipt)),
        repo.checkpoint_items(&cp, &outbox, &work, Some(&receipt))
    );
    assert!(matches!(
        (a.unwrap(), b.unwrap()),
        (CheckpointOutcome::Applied(2), CheckpointOutcome::Replayed(2))
            | (CheckpointOutcome::Replayed(2), CheckpointOutcome::Applied(2))
    ));
    assert_eq!(lookup().await.unwrap(), LearnReceiptLookup::Known(receipt.result.clone()));
    let committed = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!((committed.revision, committed.class_state.sp), (2, 50));
    assert_eq!(committed.phase4.as_ref().unwrap().checkpoint, cp.phase4);
    assert_eq!(committed.class_state, *cp.class_state.as_ref().unwrap());
    let mut different_key = cp.clone();
    different_key.idempotency.1 = IdempotencyKey::new();
    assert_eq!(
        repo.checkpoint_items(&different_key, &outbox, &work, Some(&receipt))
            .await
            .unwrap(),
        CheckpointOutcome::Stale
    );
    let mut changed = receipt.clone();
    changed.result.skill.rank = 2;
    changed.request_fingerprint = learn_skill_fingerprint(c.id, changed.result.skill);
    assert!(matches!(
        repo.checkpoint_items(&different_key, &outbox, &work, Some(&changed))
            .await,
        Err(CheckpointError::KeyReused)
    ));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), committed);
    // D59/D61: V6 whole HP zero is live only with a positive retained fraction.
    let mut fractional = cp.clone();
    fractional.revision_seen = 2;
    fractional.idempotency.1 = IdempotencyKey::new();
    fractional.hp = 0;
    fractional.phase3.as_mut().unwrap().resource_fractions.hp = 500_000;
    assert_eq!(repo.checkpoint(&fractional, &[]).await.unwrap(), CheckpointOutcome::Applied(3));
    let read = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert!(read.alive);
    assert_eq!(read.hp, Some(0));
    assert_eq!(read.phase3.as_ref().unwrap().resource_fractions.hp, 500_000);
    for alive in [false, true] {
        let mut invalid = fractional.clone();
        invalid.revision_seen = 3;
        invalid.idempotency.1 = IdempotencyKey::new();
        invalid.alive = alive;
        if alive {
            invalid.phase3.as_mut().unwrap().resource_fractions.hp = 0;
        }
        assert!(matches!(
            repo.checkpoint(&invalid, &[]).await,
            Err(CheckpointError::Constraint(_))
        ));
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), read);
    }
}

#[tokio::test]
async fn memory_unified_learning_and_item_work_settle_once() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    unified_learning_contract(repo.clone(), &bundle, None).await;
    assert_eq!(
        repo.staged_events()
            .iter()
            .filter(|event| matches!(event, DomainEvent::CharacterSkillLearned { .. }))
            .count(),
        1
    );
    assert_eq!(
        repo.item_ledger_for_test()
            .iter()
            .filter(|row| row.operation == ItemLedgerOperation::Consume)
            .count(),
        1
    );
}

#[tokio::test]
async fn postgres_unified_learning_and_item_work_roll_back_outbox_failure() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    unified_learning_contract(repo, &bundle, Some(&pool)).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM item_ledger WHERE op='consume'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE subject='nightfall.character.skill_learned'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

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
            creation_origin: Some(nightfall_api::domain::zone::ItemCreationOrigin {
                zone: ZoneId(1),
                epoch: 1,
            }),
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
            exchanges: Vec::new(),
            paused_core: None,
            tick: Tick(73),
            entity: EntityId::from_uuid(character.id.as_uuid()),
            account: character.account_id,
            checkpoint,
            ledger: instances
                .iter()
                .map(|item| ItemDeltaPlan {
                    receipt_refs: Vec::new(),
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
                fingerprint: pending.fingerprint.clone(),
                result: FrozenItemResult {
                    accepted_sequence: None,
                    request_fingerprint: pending.fingerprint.clone(),
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
    // D66: observed n -> n-1 -> n+2 -> n+1 exports grant(n -> n+3) then consume(n+3 -> n+1).
    grant_then_consume(first, first_work, 74, 3, 2)
}
/// One tick on the newest stack: Grant(+gained) then, when `consumed > 0`, the Consume
/// aggregate (-consumed). D76: ordinals are the instance's row positions (grant 0, consume 1).
fn grant_then_consume(
    first: &CharacterCheckpoint,
    first_work: &ItemCheckpointWork,
    tick: u64,
    gained: u64,
    consumed: u64,
) -> (CharacterCheckpoint, ItemCheckpointWork, Vec<DomainEvent>) {
    let mut cp = first.clone();
    cp.revision_seen = first.revision_seen.checked_add(1).unwrap();
    cp.idempotency.1 = IdempotencyKey::new();
    let retained = cp.phase4.as_mut().unwrap();
    retained.inventory_revision = retained.inventory_revision.checked_add(1).unwrap();
    let revision = retained.inventory_revision;
    let target = retained
        .instances
        .iter_mut()
        .max_by_key(|item| (item.revision, item.count))
        .unwrap();
    let before = target.clone();
    let mut granted = before.clone();
    granted.count = granted.count.checked_add(gained).unwrap();
    granted.revision = revision;
    target.count = granted.count.checked_sub(consumed).unwrap();
    target.revision = revision;
    let ended = target.clone();
    let target_id = target.id;
    let request = Uuid::now_v7();
    let cause = |ordinal, count| ItemAuditEvidence {
        family: "item_cast_payment".into(),
        fields: BTreeMap::from([
            ("operation_ordinal".into(), ItemEvidenceValue::Unsigned(ordinal)),
            ("count_delta".into(), ItemEvidenceValue::Signed(count)),
            ("receipt".into(), ItemEvidenceValue::Text(request.to_string())),
        ]),
    };
    let debit = i64::try_from(consumed).unwrap();
    let mut work = first_work.clone();
    work.projection.tick = Tick(tick);
    work.projection.checkpoint = retained.clone();
    work.projection.ledger = vec![ItemDeltaPlan {
        receipt_refs: Vec::new(),
        operation: ItemOperation::Grant,
        operation_ordinal: 0,
        instance: target_id,
        before: Some(before),
        after: Some(granted.clone()),
        count_delta: i64::try_from(gained).unwrap(),
        receipt: Some(ItemReceiptRef {
            operation: ItemOperation::Grant,
            key: request,
        }),
        causes: Vec::new(),
    }];
    if consumed > 0 {
        work.projection.ledger.push(ItemDeltaPlan {
            receipt_refs: Vec::new(),
            operation: ItemOperation::Consume,
            operation_ordinal: 1,
            instance: target_id,
            before: Some(granted),
            after: Some(ended),
            count_delta: debit.checked_neg().unwrap(),
            receipt: Some(ItemReceiptRef {
                operation: ItemOperation::Use,
                key: request,
            }),
            causes: if consumed == 1 {
                vec![cause(0, -1)]
            } else {
                vec![
                    cause(0, -1),
                    cause(2, debit.checked_sub(1).unwrap().checked_neg().unwrap()),
                ]
            },
        });
    }
    work.projection.receipts = vec![ItemReceiptFact {
        request_id: request,
        fingerprint: "grant-body-three".into(),
        result: FrozenItemResult {
            accepted_sequence: None,
            request_fingerprint: "grant-body-three".into(),
            entity: work.projection.entity,
            incarnation: 4,
            tick: Tick(tick),
            inventory_revision: revision,
            receipt: request,
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::Grant,
                affected: vec![target_id],
                recipe: None,
            },
        },
    }];
    if consumed > 0 {
        let mut use_receipt = work.projection.receipts.first().unwrap().clone();
        use_receipt.fingerprint = "use-body-two".into();
        use_receipt
            .result
            .request_fingerprint
            .clone_from(&use_receipt.fingerprint);
        use_receipt.result.detail = ItemResultDetail::Operation {
            operation: ItemOperation::Use,
            affected: vec![target_id],
            recipe: None,
        };
        work.projection.receipts.push(use_receipt);
    }
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
    checkpoint.inventory_revision = 3;
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
        receipt_refs: Vec::new(),
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
            accepted_sequence: None,
            request_fingerprint: "destroy-body".into(),
            entity: work.projection.entity,
            incarnation: 4,
            tick: Tick(75),
            inventory_revision: 3,
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
        2
    );
    assert_eq!(frozen.result.inventory_revision, 2);
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
    conflicting
        .projection
        .receipts
        .first_mut()
        .unwrap()
        .result
        .request_fingerprint = "changed-body".into();
    assert!(matches!(
        repo.checkpoint_items(&duplicate, &second_facts, &conflicting, None)
            .await,
        Err(CheckpointError::KeyReused)
    ));
    let mut toggled = third.clone();
    toggled.revision_seen = 3;
    toggled.idempotency.1 = IdempotencyKey::new();
    let checkpoint = toggled.phase4.as_mut().unwrap();
    checkpoint.inventory_revision = 4;
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
            accepted_sequence: None,
            request_fingerprint: "toggle-with-no-stack".into(),
            entity: toggle_work.projection.entity,
            incarnation: 4,
            tick: Tick(76),
            inventory_revision: 4,
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
    assert_eq!(metadata.inventory_revision, 4);
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
    assert_eq!(consumed[0].operation_ordinal, 1);
    assert_eq!(consumed[0].delta.count_delta, -2);
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
    assert_eq!(rows[0].0, 1);
    let delta: ItemLedgerDelta = serde_json::from_value(rows[0].1.clone()).unwrap();
    assert_eq!(delta.count_delta, -2);
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

#[allow(clippy::too_many_lines)] // Shared atomic refusal matrix for both storage adapters.
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
    let next_before = before.as_ref().and_then(|view| view.checkpoint.clone());
    let mut corrupt = valid.clone();
    corrupt.projection.ledger.first_mut().unwrap().count_delta = -7;
    // The zone domain gate refuses the unconserved row first (count delta mismatch).
    let refused = repo.checkpoint_items(&next, &events, &corrupt, None).await;
    assert!(
        matches!(&refused, Err(CheckpointError::Constraint(name)) if name == "item_checkpoint_projection"),
        "{refused:?}"
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
    // The Consume row's before (creation tick 72) is not the grant's after: continuity.
    let refused = repo.checkpoint_items(&next, &events, &corrupt, None).await;
    assert!(
        matches!(&refused, Err(CheckpointError::Constraint(name)) if name == "item_ledger_scalar_continuity"),
        "{refused:?}"
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
        creation_origin: Some(nightfall_api::domain::zone::ItemCreationOrigin {
            zone: ZoneId(1),
            epoch: 1,
        }),
        id,
        template_id: 57,
        count: 5,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(74),
        revision: transient.projection.checkpoint.inventory_revision,
    };
    let reference = transient.projection.receipts[0].reference();
    transient.projection.ledger.extend([
        ItemDeltaPlan {
            receipt_refs: Vec::new(),
            operation: ItemOperation::Grant,
            operation_ordinal: 0,
            instance: id,
            before: None,
            after: Some(created.clone()),
            count_delta: 5,
            receipt: Some(reference),
            causes: vec![],
        },
        ItemDeltaPlan {
            receipt_refs: Vec::new(),
            operation: ItemOperation::Consume,
            operation_ordinal: 1,
            instance: id,
            before: Some(created),
            after: None,
            count_delta: -6,
            receipt: None,
            causes: vec![ItemAuditEvidence {
                family: "skill_cast_payment".into(),
                fields: BTreeMap::from([
                    ("cast_id".into(), ItemEvidenceValue::Unsigned(1)),
                    ("count_delta".into(), ItemEvidenceValue::Signed(-6)),
                ]),
            }],
        },
    ]);
    // A consumed transient (+5 then -6) does not conserve its row count.
    let refused = repo
        .checkpoint_items(&next, &events, &transient, None)
        .await;
    assert!(
        matches!(&refused, Err(CheckpointError::Constraint(name)) if name == "item_ledger_count_or_revision"),
        "{refused:?}"
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    // (settlement result on both adapters, API settlement validator result where it differs):
    // the zone domain gate (ItemDeltaPlan/Phase4Projection::validate_structure) runs first and
    // refuses non-positional ordinals as item_checkpoint_projection; the API validator behind it
    // refuses the same shapes as item_ledger_row_ordinal (D76 defence in depth).
    for (mode, (expected, validator)) in [
        ("item_ledger_row_order", None), // pre-D66 Consume-first export
        ("item_ledger_scalar_continuity", None), // overlapping row endpoints
        ("item_checkpoint_projection", Some("item_ledger_row_ordinal")), // D76 duplicate ordinal
        ("item_ledger_count_or_revision", None), // fabricated intermediate revision step
        ("item_checkpoint_revision_step", None), // stale receipt revision
        ("item_consume_cause_order", None), // reversed causes
        ("item_consume_contribution", None), // fabricated debit
        ("item_instance_creation_immutable", None), // altered provenance
        ("item_ledger_row_order", None), // any row after the Consume aggregate
        ("item_ledger_row_order", None), // positive row after a non-Consume debit
        ("item_checkpoint_projection", None), // split Consume aggregate (domain batch key)
        ("item_checkpoint_projection", Some("item_ledger_row_ordinal")), // D76 ordinal gap
        ("item_checkpoint_projection", Some("item_ledger_row_ordinal")), // D76 Consume at 0
    ]
    .into_iter()
    .enumerate()
    {
        let mut bad = valid.clone();
        let mut bad_cp = next.clone();
        let target = bad.projection.ledger[0].instance;
        let start = bad.projection.ledger[0].before.clone().unwrap();
        let end = bad.projection.ledger[1].after.clone().unwrap();
        match mode {
            0 => {
                // The withdrawn D64 shape: consume(n -> n-2) then grant(n-2 -> n+1).
                let mut low = end.clone();
                low.count = start.count.checked_sub(2).unwrap();
                bad.projection.ledger.reverse();
                bad.projection.ledger[0].operation_ordinal = 0;
                bad.projection.ledger[1].operation_ordinal = 1;
                bad.projection.ledger[0].before = Some(start.clone());
                bad.projection.ledger[0].after = Some(low.clone());
                bad.projection.ledger[1].before = Some(low);
                bad.projection.ledger[1].after = Some(end);
            },
            1 => {
                // Each row conserves its own count, but grant(n -> n+4) overlaps consume(n+3 ->).
                let grant = &mut bad.projection.ledger[0];
                let after = grant.after.as_mut().unwrap();
                after.count = after.count.checked_add(1).unwrap();
                grant.count_delta = grant.count_delta.checked_add(1).unwrap();
            },
            2 => {
                // The e6d37ec shape: two rows of the instance share ordinal 0, grant(+1) and
                // grant(+2), then the Consume aggregate at its position 2.
                let mut middle = start.clone();
                middle.count = start.count.checked_add(1).unwrap();
                middle.revision = end.revision;
                let mut second = bad.projection.ledger[0].clone();
                let first = &mut bad.projection.ledger[0];
                first.after = Some(middle.clone());
                first.count_delta = 1;
                second.before = Some(middle);
                second.count_delta = 2;
                bad.projection.ledger.insert(1, second);
                bad.projection.ledger[2].operation_ordinal = 2;
            },
            3 => {
                // Fabricated intermediate steps inside one tick (pre-D55): grant r, consume r+1.
                bad.projection.checkpoint.inventory_revision = 4;
                bad.projection
                    .checkpoint
                    .instances
                    .iter_mut()
                    .find(|item| item.id == target)
                    .unwrap()
                    .revision = 4;
                bad.projection.ledger[1].after.as_mut().unwrap().revision = 4;
                for receipt in &mut bad.projection.receipts {
                    receipt.result.inventory_revision = 4;
                }
                bad_cp.phase4 = Some(bad.projection.checkpoint.clone());
            },
            4 => bad.projection.receipts[0].result.inventory_revision = 1,
            5 => bad.projection.ledger[1].causes.reverse(),
            6 => bad.projection.ledger[1].causes[0]
                .fields
                .insert("count_delta".into(), ItemEvidenceValue::Signed(-2))
                .map_or((), |_| ()),
            7 => {
                let changed = bad.projection.ledger[1].after.as_mut().unwrap();
                changed.creation_origin.as_mut().unwrap().epoch = 2;
                let id = changed.id;
                bad.projection
                    .checkpoint
                    .instances
                    .iter_mut()
                    .find(|item| item.id == id)
                    .unwrap()
                    .creation_origin = changed.creation_origin;
                bad_cp.phase4 = Some(bad.projection.checkpoint.clone());
            },
            8 => {
                bad.projection.ledger.push(ItemDeltaPlan {
                    operation: ItemOperation::Destroy,
                    operation_ordinal: 2,
                    instance: target,
                    count_delta: i64::try_from(end.count).unwrap().checked_neg().unwrap(),
                    before: Some(end),
                    after: None,
                    receipt: bad.projection.ledger[0].receipt,
                    receipt_refs: vec![],
                    causes: vec![],
                });
                bad.projection
                    .checkpoint
                    .instances
                    .retain(|item| item.id != target);
                bad_cp.phase4 = Some(bad.projection.checkpoint.clone());
            },
            9 => {
                // Destroy(n -> n-2) then grant(n-2 -> n+1): debits may not precede credits.
                let mut low = end.clone();
                low.count = start.count.checked_sub(2).unwrap();
                bad.projection.ledger = vec![
                    ItemDeltaPlan {
                        operation: ItemOperation::Destroy,
                        operation_ordinal: 0,
                        instance: target,
                        count_delta: -2,
                        before: Some(start),
                        after: Some(low.clone()),
                        receipt: bad.projection.ledger[1].receipt,
                        receipt_refs: vec![],
                        causes: vec![],
                    },
                    ItemDeltaPlan {
                        before: Some(low),
                        after: Some(end),
                        operation_ordinal: 1,
                        ..bad.projection.ledger[0].clone()
                    },
                ];
            },
            10 => {
                // One consume split into two aggregates (n+3 -> n+2 -> n+1).
                let mut middle = end.clone();
                middle.count = end.count.checked_add(1).unwrap();
                let mut second = bad.projection.ledger[1].clone();
                let first = &mut bad.projection.ledger[1];
                first.after = Some(middle.clone());
                first.count_delta = -1;
                first.causes.truncate(1);
                second.before = Some(middle);
                second.count_delta = -1;
                second.causes.remove(0);
                second.operation_ordinal = 2;
                bad.projection.ledger.push(second);
            },
            11 => bad.projection.ledger[1].operation_ordinal = 2,
            _ => bad.projection.ledger[1].operation_ordinal = 0,
        }
        if let Some(validator) = validator {
            let result = bad.validate_before(next_before.as_ref());
            assert!(
                matches!(&result, Err(CheckpointError::Constraint(name)) if name == validator),
                "API validator case {mode}: {result:?}"
            );
        }
        let outbox = self::events(&bad_cp, &bad);
        let result = repo.checkpoint_items(&bad_cp, &outbox, &bad, None).await;
        assert!(
            matches!(&result, Err(CheckpointError::Constraint(name)) if name == expected),
            "negative canonical case {mode}: {result:?}"
        );
        assert_eq!(repo.retained_inventory(character.id).await.unwrap(), before);
    }
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

fn pinned_cap(item: &ItemInstance) -> u64 {
    if item.template_id == 57 {
        99_900_000_000
    } else {
        i32::MAX.unsigned_abs().into()
    }
}

/// D74: caps hold on tick endpoints and, per positive row, after - later same-tick consumption.
/// Returns the character after the legal C -> C+1 -> C tick settled at tick 75.
#[allow(clippy::too_many_lines)] // One shared memory/Pg cap-edge contract.
async fn cap_edge_contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) -> Character {
    let (character, cp, work, facts) = fixture(repo, bundle, "ItemCapEdge").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let stack = cp
        .phase4
        .as_ref()
        .unwrap()
        .instances
        .iter()
        .max_by_key(|item| (item.revision, item.count))
        .unwrap()
        .clone();
    let cap = pinned_cap(&stack);
    // Tick 74 reaches the cap through its own legal C+2 -> C intermediate.
    let (at_cap, at_cap_work, at_cap_facts) = grant_then_consume(
        &cp,
        &work,
        74,
        cap.checked_add(2)
            .unwrap()
            .checked_sub(stack.count)
            .unwrap(),
        2,
    );
    at_cap_work.validate_catalogue(&bundle.items).unwrap();
    assert_eq!(
        repo.checkpoint_items(&at_cap, &at_cap_facts, &at_cap_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    // (a) start C, grant +1 (C -> C+1), consume -1 (C+1 -> C): legal, settles.
    let (edge, edge_work, edge_facts) = grant_then_consume(&at_cap, &at_cap_work, 75, 1, 1);
    let rows = &edge_work.projection.ledger;
    assert_eq!(
        rows.iter()
            .map(|row| (
                row.operation,
                row.operation_ordinal,
                row.before.as_ref().unwrap().count,
                row.after.as_ref().unwrap().count
            ))
            .collect::<Vec<_>>(),
        vec![
            (ItemOperation::Grant, 0, cap, cap.checked_add(1).unwrap()),
            (ItemOperation::Consume, 1, cap.checked_add(1).unwrap(), cap),
        ]
    );
    edge_work.validate_catalogue(&bundle.items).unwrap();
    assert_eq!(
        repo.checkpoint_items(&edge, &edge_facts, &edge_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(3)
    );
    let settled = repo.retained_inventory(character.id).await.unwrap();
    // (b) tick end C+1 is refused.
    let (over, over_work, over_facts) = grant_then_consume(&edge, &edge_work, 76, 1, 0);
    let refused = over_work.validate_catalogue(&bundle.items);
    assert!(
        matches!(&refused, Err(CheckpointError::Constraint(name)) if name == "item_count_cap"),
        "{refused:?}"
    );
    let result = repo
        .checkpoint_items(&over, &over_facts, &over_work, None)
        .await;
    assert!(
        matches!(&result, Err(CheckpointError::Constraint(name)) if name == "item_count_cap"),
        "tick-end above cap: {result:?}"
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), settled);
    // (c) grant C -> C+2, Destroy -1 (C+2 -> C+1), consume -1 (C+1 -> C): the tick ends at C,
    // but the grant's after (C+2) minus its later same-tick consumption (1) is C+1 > C.
    let (over, mut over_work, _) = grant_then_consume(&edge, &edge_work, 76, 2, 2);
    let peak = over_work.projection.ledger[0].after.clone().unwrap();
    assert_eq!(peak.count, cap.checked_add(2).unwrap());
    let mut middle = peak.clone();
    middle.count = cap.checked_add(1).unwrap();
    let consume = &mut over_work.projection.ledger[1];
    consume.before = Some(middle.clone());
    consume.count_delta = -1;
    consume.causes.truncate(1);
    consume.operation_ordinal = 2;
    over_work.projection.ledger.insert(
        1,
        ItemDeltaPlan {
            receipt_refs: Vec::new(),
            operation: ItemOperation::Destroy,
            operation_ordinal: 1,
            instance: peak.id,
            before: Some(peak),
            after: Some(middle),
            count_delta: -1,
            receipt: None,
            causes: Vec::new(),
        },
    );
    let refused = over_work.validate_catalogue(&bundle.items);
    assert!(
        matches!(&refused, Err(CheckpointError::Constraint(name)) if name == "item_count_cap"),
        "{refused:?}"
    );
    let over_facts = events(&over, &over_work);
    let result = repo
        .checkpoint_items(&over, &over_facts, &over_work, None)
        .await;
    assert!(
        matches!(&result, Err(CheckpointError::Constraint(name)) if name == "item_count_cap"),
        "positive row above cap after later consumption: {result:?}"
    );
    assert_eq!(repo.retained_inventory(character.id).await.unwrap(), settled);
    character
}

#[tokio::test]
async fn memory_cap_edge_settles_c_to_c_plus_1_to_c_and_refuses_tick_end_or_row_above_cap() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    cap_edge_contract(&repo, &bundle).await;
    let rows: Vec<_> = repo
        .item_ledger_for_test()
        .into_iter()
        .filter(|row| row.tick == Tick(75))
        .map(|row| (row.operation, row.operation_ordinal, row.delta.count_delta))
        .collect();
    assert_eq!(
        rows,
        vec![
            (ItemLedgerOperation::Grant, 0, 1),
            (ItemLedgerOperation::Consume, 1, -1)
        ]
    );
}

#[tokio::test]
async fn postgres_cap_edge_settles_c_to_c_plus_1_to_c_and_refuses_tick_end_or_row_above_cap() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let character = cap_edge_contract(&repo, &bundle).await;
    let rows: Vec<(String, i32, serde_json::Value)> = sqlx::query_as(
        "SELECT op,operation_ordinal,delta FROM item_ledger WHERE character_id=$1 AND tick=75 ORDER BY id",
    )
    .bind(character.id.as_uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    let rows: Vec<_> = rows
        .into_iter()
        .map(|(op, ordinal, delta)| {
            let delta: ItemLedgerDelta = serde_json::from_value(delta).unwrap();
            (op, ordinal, delta.before.unwrap().count, delta.after.unwrap().count)
        })
        .collect();
    let cap = rows[0].2;
    assert_eq!(
        rows,
        vec![
            ("grant".to_owned(), 0, cap, cap.checked_add(1).unwrap()),
            ("consume".to_owned(), 1, cap.checked_add(1).unwrap(), cap),
        ]
    );
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
    receipt
        .result
        .request_fingerprint
        .clone_from(&receipt.fingerprint);
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

#[allow(clippy::too_many_lines)] // Both adapters share exact archived cursor and repeat-reservation behavior.
async fn range_extension_contract<
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
>(
    repo: &R,
    bundle: &Phase4RuleBundle,
) {
    let (_, mut cp, mut work, facts) = fixture(repo, bundle, "ArchivedRange").await;
    let (start, _, end) = work.allocation.bounds();
    assert!(matches!(repo.reserve_item_range_extension(ZoneId(1), 1, start).await,
        Err(CheckpointError::Constraint(name)) if name == "item_id_ranges_not_exhausted"));
    assert!(matches!(repo.reserve_item_range_extension(ZoneId(1), 2, start).await,
        Err(CheckpointError::Constraint(name)) if name == "item_id_ranges_missing"));
    // The acknowledged zone cursor includes other owners; no fabricated per-ID ledger rows.
    work.allocation = ItemInstanceRange::new(start, end, end).unwrap();
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    assert_eq!(
        repo.reserve_item_range(ZoneId(1), 1)
            .await
            .unwrap()
            .next_reserved,
        end
    );
    let (left, right) = tokio::join!(
        repo.reserve_item_range_extension(ZoneId(1), 1, start),
        repo.reserve_item_range_extension(ZoneId(1), 1, start)
    );
    let successor = left.unwrap();
    assert_eq!(successor, right.unwrap());
    assert!(successor.range_start >= end);
    assert_eq!(successor.next_reserved, successor.range_start);
    let other = repo.reserve_item_range(ZoneId(2), 1).await.unwrap();
    assert!(other.range_start >= successor.range_end_exclusive);
    assert!(
        matches!(repo.reserve_item_range_extension(ZoneId(1), 1, successor.range_start).await,
        Err(CheckpointError::Constraint(name)) if name == "item_id_ranges_not_exhausted")
    );
    cp.revision_seen = 1;
    cp.idempotency.1 = IdempotencyKey::new();
    let checkpoint = cp.phase4.as_mut().unwrap();
    checkpoint.inventory_revision = 2;
    checkpoint.auto_shots.insert(1463);
    work.projection.tick = Tick(74);
    work.projection.checkpoint = checkpoint.clone();
    work.projection.ledger.clear();
    work.projection.materializations.clear();
    let key = Uuid::now_v7();
    work.projection.receipts = vec![ItemReceiptFact {
        request_id: key,
        fingerprint: "c".repeat(64),
        result: FrozenItemResult {
            entity: work.projection.entity,
            incarnation: 4,
            tick: Tick(74),
            inventory_revision: 2,
            receipt: key,
            accepted_sequence: None,
            request_fingerprint: "c".repeat(64),
            detail: ItemResultDetail::Operation {
                operation: ItemOperation::ToggleAutoShot,
                affected: vec![],
                recipe: None,
            },
        },
    }];
    work.allocation = ItemInstanceRange::new(
        successor.range_start,
        successor.range_end_exclusive,
        successor.range_end_exclusive,
    )
    .unwrap();
    assert_eq!(
        repo.checkpoint_items(&cp, &[], &work, None).await.unwrap(),
        CheckpointOutcome::Applied(2)
    );
    let third = repo
        .reserve_item_range_extension(ZoneId(1), 1, successor.range_start)
        .await
        .unwrap();
    assert!(third.range_start >= other.range_end_exclusive);
    assert_eq!(
        repo.reserve_item_range_extension(ZoneId(1), 1, start)
            .await
            .unwrap(),
        successor
    );
    assert_eq!(repo.reserve_item_range(ZoneId(1), 1).await.unwrap(), third);
    assert_eq!(
        repo.retained_inventory(cp.character_id)
            .await
            .unwrap()
            .unwrap()
            .character_revision,
        2
    );
}

#[tokio::test]
async fn memory_range_extensions_preserve_archived_blocks_and_frozen_repeat_identity() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    range_extension_contract(&repo, &bundle).await;
}

#[tokio::test]
async fn postgres_range_extensions_preserve_archived_blocks_and_frozen_repeat_identity() {
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    range_extension_contract(&repo, &bundle).await;
    let rows: Vec<(i64, i64)> = sqlx::query_as("SELECT next_reserved,range_end_exclusive FROM item_id_ranges WHERE zone_id=1 AND epoch=1 ORDER BY range_start").fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, rows[0].1);
    assert_eq!(rows[1].0, rows[1].1);
    assert!(rows[2].0 < rows[2].1);
    // A legal final archived endpoint prevents any overflow reservation, atomically.
    let top_start = i64::MAX.checked_sub(1_i64 << 32).unwrap();
    sqlx::query("INSERT INTO item_id_ranges(zone_id,epoch,range_start,range_end_exclusive,next_reserved) VALUES(3,1,$1,$2,$2)")
        .bind(top_start).bind(i64::MAX).execute(&pool).await.unwrap();
    assert!(
        matches!(repo.reserve_item_range_extension(ZoneId(3), 1, u64::try_from(top_start).unwrap()).await,
        Err(CheckpointError::Constraint(name)) if name == "item_id_ranges_exhausted")
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM item_id_ranges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 5);
}

// Storage-boundary regression; domain fold/execution of these plans is owned by Z.
#[allow(clippy::too_many_lines)]
async fn new_epoch_canonical_contract<R>(repo: &R, bundle: &Phase4RuleBundle)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository,
{
    use nightfall_api::domain::zone::EquipSlot;
    let (character, first, first_work, first_events) =
        fixture(repo, bundle, "EpochCanonical").await;
    assert_eq!(
        repo.checkpoint_items(&first, &first_events, &first_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let mut cp = first.clone();
    cp.revision_seen = 1;
    cp.idempotency.1 = IdempotencyKey::new();
    let checkpoint = cp.phase4.as_mut().unwrap();
    checkpoint.inventory_revision = 2;
    let stack = checkpoint
        .instances
        .iter_mut()
        .max_by_key(|item| item.count)
        .unwrap();
    let before = stack.clone();
    let debit = i64::try_from(stack.count.checked_sub(3).unwrap())
        .unwrap()
        .checked_neg()
        .unwrap();
    stack.count = 3;
    stack.revision = 2;
    let after = stack.clone();
    let gear = checkpoint
        .instances
        .iter_mut()
        .find(|item| item.count == 1)
        .unwrap();
    let unequipped = gear.clone();
    gear.location = ItemLocation::Paperdoll;
    gear.slot = Some(EquipSlot::Rhand);
    gear.revision = 2;
    let equipped = gear.clone();
    checkpoint.paperdoll.insert(EquipSlot::Rhand, gear.id);
    let mut work = first_work.clone();
    work.epoch = 2;
    let reserved = repo
        .reserve_item_range(work.zone, work.epoch)
        .await
        .unwrap();
    work.allocation = ItemInstanceRange::new(
        reserved.range_start,
        reserved.next_reserved,
        reserved.range_end_exclusive,
    )
    .unwrap();
    work.projection.tick = Tick(0);
    work.projection.checkpoint = checkpoint.clone();
    work.projection.materializations.clear();
    work.projection.receipts.clear();
    work.projection.ledger = vec![
        ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: before.id,
            before: Some(before),
            after: Some(after),
            count_delta: debit,
            receipt: None,
            receipt_refs: vec![],
            causes: vec![ItemAuditEvidence {
                family: "skill_cast_payment".into(),
                fields: BTreeMap::from([
                    ("cast_id".into(), ItemEvidenceValue::Unsigned(1)),
                    ("count_delta".into(), ItemEvidenceValue::Signed(debit)),
                ]),
            }],
        },
        ItemDeltaPlan {
            operation: ItemOperation::Equip,
            operation_ordinal: 0,
            instance: equipped.id,
            before: Some(unequipped),
            after: Some(equipped),
            count_delta: 0,
            receipt: None,
            receipt_refs: vec![],
            causes: vec![],
        },
    ];
    let outbox = events(&cp, &work);
    assert_eq!(
        repo.checkpoint_items(&cp, &outbox, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    let (next, mut next_work, _) = interleaved(&cp, &work);
    next_work.projection.tick = Tick(1);
    for receipt in &mut next_work.projection.receipts {
        receipt.result.tick = Tick(1);
    }
    // D66: observed 3 -> 2 -> 5 -> 4 exports grant(3 -> 6, +3) then consume(6 -> 4, -2).
    let grant = &next_work.projection.ledger[0];
    let consume = &next_work.projection.ledger[1];
    assert_eq!(
        (
            grant.operation,
            grant.before.as_ref().unwrap().count,
            grant.after.as_ref().unwrap().count,
            grant.count_delta
        ),
        (ItemOperation::Grant, 3, 6, 3)
    );
    assert_eq!(
        (
            consume.operation,
            consume.before.as_ref().unwrap().count,
            consume.after.as_ref().unwrap().count,
            consume.count_delta
        ),
        (ItemOperation::Consume, 6, 4, -2)
    );
    assert_eq!(grant.after, consume.before);
    let outbox = events(&next, &next_work);
    assert_eq!(
        repo.checkpoint_items(&next, &outbox, &next_work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(3)
    );
    let retained = repo
        .retained_inventory(character.id)
        .await
        .unwrap()
        .unwrap()
        .checkpoint
        .unwrap();
    assert_eq!(retained.inventory_revision, 3);
    for original in &first_work.projection.checkpoint.instances {
        let current = retained
            .instances
            .iter()
            .find(|item| item.id == original.id)
            .unwrap();
        assert_eq!(current.created_tick, Tick(73));
        assert_eq!(current.created_tick, original.created_tick);
        assert_eq!(current.creation_origin, original.creation_origin);
    }
    let mut fabricated = next_work.clone();
    let id = fabricated.allocation.allocate().unwrap();
    let mut item = fabricated.projection.checkpoint.instances[0].clone();
    item.id = id;
    item.created_tick = Tick(2);
    item.creation_origin = Some(nightfall_api::domain::zone::ItemCreationOrigin {
        zone: fabricated.zone,
        epoch: fabricated.epoch,
    });
    fabricated.projection.ledger.push(ItemDeltaPlan {
        operation: ItemOperation::Grant,
        operation_ordinal: 0,
        instance: id,
        before: None,
        after: Some(item),
        count_delta: 1,
        receipt: None,
        receipt_refs: vec![],
        causes: vec![],
    });
    assert!(fabricated.validate_catalogue(&bundle.items).is_err());
    fabricated
        .projection
        .ledger
        .last_mut()
        .unwrap()
        .after
        .as_mut()
        .unwrap()
        .created_tick = Tick(1);
    fabricated
        .projection
        .ledger
        .last_mut()
        .unwrap()
        .after
        .as_mut()
        .unwrap()
        .creation_origin
        .as_mut()
        .unwrap()
        .epoch = 1;
    assert!(fabricated.validate_catalogue(&bundle.items).is_err());
}

#[tokio::test]
async fn memory_new_epoch_preserves_creation_and_settles_3_to_2_to_5_to_4_canonically() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    new_epoch_canonical_contract(&repo, &bundle).await;
    let rows = repo.item_ledger_for_test();
    let rows: Vec<_> = rows
        .iter()
        .filter(|row| row.epoch == 2 && row.tick == Tick(1))
        .collect();
    assert_eq!(rows.len(), 2);
    // D76: row positions, grant 0 then the Consume aggregate 1.
    assert_eq!((rows[0].operation, rows[0].operation_ordinal), (ItemLedgerOperation::Grant, 0));
    assert_eq!(
        (rows[1].operation, rows[1].operation_ordinal),
        (ItemLedgerOperation::Consume, 1)
    );
    assert_eq!((rows[0].delta.count_delta, rows[1].delta.count_delta), (3, -2));
    assert_eq!(rows[0].delta.after, rows[1].delta.before);
    assert_eq!(rows[1].delta.after.as_ref().unwrap().count, 4);
    assert_eq!((rows[0].revision, rows[1].revision), (3, 3));
    let character = rows[0].character_id;
    let expected = repo.retained_inventory(character).await.unwrap().unwrap();
    let mut historical = expected.checkpoint.clone().unwrap();
    for item in &mut historical.instances {
        item.creation_origin = None;
    }
    repo.insert_item_checkpoint_for_test(character, historical)
        .unwrap();
    assert_eq!(repo.retained_inventory(character).await.unwrap().unwrap(), expected);
    let loaded = repo.load_for_admission(character).await.unwrap().unwrap();
    assert_eq!(loaded.phase4.unwrap().checkpoint, expected.checkpoint);
    let mut rewritten = expected.checkpoint.unwrap();
    rewritten.instances[0].created_tick = Tick(74);
    repo.insert_item_checkpoint_for_test(character, rewritten)
        .unwrap();
    assert_constraint(
        &repo.retained_inventory(character).await.unwrap_err(),
        "item_creation_ledger_tick",
    );
}

#[tokio::test]
async fn postgres_new_epoch_preserves_creation_and_settles_3_to_2_to_5_to_4_canonically() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    new_epoch_canonical_contract(&repo, &bundle).await;
    let rows: Vec<(String, i32, i64, serde_json::Value)> = sqlx::query_as(
        "SELECT op,operation_ordinal,revision,delta FROM item_ledger WHERE epoch=2 AND tick=1 ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 2);
    // Append order (identity id) is the canonical D66 order; D76 ordinals are row positions.
    assert_eq!((&*rows[0].0, rows[0].1, rows[0].2), ("grant", 0, 3));
    assert_eq!((&*rows[1].0, rows[1].1, rows[1].2), ("consume", 1, 3));
    let grant: ItemLedgerDelta = serde_json::from_value(rows[0].3.clone()).unwrap();
    let consume: ItemLedgerDelta = serde_json::from_value(rows[1].3.clone()).unwrap();
    assert_eq!((grant.count_delta, consume.count_delta), (3, -2));
    assert_eq!(grant.after, consume.before);
    assert_eq!(consume.after.as_ref().unwrap().count, 4);
    let character: Uuid = sqlx::query_scalar("SELECT character_id FROM item_ledger LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let character = CharacterId::from_uuid(character);
    let expected = repo.retained_inventory(character).await.unwrap().unwrap();
    // Missing additive fields resolve to the creation row, not the latest epoch2/tick1.
    sqlx::query("UPDATE characters SET phase4_checkpoint=jsonb_set(phase4_checkpoint,'{instances}',(SELECT jsonb_agg(item - 'creation_origin') FROM jsonb_array_elements(phase4_checkpoint->'instances') item)) WHERE id=$1")
        .bind(character.as_uuid()).execute(&pool).await.unwrap();
    assert_eq!(repo.retained_inventory(character).await.unwrap().unwrap(), expected);
    assert_eq!(
        repo.load_for_admission(character)
            .await
            .unwrap()
            .unwrap()
            .phase4
            .unwrap()
            .checkpoint,
        expected.checkpoint
    );
    sqlx::query("UPDATE characters SET phase4_checkpoint=jsonb_set(phase4_checkpoint,'{instances}',(SELECT jsonb_agg(item || '{\"creation_origin\":{\"zone\":1,\"epoch\":2}}'::jsonb) FROM jsonb_array_elements(phase4_checkpoint->'instances') item)) WHERE id=$1")
        .bind(character.as_uuid()).execute(&pool).await.unwrap();
    assert_constraint(
        &repo.retained_inventory(character).await.unwrap_err(),
        "item_creation_ledger_origin",
    );
}

/// D77: a retained item without its birth ledger row is a permanent named constraint on the
/// settlement and read paths; nothing is written and the call returns promptly (no retry).
async fn missing_birth_contract<R>(
    repo: &R,
    character: &Character,
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
) where
    R: CharacterRepository + ItemCheckpointRepository + ItemInventoryRepository,
{
    assert_constraint(
        &repo.retained_inventory(character.id).await.unwrap_err(),
        "item_creation_ledger_missing",
    );
    assert_constraint(
        &repo.load_for_admission(character.id).await.unwrap_err(),
        "item_creation_ledger_missing",
    );
    // A single Grant row: refused on provenance before any D76 multi-row question arises.
    let (next, next_work, next_facts) = grant_then_consume(cp, work, 74, 3, 0);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        repo.checkpoint_items(&next, &next_facts, &next_work, None),
    )
    .await
    .unwrap();
    assert!(
        matches!(&result, Err(CheckpointError::Constraint(name)) if name == "item_creation_ledger_missing"),
        "{result:?}"
    );
}

#[tokio::test]
async fn memory_missing_birth_ledger_row_is_a_permanent_named_constraint() {
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, cp, work, facts) = fixture(&repo, &bundle, "ItemNoBirth").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let mut corrupt = cp.phase4.clone().unwrap();
    let mut orphan = corrupt.instances[0].clone();
    orphan.id = work.allocation.clone().allocate().unwrap();
    corrupt.instances.push(orphan);
    repo.insert_item_checkpoint_for_test(character.id, corrupt)
        .unwrap();
    let rows = repo.item_ledger_for_test();
    missing_birth_contract(&repo, &character, &cp, &work).await;
    assert_eq!(repo.item_ledger_for_test(), rows);
    let header = repo.get(character.id).await.unwrap().unwrap();
    assert_eq!(header.item_metadata.unwrap().inventory_revision, 1);
}

#[tokio::test]
async fn postgres_missing_birth_ledger_row_is_a_permanent_named_constraint() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, cp, work, facts) = fixture(&repo, &bundle, "ItemNoBirth").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let source = i64::try_from(cp.phase4.as_ref().unwrap().instances[0].id.get()).unwrap();
    let orphan = i64::try_from(work.allocation.clone().allocate().unwrap().get()).unwrap();
    // A retained row and checkpoint entry whose append-only birth row was never written.
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,bound,created_tick,revision) SELECT $2,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,bound,created_tick,revision FROM item_instances WHERE id=$1")
        .bind(source).bind(orphan).execute(&pool).await.unwrap();
    sqlx::query("UPDATE characters SET phase4_checkpoint=jsonb_set(phase4_checkpoint,'{instances}',(phase4_checkpoint->'instances') || jsonb_build_array((SELECT item || jsonb_build_object('id',$2::bigint) FROM jsonb_array_elements(phase4_checkpoint->'instances') item WHERE (item->>'id')::bigint=$1))) WHERE id=$3")
        .bind(source).bind(orphan).bind(character.id.as_uuid()).execute(&pool).await.unwrap();
    let counts =
        "SELECT (SELECT count(*) FROM item_ledger)+(SELECT count(*) FROM item_mutation_receipts)";
    let rows: i64 = sqlx::query_scalar(counts).fetch_one(&pool).await.unwrap();
    missing_birth_contract(&repo, &character, &cp, &work).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(counts)
            .fetch_one(&pool)
            .await
            .unwrap(),
        rows
    );
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM characters WHERE id=$1")
        .bind(character.id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(revision, 1);
}

/// D76 at the storage boundary: the deferred trigger refuses duplicate, gapped and
/// Consume-not-last ordinals with its exact name, and accepts the canonical positions.
#[tokio::test]
async fn postgres_item_ledger_row_ordinal_trigger_enforces_row_positions() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, Uuid::from_u128(1)).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, cp, work, facts) = fixture(&repo, &bundle, "ItemRowOrdinal").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &facts, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let row = "INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES($1,9,1,500,2,$2,7,$3,'{}'::jsonb)";
    for (rows, refused) in [
        (vec![("grant", 0), ("consume", 1)], false),
        (vec![("unequip", 0), ("destroy", 0)], true), // duplicate (the e6d37ec shape)
        (vec![("equip", 0), ("unequip", 2)], true),   // gap
        (vec![("grant", 1), ("consume", 0)], true),   // Consume at 0 after a positive row
        (vec![("equip", 1)], true),                   // does not start at 0
    ] {
        let mut tx = pool.begin().await.unwrap();
        for (op, ordinal) in &rows {
            sqlx::query(row)
                .bind(character.id.as_uuid())
                .bind(*op)
                .bind(*ordinal)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        let result = sqlx::query("SET CONSTRAINTS item_ledger_row_ordinal IMMEDIATE")
            .execute(&mut *tx)
            .await;
        if refused {
            let error = result.unwrap_err();
            let database = error.as_database_error().unwrap();
            assert_eq!(
                (database.code().as_deref(), database.constraint()),
                (Some("23514"), Some("item_ledger_row_ordinal")),
                "{rows:?}"
            );
            tx.rollback().await.unwrap();
        } else {
            result.unwrap();
            tx.rollback().await.unwrap();
        }
    }
    // Deferred to COMMIT when nothing sets it immediate.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(row)
        .bind(character.id.as_uuid())
        .bind("equip")
        .bind(1)
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(error.as_database_error().unwrap().constraint(), Some("item_ledger_row_ordinal"));
}
