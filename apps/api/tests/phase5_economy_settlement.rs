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
    clippy::too_many_arguments
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
    work.projection.ledger = Vec::new();
    work.projection.receipts = Vec::new();
    work.projection.materializations = Vec::new();
    let mut ground = picked(0);
    ground.location = ItemLocation::Ground;
    let mut ledger = vec![row(
        "ground",
        GROUND,
        "transfer_out",
        1,
        80,
        0,
        serde_json::json!({"before": ground, "after": null, "transfer": {"link": PICKUP_LINK, "mode": "identity"}, "receipt": "receipt:pick_up_item:00000000-0000-0000-0000-000000007777"}),
    )];
    if character_rows {
        ledger.push(row(
            "character",
            character,
            "transfer_in",
            1,
            80,
            0,
            serde_json::json!({"before": null, "after": picked(2), "transfer": {"link": PICKUP_LINK, "mode": "identity"}, "receipt": "receipt:pick_up_item:00000000-0000-0000-0000-000000007777"}),
        ));
    }
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
    let (next, work2, write) = pickup(&cp, &work, true, "fp");
    let rows = EconomyCharacterRow::of_write(next.character_id, &write).unwrap();
    assert_eq!(rows.len(), 1, "only the character side");
    work2
        .validate_before_with_economy(Some(&before), Some(&bundle.items), &rows)
        .unwrap();
    // Without the character row the new scalar is unledgered (the V6 rule is unchanged).
    assert_eq!(
        constraint(work2.validate_before_with_economy(Some(&before), Some(&bundle.items), &[])),
        "item_checkpoint_unledgered_scalar"
    );
    // The economy row's after must be the settled scalar and revision.
    let mut stale = rows.clone();
    stale[0].after = Some(picked(1));
    assert_eq!(
        constraint(work2.validate_before_with_economy(Some(&before), Some(&bundle.items), &stale)),
        "item_ledger_revision_step"
    );
    // T1: an economy row must hold its canonical position.
    let mut gap = rows.clone();
    gap[0].operation_ordinal = 1;
    assert_eq!(
        constraint(work2.validate_before_with_economy(Some(&before), Some(&bundle.items), &gap)),
        "item_ledger_row_ordinal"
    );
    // A second creation of the same instance is an identity reuse.
    let mut twice = rows.clone();
    twice.push(EconomyCharacterRow {
        operation_ordinal: 1,
        ..rows[0].clone()
    });
    assert!(matches!(
        work2.validate_before_with_economy(Some(&before), Some(&bundle.items), &twice),
        Err(CheckpointError::Constraint(_))
    ));
    // A character row of another character is refused.
    let mut foreign = write.clone();
    foreign.ledger[1].owner_id = Uuid::from_u128(0xF00);
    assert_eq!(
        constraint(EconomyCharacterRow::of_write(next.character_id, &foreign).map(drop)),
        "economy_character_row_foreign"
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
        vec![
            ("ground".into(), "loot".into()),
            ("ground".into(), "transfer_out".into()),
            ("character".into(), "transfer_in".into())
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
    let (cp2, work2) = tick(&cp, &work, 1, 2, 80, rest.clone());
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
        ledger: vec![
            moved("character", owner, "transfer_out", 80, id, Some(&item), None, Some(link_in)),
            moved("warehouse", owner, "transfer_in", 80, id, None, Some(&stored), Some(link_in)),
        ],
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
    let (cp3, work3) = tick(&cp, &work, 2, 3, 81, instances);
    let withdraw = |link: Option<&str>| EconomyCheckpointWrite {
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
        ledger: vec![
            moved("warehouse", owner, "transfer_out", 81, id, Some(&stored), None, Some(link_out)),
            moved("character", owner, "transfer_in", 81, id, None, Some(&back), link),
        ],
        lease_releases: vec![key],
        ..Default::default()
    };
    // Without the B13 link the instance's history makes the before = None row a D67 reuse.
    leases.acquire(&grant(vec![key], 2)).await.unwrap();
    let refused = repo
        .checkpoint_with_economy(&cp3, &[], Some(&work3), &withdraw(None))
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
        repo.checkpoint_with_economy(&cp3, &[], Some(&work3), &withdraw(Some(link_out)))
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
