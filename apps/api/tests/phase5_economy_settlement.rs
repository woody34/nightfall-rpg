//! P5-Q13 (design §11.2, §11.5): a V7 economy change to a character inventory settles through
//! the player's V6 item projection (scalars) plus the tick's economy rows (every owner's ledger,
//! the character side included) and the frozen `EconomyResult` receipt, in one revision-fenced
//! transaction (`PgCharacterRepository::checkpoint_with_economy`). Covers: economy rows as the
//! V6 coverage of changed scalars, the shared T1 positions with V6 rows, a foreign character row,
//! receipt persistence with replay and conflict, and the creation origin taken from the
//! instance's first ledger row across owners (the ground drop, not the `transfer_in`). The
//! Postgres half runs only with `DATABASE_URL`.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used,
    clippy::too_many_lines,
    clippy::arithmetic_side_effects,
    clippy::too_many_arguments,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::many_single_char_names
)]
mod common;
use nightfall_api::application::economy::{
    ContainerDelta, DeltaChange, EconomyCharacterCheckpointRepository, EconomyCheckpointRepository,
    EconomyCheckpointWrite, EconomyReceipt, LeaseGrant, LeaseKey, LeaseKind, LeaseRepository,
    OwnerLedgerRow,
};
use nightfall_api::application::economy_reads::EconomyReadRepository;
use nightfall_api::application::ports::*;
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::zone::phase4_checkpoint::StarterKitReceipt;
use nightfall_api::domain::zone::phase4_projection::*;
use nightfall_api::domain::zone::{
    EntityId, ItemCreationOrigin, ItemInstance, ItemInstanceId, ItemInstanceRange, ItemLocation,
    Phase3Checkpoint, Phase4Checkpoint, Phase4Projection, Tick, ZoneId,
};
use nightfall_api::domain::{
    AccountId, Character, CharacterName, DomainEvent, EventMetadata, ItemEventFact, Position, Race,
};
use nightfall_api::infrastructure::item_data::{embedded_phase4_bundle, Phase4RuleBundle};
use nightfall_api::infrastructure::memory::{InMemoryCharacterRepository, InMemoryEconomyStore};
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgEconomyRepository};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use uuid::Uuid;

const ACCOUNT: Uuid = Uuid::from_u128(1);
const GROUND: Uuid = Uuid::from_u128(0x6000);
const PICKED: u64 = 9_000_001;
const PICKUP_LINK: &str = "6f1c3c1e-7c55-5d7e-9e1a-000000000001";

/// The starter-kit tick (revision 1), as the Phase 4 settlement fixture builds it.
async fn starter<R: CharacterRepository + ItemAdmissionRepository>(
    repo: &R,
    bundle: &Phase4RuleBundle,
    name: &str,
) -> (Character, CharacterCheckpoint, ItemCheckpointWork, Vec<DomainEvent>) {
    let character = Character::create(
        AccountId::from_uuid(ACCOUNT),
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
    let instances: Vec<_> = kit
        .items
        .iter()
        .map(|row| ItemInstance {
            creation_origin: Some(ItemCreationOrigin {
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
        phase5: None,
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
    let receipt = ItemReceiptRef {
        operation: ItemOperation::Starter,
        key: pending.request_id,
    };
    let work = ItemCheckpointWork {
        zone: ZoneId(1),
        epoch: 1,
        allocation,
        projection: Phase4Projection {
            enchant_attempts: Vec::new(),
            exchanges: Vec::new(),
            refines: Vec::new(),
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
                    receipt: Some(receipt),
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
                receipt,
            }],
            evidence: Vec::new(),
        },
    };
    let events = work
        .projection
        .ledger
        .iter()
        .enumerate()
        .map(|(ordinal, delta)| DomainEvent::ItemGranted {
            fact: ItemEventFact {
                metadata: EventMetadata {
                    event_id: Uuid::from_u128(1000 + u128::try_from(ordinal).unwrap()),
                    sequence: (1, u64::try_from(ordinal).unwrap()),
                },
                character_id: cp.character_id,
                instance_id: delta.instance,
                template_id: delta.template().unwrap(),
                count_delta: delta.count_delta,
                before: None,
                after: delta.after.clone(),
                causes: Vec::new(),
            },
        })
        .collect();
    (character, cp, work, events)
}

/// The picked-up instance: dropped in epoch 0 at tick 70, so its creation origin is (1, 0).
fn picked(revision: u64) -> ItemInstance {
    ItemInstance {
        creation_origin: Some(ItemCreationOrigin {
            zone: ZoneId(1),
            epoch: 0,
        }),
        id: ItemInstanceId::new(PICKED).unwrap(),
        template_id: 57,
        count: 120,
        location: ItemLocation::Inventory,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(70),
        revision,
    }
}

fn row(
    owner_kind: &str,
    owner: Uuid,
    op: &str,
    epoch: u64,
    tick: u64,
    ordinal: u32,
    delta: serde_json::Value,
) -> OwnerLedgerRow {
    OwnerLedgerRow {
        owner_kind: owner_kind.into(),
        owner_id: owner,
        zone_id: 1,
        epoch,
        tick,
        revision: if owner_kind == "character" { 2 } else { 0 },
        op: op.into(),
        instance_id: PICKED,
        operation_ordinal: ordinal,
        delta,
        receipt_ref: None,
    }
}

const PICKUP_RECEIPT: &str = "receipt:pick_up_item:00000000-0000-0000-0000-000000007777";

/// A V7 character row of the item projection (P5-D22): an ordinary `ItemDeltaPlan` with the
/// `economy` audit cause carrying the receipt and the B13 transfer link.
fn plan(
    operation: ItemOperation,
    instance: u64,
    before: Option<&ItemInstance>,
    after: Option<&ItemInstance>,
    ordinal: u32,
    receipt: &str,
    link: Option<&str>,
) -> ItemDeltaPlan {
    let count = |item: Option<&ItemInstance>| i64::try_from(item.map_or(0, |i| i.count)).unwrap();
    let mut fields = BTreeMap::new();
    fields.insert("receipt".to_owned(), ItemEvidenceValue::Text(receipt.into()));
    if let Some(link) = link {
        fields.insert("link".to_owned(), ItemEvidenceValue::Text(link.into()));
        fields.insert("mode".to_owned(), ItemEvidenceValue::Text("identity".into()));
    }
    ItemDeltaPlan {
        operation,
        operation_ordinal: ordinal,
        instance: ItemInstanceId::new(instance).unwrap(),
        before: before.cloned(),
        after: after.cloned(),
        count_delta: count(after) - count(before),
        receipt: None,
        receipt_refs: Vec::new(),
        causes: vec![ItemAuditEvidence {
            family: "economy".into(),
            fields,
        }],
    }
}

/// Tick 80 (revision 2): the character picks the ground stack up.
fn pickup(
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
    character_rows: bool,
    fingerprint: &str,
) -> (CharacterCheckpoint, ItemCheckpointWork, EconomyCheckpointWrite) {
    let character = cp.character_id.as_uuid();
    let mut checkpoint = work.projection.checkpoint.clone();
    checkpoint.inventory_revision = 2;
    checkpoint.instances.push(picked(2));
    checkpoint.instances.sort_by_key(|item| item.id);
    let mut next = cp.clone();
    next.revision_seen = 1;
    next.idempotency = ("save_checkpoint".into(), IdempotencyKey::new());
    next.phase4 = Some(checkpoint.clone());
    let mut work = work.clone();
    work.projection.tick = Tick(80);
    work.projection.checkpoint = checkpoint;
    work.projection.ledger = if character_rows {
        vec![plan(
            ItemOperation::TransferIn,
            PICKED,
            None,
            Some(&picked(2)),
            0,
            PICKUP_RECEIPT,
            Some(PICKUP_LINK),
        )]
    } else {
        Vec::new()
    };
    work.projection.receipts = Vec::new();
    work.projection.materializations = Vec::new();
    let mut ground = picked(0);
    ground.location = ItemLocation::Ground;
    let ledger = vec![row(
        "ground",
        GROUND,
        "transfer_out",
        1,
        80,
        0,
        serde_json::json!({"before": ground, "after": null, "transfer": {"link": PICKUP_LINK, "mode": "identity"}, "receipt": "receipt:pick_up_item:00000000-0000-0000-0000-000000007777"}),
    )];
    let write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 80,
        ledger,
        receipts: vec![EconomyReceipt {
            account_id: ACCOUNT,
            character_id: character,
            operation: "pick_up_item".into(),
            request_key: Uuid::from_u128(0x7777),
            fingerprint: fingerprint.into(),
            receipt_id: Uuid::from_u128(0x7778),
            frozen_result: "0a0208011001".into(),
        }],
        ..Default::default()
    };
    (next, work, write)
}

fn constraint(result: Result<(), CheckpointError>) -> String {
    match result {
        Err(CheckpointError::Constraint(name)) => name,
        other => panic!("expected a named constraint, got {other:?}"),
    }
}

#[tokio::test]
async fn economy_character_rows_cover_changed_scalars_and_share_t1_positions() {
    let repo = InMemoryCharacterRepository::default();
    let bundle = embedded_phase4_bundle().unwrap();
    let (_, cp, work, _) = starter(&repo, &bundle, "EconomyCover").await;
    let before = work.projection.checkpoint.clone();
    let (_, work2, write) = pickup(&cp, &work, true, "fp");
    let links = IdentityLinks::of_write(&write).unwrap();
    let validate = |work: &ItemCheckpointWork, links: &IdentityLinks| {
        work.validate_before_with_economy(Some(&before), Some(&bundle.items), links)
    };
    validate(&work2, &links).unwrap();
    // Without the character row the new scalar is unledgered (the V6 rule is unchanged).
    let (_, bare, _) = pickup(&cp, &work, false, "fp");
    assert_eq!(constraint(validate(&bare, &links)), "item_checkpoint_unledgered_scalar");
    // The character row's after must be the settled scalar and revision.
    let mut stale = work2.clone();
    stale.projection.ledger[0].after = Some(picked(1));
    assert_eq!(constraint(validate(&stale, &links)), "item_ledger_revision_step");
    // T1: the row must hold its canonical position.
    let mut gap = work2.clone();
    gap.projection.ledger[0].operation_ordinal = 1;
    assert_eq!(constraint(validate(&gap, &links)), "item_ledger_row_ordinal");
    // B13: an identity transfer_in without a matching transfer_out link of the same write is
    // refused.
    assert_eq!(
        constraint(validate(&work2, &IdentityLinks::default())),
        "economy_transfer_unlinked"
    );
    let mut other = write.clone();
    other.ledger[0].delta["transfer"]["link"] =
        serde_json::json!("6f1c3c1e-7c55-5d7e-9e1a-00000000ffff");
    assert_eq!(
        constraint(validate(&work2, &IdentityLinks::of_write(&other).unwrap())),
        "economy_transfer_unlinked"
    );
    // P5-D22: a character row inside the economy write is refused.
    let mut misplaced = write.clone();
    misplaced.ledger.push(OwnerLedgerRow {
        owner_kind: "character".into(),
        ..write.ledger[0].clone()
    });
    assert_eq!(
        constraint(IdentityLinks::of_write(&misplaced).map(drop)),
        "economy_character_row_in_economy_write"
    );
    // A receipt outside the economy scopes is refused before any write.
    let mut bad = write;
    bad.receipts[0].operation = "use_item".into();
    assert!(bad.validate().is_err());
}

#[tokio::test]
async fn postgres_pickup_settles_scalars_economy_rows_and_receipt_in_one_transaction() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, ACCOUNT).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let (character, cp, work, events) = starter(&repo, &bundle, "EconomyPickup").await;
    assert_eq!(
        repo.checkpoint_items(&cp, &events, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    // The drop: the ground owner creates the instance in epoch 0 (standalone economy write).
    let economy = PgEconomyRepository::new(
        nightfall_api::infrastructure::postgres::connection_from_pool(&pool),
    );
    let mut ground = picked(0);
    ground.location = ItemLocation::Ground;
    economy
        .checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: 1,
            epoch: 0,
            tick: 70,
            ledger: vec![row(
                "ground",
                GROUND,
                "loot",
                0,
                70,
                0,
                serde_json::json!({"before": null, "after": ground}),
            )],
            ..Default::default()
        })
        .await
        .unwrap();

    // Without the character-side row the scalar change is refused and nothing is written.
    let (next, work2, write) = pickup(&cp, &work, false, "fp");
    let refused = repo
        .checkpoint_with_economy(&next, &[], Some(&work2), &write)
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, CheckpointError::Constraint(name) if name == "item_checkpoint_unledgered_scalar"),
        "{refused:?}"
    );
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM item_mutation_receipts WHERE operation='pick_up_item'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(receipts, 0, "rolled back");

    let (next, work2, write) = pickup(&cp, &work, true, "fp");
    assert_eq!(
        repo.checkpoint_with_economy(&next, &[], Some(&work2), &write)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    let stored: (String, serde_json::Value) = sqlx::query_as(
        "SELECT fingerprint,response FROM item_mutation_receipts WHERE account_id=$1 AND operation='pick_up_item' AND key=$2",
    )
    .bind(ACCOUNT)
    .bind(Uuid::from_u128(0x7777))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored.0, "fp");
    // P5-D18: one picked_up fact, derived from the two pickup rows by the writer.
    let facts: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE subject='nightfall.item.picked_up'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(
        facts[0]["operation"],
        "receipt:pick_up_item:00000000-0000-0000-0000-000000007777"
    );
    assert_eq!(stored.1["economy_result"], "0a0208011001");
    let owners: Vec<(String, String)> =
        sqlx::query_as("SELECT owner_kind,op FROM item_ledger WHERE instance_id=$1 ORDER BY id")
            .bind(i64::try_from(PICKED).unwrap())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        owners,
        // The settlement writes the character row before the economy rows of the same
        // transaction; the drop's loot row (an earlier tick) stays first.
        vec![
            ("ground".into(), "loot".into()),
            ("character".into(), "transfer_in".into()),
            ("ground".into(), "transfer_out".into())
        ]
    );
    // The retained load takes the creation origin from the ground drop (epoch 0), not from the
    // character's own `transfer_in` (epoch 1).
    let loaded = repo
        .load_for_admission(character.id)
        .await
        .unwrap()
        .unwrap()
        .phase4
        .unwrap()
        .checkpoint
        .unwrap();
    let item = loaded
        .instances
        .iter()
        .find(|i| i.id.get() == PICKED)
        .unwrap();
    assert_eq!(item, &picked(2));
    assert_eq!(loaded.inventory_revision, 2);

    // The same request key again: same fingerprint is a replay (nothing applied), another
    // fingerprint is the D43 conflict.
    let (mut again, work3, write3) = pickup(&cp, &work, true, "fp");
    again.revision_seen = 2;
    assert_eq!(
        repo.checkpoint_with_economy(&again, &[], Some(&work3), &write3)
            .await
            .unwrap(),
        CheckpointOutcome::Stale
    );
    let (mut other, work4, write4) = pickup(&cp, &work, true, "fp-other");
    other.revision_seen = 2;
    assert!(matches!(
        repo.checkpoint_with_economy(&other, &[], Some(&work4), &write4)
            .await
            .unwrap_err(),
        CheckpointError::KeyReused
    ));
    // Receipts only settle with a character checkpoint.
    assert!(economy.checkpoint_economy(&write4).await.is_err());
}

/// One owner row of a deposit or withdraw (identity mode, B13 link).
fn moved(
    owner_kind: &str,
    owner: Uuid,
    op: &str,
    tick: u64,
    item: u64,
    before: Option<&ItemInstance>,
    after: Option<&ItemInstance>,
    link: Option<&str>,
) -> OwnerLedgerRow {
    let mut delta = serde_json::json!({"before": before, "after": after});
    if let Some(link) = link {
        delta["transfer"] = serde_json::json!({"link": link, "mode": "identity"});
    }
    OwnerLedgerRow {
        owner_kind: owner_kind.into(),
        owner_id: owner,
        zone_id: 1,
        epoch: 1,
        tick,
        revision: after.or(before).map_or(0, |i| i.revision),
        op: op.into(),
        instance_id: item,
        operation_ordinal: 0,
        delta,
        receipt_ref: None,
    }
}

fn located(item: &ItemInstance, location: ItemLocation, revision: u64) -> ItemInstance {
    ItemInstance {
        location,
        revision,
        ..item.clone()
    }
}

/// The next tick's character checkpoint with `instances` at `revision` (no V6 rows).
fn tick(
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
    seen: u64,
    revision: u64,
    tick: u64,
    instances: Vec<ItemInstance>,
) -> (CharacterCheckpoint, ItemCheckpointWork) {
    let mut checkpoint = work.projection.checkpoint.clone();
    checkpoint.inventory_revision = revision;
    checkpoint.instances = instances;
    checkpoint.instances.sort_by_key(|item| item.id);
    let mut next = cp.clone();
    next.revision_seen = seen;
    next.idempotency = ("save_checkpoint".into(), IdempotencyKey::new());
    next.phase4 = Some(checkpoint.clone());
    let mut work = work.clone();
    work.projection.tick = Tick(tick);
    work.projection.checkpoint = checkpoint;
    work.projection.ledger = Vec::new();
    work.projection.receipts = Vec::new();
    work.projection.materializations = Vec::new();
    (next, work)
}

fn grant(keys: Vec<LeaseKey>, n: u128) -> LeaseGrant {
    LeaseGrant {
        zone_id: 1,
        epoch: 1,
        account_id: Some(ACCOUNT),
        operation: "warehouse_deposit".into(),
        request_key: Uuid::from_u128(n),
        keys,
    }
}

/// P5-D16: a deposit (inventory delete + container Insert) and a withdraw (container Delete +
/// inventory upsert) of the SAME instance id through `checkpoint_with_economy` with items.
async fn deposit_and_withdraw<R>(
    repo: &R,
    leases: &dyn LeaseRepository,
    reads: &dyn EconomyReadRepository,
    bundle: &Phase4RuleBundle,
    name: &str,
) where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + EconomyCharacterCheckpointRepository,
{
    let (character, cp, work, events) = starter(repo, bundle, name).await;
    assert_eq!(
        repo.checkpoint_items(&cp, &events, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let owner = character.id.as_uuid();
    let key = LeaseKey::of_uuid(LeaseKind::Warehouse, owner);
    let item = work.projection.checkpoint.instances[0].clone();
    let id = item.id.get();
    let rest: Vec<ItemInstance> = work.projection.checkpoint.instances[1..].to_vec();
    let link_in = "6f1c3c1e-7c55-5d7e-9e1a-0000000000d1";
    let link_out = "6f1c3c1e-7c55-5d7e-9e1a-0000000000d2";

    // Deposit, tick 80 (revision 2): the settlement deletes the inventory row, then the
    // container Insert writes the same id as a warehouse row.
    leases.acquire(&grant(vec![key], 1)).await.unwrap();
    let stored = located(&item, ItemLocation::Warehouse, 0);
    let (cp2, mut work2) = tick(&cp, &work, 1, 2, 80, rest.clone());
    work2.projection.ledger = vec![plan(
        ItemOperation::TransferOut,
        id,
        Some(&item),
        None,
        0,
        "receipt:warehouse_deposit:k1",
        Some(link_in),
    )];
    let deposit = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 80,
        deltas: vec![ContainerDelta {
            lease: key,
            row_kind: "item_instance".into(),
            row_key: id.to_string(),
            change: DeltaChange::Insert,
            expected_revision: None,
            columns: [
                ("character_id", serde_json::json!(owner.to_string())),
                ("owner_kind", serde_json::json!("warehouse")),
                ("location", serde_json::json!("warehouse")),
                ("template_id", serde_json::json!(item.template_id)),
                ("count", serde_json::json!(item.count)),
                ("created_tick", serde_json::json!(item.created_tick.0)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        }],
        ledger: vec![moved(
            "warehouse",
            owner,
            "transfer_in",
            80,
            id,
            None,
            Some(&stored),
            Some(link_in),
        )],
        lease_releases: vec![key],
        ..Default::default()
    };
    assert_eq!(
        repo.checkpoint_with_economy(&cp2, &[], Some(&work2), &deposit)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(2)
    );
    let warehouse = reads.container_items("warehouse", owner).await.unwrap();
    assert_eq!(
        warehouse.iter().map(|i| i.instance_id).collect::<Vec<_>>(),
        vec![id],
        "same id in the warehouse"
    );
    let loaded = repo
        .load_for_admission(character.id)
        .await
        .unwrap()
        .unwrap()
        .phase4
        .unwrap()
        .checkpoint
        .unwrap();
    assert!(loaded.instances.iter().all(|i| i.id != item.id), "left the inventory");

    // Withdraw, tick 81 (revision 3): the container Delete runs before the settlement upsert.
    let back = located(&item, ItemLocation::Inventory, 3);
    let mut instances = rest.clone();
    instances.push(back.clone());
    let (cp3, base3) = tick(&cp, &work, 2, 3, 81, instances);
    let work3 = |link: Option<&str>| {
        let mut w = base3.clone();
        w.projection.ledger = vec![plan(
            ItemOperation::TransferIn,
            id,
            None,
            Some(&back),
            0,
            "receipt:warehouse_withdraw:k2",
            link,
        )];
        w
    };
    let withdraw = || EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 81,
        deltas: vec![ContainerDelta {
            lease: key,
            row_kind: "item_instance".into(),
            row_key: id.to_string(),
            change: DeltaChange::Delete,
            expected_revision: Some(warehouse[0].revision),
            columns: serde_json::Map::new(),
        }],
        ledger: vec![moved(
            "warehouse",
            owner,
            "transfer_out",
            81,
            id,
            Some(&stored),
            None,
            Some(link_out),
        )],
        lease_releases: vec![key],
        ..Default::default()
    };
    // Without the B13 link the instance's history makes the before = None row a D67 reuse.
    leases.acquire(&grant(vec![key], 2)).await.unwrap();
    let refused = repo
        .checkpoint_with_economy(&cp3, &[], Some(&work3(None)), &withdraw())
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, CheckpointError::Constraint(n) if n == "item_instance_identity_reused"),
        "{refused:?}"
    );
    assert_eq!(
        reads
            .container_items("warehouse", owner)
            .await
            .unwrap()
            .len(),
        1,
        "nothing written"
    );
    assert_eq!(
        repo.checkpoint_with_economy(&cp3, &[], Some(&work3(Some(link_out))), &withdraw())
            .await
            .unwrap(),
        CheckpointOutcome::Applied(3)
    );
    assert!(reads
        .container_items("warehouse", owner)
        .await
        .unwrap()
        .is_empty());
    let loaded = repo
        .load_for_admission(character.id)
        .await
        .unwrap()
        .unwrap()
        .phase4
        .unwrap()
        .checkpoint
        .unwrap();
    let returned = loaded.instances.iter().find(|i| i.id == item.id).unwrap();
    assert_eq!(returned, &back, "same id, same creation origin (the V6 starter birth)");
}

#[tokio::test]
async fn memory_deposit_and_withdraw_keep_the_instance_id() {
    let bundle = embedded_phase4_bundle().unwrap();
    let store = Arc::new(InMemoryEconomyStore::default());
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap()
        .with_economy(store.clone());
    deposit_and_withdraw(&repo, store.as_ref(), store.as_ref(), &bundle, "MemoryDeposit").await;
}

#[tokio::test]
async fn postgres_deposit_and_withdraw_keep_the_instance_id() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, ACCOUNT).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let economy = PgEconomyRepository::new(
        nightfall_api::infrastructure::postgres::connection_from_pool(&pool),
    );
    deposit_and_withdraw(&repo, &economy, &economy, &bundle, "PgDeposit").await;
}

// ---- Story 4.8 (design §14.4 "Crash windows"): the character-checkpoint rows -------------------
//
// Each row runs on memory and Postgres through `checkpoint_with_economy`:
// - crash after commit, before the ack: the identical retry replays (`Replayed`) and the zone's
//   redo of the same request (a new checkpoint key, the same receipt) is `Stale`. Nothing is
//   applied twice: one fact, one container change.
// - crash before commit (memory injects it; on Postgres a crash before COMMIT leaves nothing by
//   construction): nothing is written, and the retry applies once.

fn receipt(character: Uuid, scope: &str, n: u128, fingerprint: &str) -> EconomyReceipt {
    EconomyReceipt {
        account_id: ACCOUNT,
        character_id: character,
        operation: scope.into(),
        request_key: Uuid::from_u128(n),
        fingerprint: fingerprint.into(),
        receipt_id: Uuid::from_u128(n + 1),
        frozen_result: "0a00".into(),
    }
}

fn receipt_text(scope: &str, n: u128) -> String {
    format!("receipt:{scope}:{}", Uuid::from_u128(n))
}

fn ground_row(
    op: &str,
    owner: Uuid,
    tick: u64,
    instance: u64,
    delta: serde_json::Value,
) -> OwnerLedgerRow {
    OwnerLedgerRow {
        instance_id: instance,
        ..row("ground", owner, op, 1, tick, 0, delta)
    }
}

fn item(
    id: u64,
    template: u32,
    count: u64,
    location: ItemLocation,
    revision: u64,
    created: u64,
) -> ItemInstance {
    ItemInstance {
        creation_origin: Some(ItemCreationOrigin {
            zone: ZoneId(1),
            epoch: 1,
        }),
        id: ItemInstanceId::new(id).unwrap(),
        template_id: template,
        count,
        location,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(created),
        revision,
    }
}

/// Settle `write` + `work` once, prove the replay and the zone redo, and count the facts.
async fn settle_with_replays<R>(
    repo: &R,
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
    write: &EconomyCheckpointWrite,
    applied: u64,
) where
    R: EconomyCharacterCheckpointRepository,
{
    assert_eq!(
        repo.checkpoint_with_economy(cp, &[], Some(work), write)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(applied)
    );
    assert_eq!(
        repo.checkpoint_with_economy(cp, &[], Some(work), write)
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(applied),
        "identical retry"
    );
    if !write.receipts.is_empty() {
        let mut redo = cp.clone();
        redo.idempotency = ("save_checkpoint".into(), IdempotencyKey::new());
        redo.revision_seen = applied;
        assert_eq!(
            repo.checkpoint_with_economy(&redo, &[], Some(work), write)
                .await
                .unwrap(),
            CheckpointOutcome::Stale,
            "zone redo of the same request"
        );
    }
}

trait EconomyAdapter:
    LeaseRepository + EconomyCheckpointRepository + EconomyReadRepository + Sync
{
}
impl<T: LeaseRepository + EconomyCheckpointRepository + EconomyReadRepository + Sync> EconomyAdapter
    for T
{
}

async fn commit_crash_windows<R>(
    repo: &R,
    economy: &dyn EconomyAdapter,
    facts: &(dyn Fn(&str) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send>>
          + Sync),
    inject: Option<&InMemoryEconomyStore>,
    bundle: &Phase4RuleBundle,
    name: &str,
) where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + EconomyCharacterCheckpointRepository,
{
    let (character, cp, work, events) = starter(repo, bundle, name).await;
    assert_eq!(
        repo.checkpoint_items(&cp, &events, &work, None)
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    let me = character.id.as_uuid();
    let (receiver, _, _, _) = starter(repo, bundle, &format!("{name}R")).await;
    let ground = Uuid::from_u128(0x6100);
    // Drops in epoch 1: adena 9_100_001 (120) and 9_100_002 (10).
    let adena = item(9_100_001, 57, 120, ItemLocation::Ground, 0, 70);
    let split_drop = item(9_100_002, 57, 10, ItemLocation::Ground, 0, 71);
    economy
        .checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: 1,
            epoch: 1,
            tick: 72,
            ledger: vec![
                ground_row("loot", ground, 72, 9_100_001, serde_json::json!({"before": null, "after": adena, "causes": ["loot:npc@72"]})),
                ground_row("loot", Uuid::from_u128(0x6101), 72, 9_100_002, serde_json::json!({"before": null, "after": split_drop, "causes": ["loot:npc@72"]})),
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let mut instances = work.projection.checkpoint.instances.clone();

    // (1) Pickup, tick 80, revision 2.
    let held = item(9_100_001, 57, 120, ItemLocation::Inventory, 2, 70);
    instances.push(held.clone());
    let (cp2, mut w2) = tick(&cp, &work, 1, 2, 80, instances.clone());
    let pick = receipt_text("pick_up_item", 0x8000);
    w2.projection.ledger = vec![plan(
        ItemOperation::TransferIn,
        9_100_001,
        None,
        Some(&held),
        0,
        &pick,
        Some("6f1c3c1e-7c55-5d7e-9e1a-0000000000a1"),
    )];
    let pickup = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 80,
        ledger: vec![ground_row(
            "transfer_out",
            ground,
            80,
            9_100_001,
            serde_json::json!({"before": adena, "after": null, "receipt": pick, "transfer": {"link": "6f1c3c1e-7c55-5d7e-9e1a-0000000000a1", "mode": "identity"}}),
        )],
        receipts: vec![receipt(me, "pick_up_item", 0x8000, "fp-pick")],
        ..Default::default()
    };
    if let Some(store) = inject {
        // Crash before commit: the economy half fails, so nothing of the tick is written.
        store.fail_next_transient(1);
        assert!(repo
            .checkpoint_with_economy(&cp2, &[], Some(&w2), &pickup)
            .await
            .is_err());
        let loaded = repo
            .load_for_admission(character.id)
            .await
            .unwrap()
            .unwrap()
            .phase4
            .unwrap()
            .checkpoint
            .unwrap();
        assert!(loaded.instances.iter().all(|i| i.id.get() != 9_100_001));
        assert_eq!(facts("picked_up").await, 0);
    }
    settle_with_replays(repo, &cp2, &w2, &pickup, 2).await;
    assert_eq!(facts("picked_up").await, 1, "one pickup fact");

    // (2) Party even split, tick 81, revision 3: the ground stack is destroyed and the (only)
    // recipient gets a fresh party_split row.
    let mut allocation = w2.allocation;
    let fresh = allocation.allocate().unwrap();
    let share = item(fresh.get(), 57, 10, ItemLocation::Inventory, 3, 81);
    instances.push(share.clone());
    let (cp3, mut w3) = tick(&cp2, &w2, 2, 3, 81, instances.clone());
    w3.allocation = allocation;
    let split = receipt_text("pick_up_item", 0x8100);
    w3.projection.ledger = vec![plan(
        ItemOperation::PartySplit,
        fresh.get(),
        None,
        Some(&share),
        0,
        &split,
        None,
    )];
    let party = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 81,
        ledger: vec![OwnerLedgerRow {
            owner_id: Uuid::from_u128(0x6101),
            ..ground_row(
                "destroy",
                ground,
                81,
                9_100_002,
                serde_json::json!({"before": split_drop, "after": null, "receipt": split, "causes": ["party_split:r"]}),
            )
        }],
        receipts: vec![receipt(me, "pick_up_item", 0x8100, "fp-split")],
        ..Default::default()
    };
    settle_with_replays(repo, &cp3, &w3, &party, 3).await;
    assert_eq!(facts("picked_up").await, 2, "one fact per pickup operation");

    // (3) Warehouse deposit with merge, tick 82, revision 4: the inventory stack leaves whole,
    // the warehouse stack of the template grows (CAS Update).
    let stack = work
        .projection
        .checkpoint
        .instances
        .iter()
        .find(|i| i.count > 1)
        .cloned()
        .expect("a stackable starter item");
    let key = LeaseKey::of_uuid(LeaseKind::Warehouse, me);
    economy.acquire(&grant(vec![key], 0x8200)).await.unwrap();
    let stored = item(9_100_100, stack.template_id, 3, ItemLocation::Warehouse, 0, 1);
    economy
        .checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: 1,
            epoch: 1,
            tick: 75,
            deltas: vec![ContainerDelta {
                lease: key,
                row_kind: "item_instance".into(),
                row_key: "9100100".into(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("character_id", serde_json::json!(me.to_string())),
                    ("owner_kind", serde_json::json!("warehouse")),
                    ("location", serde_json::json!("warehouse")),
                    ("template_id", serde_json::json!(stack.template_id)),
                    ("count", serde_json::json!(3)),
                    ("created_tick", serde_json::json!(1)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            }],
            ledger: vec![OwnerLedgerRow {
                owner_kind: "warehouse".into(),
                owner_id: me,
                instance_id: 9_100_100,
                ..ground_row(
                    "transfer_in",
                    me,
                    75,
                    9_100_100,
                    serde_json::json!({"before": null, "after": stored}),
                )
            }],
            lease_releases: vec![key],
            ..Default::default()
        })
        .await
        .unwrap();
    economy.acquire(&grant(vec![key], 0x8201)).await.unwrap();
    instances.retain(|i| i.id != stack.id);
    let (cp4, mut w4) = tick(&cp3, &w3, 3, 4, 82, instances.clone());
    let deposit = receipt_text("warehouse_deposit", 0x8300);
    let merge_link = "6f1c3c1e-7c55-5d7e-9e1a-0000000000a3";
    let mut out = plan(
        ItemOperation::TransferOut,
        stack.id.get(),
        Some(&stack),
        None,
        0,
        &deposit,
        None,
    );
    out.causes[0]
        .fields
        .insert("link".into(), ItemEvidenceValue::Text(merge_link.into()));
    out.causes[0]
        .fields
        .insert("mode".into(), ItemEvidenceValue::Text("merge".into()));
    w4.projection.ledger = vec![out];
    let grown = item(9_100_100, stack.template_id, 3 + stack.count, ItemLocation::Warehouse, 1, 1);
    let merge = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 82,
        deltas: vec![ContainerDelta {
            lease: key,
            row_kind: "item_instance".into(),
            row_key: "9100100".into(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: [
                ("character_id", serde_json::json!(me.to_string())),
                ("owner_kind", serde_json::json!("warehouse")),
                ("location", serde_json::json!("warehouse")),
                ("count", serde_json::json!(3 + stack.count)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        }],
        ledger: vec![OwnerLedgerRow {
            owner_kind: "warehouse".into(),
            owner_id: me,
            instance_id: 9_100_100,
            ..ground_row(
                "transfer_in",
                me,
                82,
                9_100_100,
                serde_json::json!({"before": stored, "after": grown, "receipt": deposit, "transfer": {"link": merge_link, "mode": "merge"}}),
            )
        }],
        lease_releases: vec![key],
        receipts: vec![receipt(me, "warehouse_deposit", 0x8300, "fp-deposit")],
        ..Default::default()
    };
    settle_with_replays(repo, &cp4, &w4, &merge, 4).await;
    let warehouse = economy.container_items("warehouse", me).await.unwrap();
    assert_eq!(
        (warehouse.len(), warehouse[0].count),
        (1, 3 + stack.count),
        "merged exactly once"
    );
    assert_eq!(facts("deposited").await, 1);

    // (4) Auction bid, tick 83, revision 5: 100 adena split off the picked stack into escrow.
    let auction = LeaseKey {
        kind: LeaseKind::Auction,
        owner: 7,
    };
    economy
        .acquire(&grant(vec![auction], 0x8400))
        .await
        .unwrap();
    economy
        .checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: 1,
            epoch: 1,
            tick: 76,
            deltas: vec![ContainerDelta {
                lease: auction,
                row_kind: "item_auction".into(),
                row_key: "7".into(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("instance_npc", serde_json::json!(32320)),
                    ("item_index", serde_json::json!(0)),
                    ("state", serde_json::json!(1)),
                    ("start_unix_ms", serde_json::json!(1_000)),
                    ("end_unix_ms", serde_json::json!(4_102_444_800_000_i64)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            }],
            lease_releases: vec![auction],
            ..Default::default()
        })
        .await
        .unwrap();
    economy
        .acquire(&grant(nightfall_api::application::economy::sets::auction(7), 0x8401))
        .await
        .unwrap();
    let left = item(9_100_001, 57, 20, ItemLocation::Inventory, 5, 70);
    instances.retain(|i| i.id.get() != 9_100_001);
    instances.push(left.clone());
    let (cp5, mut w5) = tick(&cp4, &w4, 4, 5, 83, instances.clone());
    let bid = receipt_text("auction_bid", 0x8500);
    let bid_link = "6f1c3c1e-7c55-5d7e-9e1a-0000000000a5";
    let mut split_out =
        plan(ItemOperation::TransferOut, 9_100_001, Some(&held), Some(&left), 0, &bid, None);
    split_out.causes[0]
        .fields
        .insert("link".into(), ItemEvidenceValue::Text(bid_link.into()));
    split_out.causes[0]
        .fields
        .insert("mode".into(), ItemEvidenceValue::Text("split".into()));
    w5.projection.ledger = vec![split_out];
    let escrow_owner = nightfall_api::application::economy::auction_bid_owner(7, me);
    let escrow = item(9_100_200, 57, 100, ItemLocation::AuctionBid, 0, 83);
    let bid_write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 83,
        deltas: vec![
            ContainerDelta {
                lease: auction,
                row_kind: "item_auction".into(),
                row_key: "7".into(),
                change: DeltaChange::Update,
                expected_revision: Some(0),
                columns: [
                    ("instance_npc", serde_json::json!(32320)),
                    ("item_index", serde_json::json!(0)),
                    ("state", serde_json::json!(1)),
                    ("highest_bidder", serde_json::json!(me.to_string())),
                    ("last_bidder", serde_json::json!(me.to_string())),
                    ("highest_bid", serde_json::json!(100)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            },
            ContainerDelta {
                lease: auction,
                row_kind: "item_auction_bid".into(),
                row_key: format!("7:{me}"),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("auction_id", serde_json::json!(7)),
                    ("character_id", serde_json::json!(me.to_string())),
                    ("last_bid", serde_json::json!(100)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            },
            ContainerDelta {
                lease: auction,
                row_kind: "item_instance".into(),
                row_key: "9100200".into(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("owner_kind", serde_json::json!("auction_bid")),
                    ("location", serde_json::json!("auction_bid")),
                    ("container_id", serde_json::json!(escrow_owner.to_string())),
                    ("template_id", serde_json::json!(57)),
                    ("count", serde_json::json!(100)),
                    ("created_tick", serde_json::json!(83)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            },
        ],
        ledger: vec![OwnerLedgerRow {
            owner_kind: "auction_bid".into(),
            owner_id: escrow_owner,
            instance_id: 9_100_200,
            ..ground_row(
                "transfer_in",
                escrow_owner,
                83,
                9_100_200,
                serde_json::json!({"before": null, "after": escrow, "receipt": bid, "transfer": {"link": bid_link, "mode": "split"}}),
            )
        }],
        lease_releases: nightfall_api::application::economy::sets::auction(7),
        receipts: vec![receipt(me, "auction_bid", 0x8500, "fp-bid")],
        ..Default::default()
    };
    settle_with_replays(repo, &cp5, &w5, &bid_write, 5).await;
    assert_eq!(economy.bids(7).await.unwrap().len(), 1);
    assert_eq!(economy.auctions(32320).await.unwrap()[0].highest_bid, Some(100));
    assert_eq!(facts("auction_bid").await, 1);

    // (5) Mail send with COD, tick 84, revision 6: a non-stackable item to another character's
    // mailbox. The COD payment and the take are blocked on the economy-Consume contract (see
    // api-inbox, E4.8 note); the delivery row they create executes as a system job, which
    // `phase5_crash_windows` covers on both adapters.
    let gift = work
        .projection
        .checkpoint
        .instances
        .iter()
        .find(|i| i.count == 1)
        .cloned()
        .unwrap();
    let message = Uuid::from_u128(0x8600);
    economy
        .acquire(&grant(
            nightfall_api::application::economy::sets::mail_send(me, receiver.id.as_uuid()),
            0x8600,
        ))
        .await
        .unwrap();
    instances.retain(|i| i.id != gift.id);
    let (cp6, mut w6) = tick(&cp5, &w5, 5, 6, 84, instances.clone());
    let send = receipt_text("mail_send", 0x8700);
    let mail_link = "6f1c3c1e-7c55-5d7e-9e1a-0000000000a7";
    w6.projection.ledger = vec![plan(
        ItemOperation::TransferOut,
        gift.id.get(),
        Some(&gift),
        None,
        0,
        &send,
        Some(mail_link),
    )];
    let attached = located(&gift, ItemLocation::Mail, 0);
    let mail_key = LeaseKey::of_uuid(LeaseKind::Mailbox, receiver.id.as_uuid());
    let mail_write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 84,
        deltas: vec![
            ContainerDelta {
                lease: mail_key,
                row_kind: "mail_message".into(),
                row_key: message.to_string(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("sender_id", serde_json::json!(me.to_string())),
                    ("receiver_id", serde_json::json!(receiver.id.as_uuid().to_string())),
                    ("subject", serde_json::json!("cod")),
                    ("body", serde_json::json!("pay")),
                    ("cod_adena", serde_json::json!(100)),
                    ("has_attachments", serde_json::json!(true)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            },
            ContainerDelta {
                lease: mail_key,
                row_kind: "item_instance".into(),
                row_key: gift.id.get().to_string(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: [
                    ("owner_kind", serde_json::json!("mail")),
                    ("location", serde_json::json!("mail")),
                    ("container_id", serde_json::json!(message.to_string())),
                    ("mail_id", serde_json::json!(message.to_string())),
                    ("template_id", serde_json::json!(gift.template_id)),
                    ("count", serde_json::json!(1)),
                    ("created_tick", serde_json::json!(gift.created_tick.0)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect(),
            },
        ],
        ledger: vec![OwnerLedgerRow {
            owner_kind: "mail".into(),
            owner_id: message,
            instance_id: gift.id.get(),
            ..ground_row(
                "transfer_in",
                message,
                84,
                gift.id.get(),
                serde_json::json!({"before": null, "after": attached, "receipt": send, "transfer": {"link": mail_link, "mode": "identity"}}),
            )
        }],
        lease_releases: nightfall_api::application::economy::sets::mail_send(
            me,
            receiver.id.as_uuid(),
        ),
        receipts: vec![receipt(me, "mail_send", 0x8700, "fp-mail")],
        ..Default::default()
    };
    settle_with_replays(repo, &cp6, &w6, &mail_write, 6).await;
    assert_eq!(economy.mail_attachments(message).await.unwrap().len(), 1);
    assert_eq!(facts("mailed").await, 1);
}

#[tokio::test]
async fn memory_commit_crash_windows() {
    let bundle = embedded_phase4_bundle().unwrap();
    let store = Arc::new(InMemoryEconomyStore::default());
    let repo = InMemoryCharacterRepository::default()
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap()
        .with_economy(store.clone());
    let facts = |subject: &str| {
        let subject = subject.to_owned();
        let store = store.clone();
        Box::pin(async move {
            store
                .outbox()
                .iter()
                .filter(|f| f.subject == subject)
                .count()
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send>>
    };
    commit_crash_windows(&repo, store.as_ref(), &facts, Some(store.as_ref()), &bundle, "MemCrash")
        .await;
}

#[tokio::test]
async fn postgres_commit_crash_windows() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, ACCOUNT).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = PgCharacterRepository::new(pool.clone())
        .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
        .unwrap();
    let economy = PgEconomyRepository::new(
        nightfall_api::infrastructure::postgres::connection_from_pool(&pool),
    );
    let facts = |subject: &str| {
        let subject = format!("nightfall.item.{subject}");
        let pool = pool.clone();
        Box::pin(async move {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE subject=$1")
                .bind(subject)
                .fetch_one(&pool)
                .await
                .unwrap();
            usize::try_from(n).unwrap()
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send>>
    };
    commit_crash_windows(&repo, &economy, &facts, None, &bundle, "PgCrash").await;
}

// ---- P5-D27: the multi-character writer (trade, private store) ---------------------------------

struct Trader {
    cp: CharacterCheckpoint,
    work: ItemCheckpointWork,
    id: Uuid,
}

/// A and B swap one non-stackable item each (identity transfers, links L1/L2), revision 2.
fn swap(
    a: &Trader,
    b: &Trader,
    key: u128,
) -> (
    (CharacterCheckpoint, ItemCheckpointWork),
    (CharacterCheckpoint, ItemCheckpointWork),
    EconomyCheckpointWrite,
) {
    let pick = |t: &Trader| {
        t.work
            .projection
            .checkpoint
            .instances
            .iter()
            .find(|i| i.count == 1)
            .cloned()
            .unwrap()
    };
    let (x, y) = (pick(a), pick(b));
    let receipt_key = receipt_text("trade_confirm", key);
    let l1 = Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("l1:{key}").as_bytes()).to_string();
    let l2 = Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("l2:{key}").as_bytes()).to_string();
    let side =
        |me: &Trader, give: &ItemInstance, take: &ItemInstance, out_link: &str, in_link: &str| {
            let mut instances: Vec<ItemInstance> = me
                .work
                .projection
                .checkpoint
                .instances
                .iter()
                .filter(|i| i.id != give.id)
                .cloned()
                .collect();
            let taken = ItemInstance {
                revision: 2,
                ..take.clone()
            };
            instances.push(taken.clone());
            let (cp, mut work) = tick(&me.cp, &me.work, 1, 2, 90, instances);
            let mut rows = vec![
                plan(
                    ItemOperation::TransferOut,
                    give.id.get(),
                    Some(give),
                    None,
                    0,
                    &receipt_key,
                    Some(out_link),
                ),
                plan(
                    ItemOperation::TransferIn,
                    take.id.get(),
                    None,
                    Some(&taken),
                    0,
                    &receipt_key,
                    Some(in_link),
                ),
            ];
            rows.sort_by_key(|d| d.instance);
            work.projection.ledger = rows;
            (cp, work)
        };
    let a_side = side(a, &x, &y, &l1, &l2);
    let b_side = side(b, &y, &x, &l2, &l1);
    let write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 90,
        receipts: vec![receipt(
            b.id,
            "trade_confirm",
            key,
            &format!("fp-trade-{key}"),
        )],
        ..Default::default()
    };
    (a_side, b_side, write)
}

async fn traders<R: CharacterRepository + ItemAdmissionRepository + ItemCheckpointRepository>(
    repo: &R,
    bundle: &Phase4RuleBundle,
    name: &str,
) -> (Trader, Trader) {
    let mut out = Vec::new();
    for suffix in ["A", "B"] {
        let (character, cp, work, events) = starter(repo, bundle, &format!("{name}{suffix}")).await;
        assert_eq!(
            repo.checkpoint_items(&cp, &events, &work, None)
                .await
                .unwrap(),
            CheckpointOutcome::Applied(1)
        );
        out.push(Trader {
            cp,
            work,
            id: character.id.as_uuid(),
        });
    }
    let b = out.pop().unwrap();
    let a = out.pop().unwrap();
    (a, b)
}

async fn holds<R: CharacterRepository>(repo: &R, who: Uuid, item: ItemInstanceId) -> bool {
    repo.load_for_admission(nightfall_api::domain::CharacterId::from_uuid(who))
        .await
        .unwrap()
        .unwrap()
        .phase4
        .unwrap()
        .checkpoint
        .unwrap()
        .instances
        .iter()
        .any(|i| i.id == item)
}

async fn multi_character_contract<R>(repo: Arc<R>, bundle: &Phase4RuleBundle, name: &str)
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + EconomyCharacterCheckpointRepository
        + 'static,
{
    use nightfall_api::application::economy::CharacterCheckpointPart;
    let part = |side: &(CharacterCheckpoint, ItemCheckpointWork)| -> (CharacterCheckpoint, ItemCheckpointWork) { side.clone() };
    // (1) A partial fence miss writes nothing: B's part has a stale revision.
    let (a, b) = traders(repo.as_ref(), bundle, &format!("{name}F")).await;
    let (sa, sb, write) = swap(&a, &b, 0x9100);
    let mut stale_b = sb.0.clone();
    stale_b.revision_seen = 7;
    let outcomes = repo
        .checkpoint_many(
            &[
                CharacterCheckpointPart {
                    cp: &sa.0,
                    events: &[],
                    items: Some(&sa.1),
                },
                CharacterCheckpointPart {
                    cp: &stale_b,
                    events: &[],
                    items: Some(&sb.1),
                },
            ],
            &write,
        )
        .await
        .unwrap();
    assert_eq!(
        outcomes,
        vec![CheckpointOutcome::Stale, CheckpointOutcome::Stale],
        "all or nothing"
    );
    let x =
        sa.1.projection
            .ledger
            .iter()
            .find(|d| d.operation == ItemOperation::TransferOut)
            .unwrap()
            .instance;
    assert!(
        holds(repo.as_ref(), a.id, x).await && !holds(repo.as_ref(), b.id, x).await,
        "nothing moved"
    );
    // The swap applies in either part order (fences are canonical), once; the retry replays.
    let parts = [
        CharacterCheckpointPart {
            cp: &sb.0,
            events: &[],
            items: Some(&sb.1),
        },
        CharacterCheckpointPart {
            cp: &sa.0,
            events: &[],
            items: Some(&sa.1),
        },
    ];
    assert_eq!(
        repo.checkpoint_many(&parts, &write).await.unwrap(),
        vec![CheckpointOutcome::Applied(2), CheckpointOutcome::Applied(2)]
    );
    assert!(
        holds(repo.as_ref(), b.id, x).await && !holds(repo.as_ref(), a.id, x).await,
        "X re-homed A to B, same id"
    );
    let y =
        sb.1.projection
            .ledger
            .iter()
            .find(|d| d.operation == ItemOperation::TransferOut)
            .unwrap()
            .instance;
    assert!(
        holds(repo.as_ref(), a.id, y).await && !holds(repo.as_ref(), b.id, y).await,
        "Y re-homed B to A"
    );
    assert_eq!(
        repo.checkpoint_many(&parts, &write).await.unwrap(),
        vec![
            CheckpointOutcome::Replayed(2),
            CheckpointOutcome::Replayed(2)
        ]
    );

    // (2) Opposite-order writers on the same pair: no deadlock, exactly one applies.
    let (a, b) = traders(repo.as_ref(), bundle, &format!("{name}O")).await;
    let (sa1, sb1, w1) = swap(&a, &b, 0x9200);
    let (sa2, sb2, w2) = swap(&a, &b, 0x9201);
    let (r1, r2) = (repo.clone(), repo.clone());
    let (p1, p2) = (part(&sa1), part(&sb1));
    let (q1, q2) = (part(&sa2), part(&sb2));
    let first = tokio::spawn(async move {
        r1.checkpoint_many(
            &[
                CharacterCheckpointPart {
                    cp: &p1.0,
                    events: &[],
                    items: Some(&p1.1),
                },
                CharacterCheckpointPart {
                    cp: &p2.0,
                    events: &[],
                    items: Some(&p2.1),
                },
            ],
            &w1,
        )
        .await
    });
    let second = tokio::spawn(async move {
        r2.checkpoint_many(
            &[
                CharacterCheckpointPart {
                    cp: &q2.0,
                    events: &[],
                    items: Some(&q2.1),
                },
                CharacterCheckpointPart {
                    cp: &q1.0,
                    events: &[],
                    items: Some(&q1.1),
                },
            ],
            &w2,
        )
        .await
    });
    let results = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        (first.await.unwrap().unwrap(), second.await.unwrap().unwrap())
    })
    .await
    .expect("opposite-order writers finish (no deadlock)");
    let applied = [&results.0, &results.1]
        .iter()
        .filter(|o| o.iter().all(|x| matches!(x, CheckpointOutcome::Applied(2))))
        .count();
    let stale = [&results.0, &results.1]
        .iter()
        .filter(|o| o.iter().all(|x| *x == CheckpointOutcome::Stale))
        .count();
    assert_eq!((applied, stale), (1, 1), "{results:?}");

    // (3) An 8-way race on one pair: exactly one write applies, seven are Stale.
    let (a, b) = traders(repo.as_ref(), bundle, &format!("{name}R")).await;
    let mut tasks = Vec::new();
    for n in 0..8_u128 {
        let (sa, sb, w) = swap(&a, &b, 0x9300 + n);
        let r = repo.clone();
        tasks.push(tokio::spawn(async move {
            let parts = if n % 2 == 0 {
                vec![(sa.0, sa.1), (sb.0, sb.1)]
            } else {
                vec![(sb.0, sb.1), (sa.0, sa.1)]
            };
            let refs: Vec<CharacterCheckpointPart<'_>> = parts
                .iter()
                .map(|(cp, work)| CharacterCheckpointPart {
                    cp,
                    events: &[],
                    items: Some(work),
                })
                .collect();
            r.checkpoint_many(&refs, &w).await.unwrap()
        }));
    }
    let mut winners = 0;
    for task in tasks {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(120), task)
            .await
            .expect("no deadlock")
            .unwrap();
        if outcome
            .iter()
            .all(|o| matches!(o, CheckpointOutcome::Applied(2)))
        {
            winners += 1;
        } else {
            assert!(outcome.iter().all(|o| *o == CheckpointOutcome::Stale), "{outcome:?}");
        }
    }
    assert_eq!(winners, 1);
}

#[tokio::test]
async fn memory_multi_character_writer_is_all_or_nothing_and_canonically_fenced() {
    let bundle = embedded_phase4_bundle().unwrap();
    let store = Arc::new(InMemoryEconomyStore::default());
    let repo = Arc::new(
        InMemoryCharacterRepository::default()
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap()
            .with_economy(store),
    );
    Box::pin(multi_character_contract(repo, &bundle, "MemMulti")).await;
}

#[tokio::test]
async fn postgres_multi_character_writer_is_all_or_nothing_and_canonically_fenced() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    common::pg::insert_account(&pool, ACCOUNT).await;
    let bundle = embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    Box::pin(multi_character_contract(repo, &bundle, "PgMulti")).await;
    let traded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE subject='nightfall.item.traded'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(traded, 3, "one traded fact per applied trade");
}
