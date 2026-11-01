//! Story 4.7 (design §11.6): epoch close and `EpochReconcile`, API side, over an
//! abandoned-epoch fixture on the memory and Postgres adapters (the Postgres half runs only with
//! `DATABASE_URL`). Done When: exactly one terminal row per live ground/refund instance, and
//! every lease of the abandoned epochs released.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::panic
)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use nightfall_api::application::economy::{
    EconomyCheckpointRepository, EconomyCheckpointWrite, LeaseGrant, LeaseKey, LeaseKind,
    LeaseRepository, OwnerLedgerRow, SystemJob,
};
use nightfall_api::application::economy_epoch::{
    frozen_manifest, void_rows, EpochReconciler, ReconcileReport,
};
use nightfall_api::application::economy_reads::EconomyReadRepository;
use nightfall_api::application::economy_scheduler::{
    HostZone, JobRefused, ScheduledJob, SystemJobPort,
};
use nightfall_api::domain::zone::phase5_command::Phase5Command;
use nightfall_api::infrastructure::memory::InMemoryEconomyStore;
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgEconomyRepository,
};
use parking_lot::Mutex;
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

const OWNER: Uuid = Uuid::from_u128(0x5000);
const ACCOUNT: Uuid = Uuid::from_u128(0x5100);

#[derive(Default)]
struct Port {
    jobs: Mutex<Vec<ScheduledJob>>,
}

#[async_trait]
impl SystemJobPort for Port {
    async fn host(&self, _: &HostZone) -> Option<(u32, u64)> {
        Some((1, 3))
    }
    async fn enqueue(&self, job: ScheduledJob) -> Result<(), JobRefused> {
        self.jobs.lock().push(job);
        Ok(())
    }
}

fn item(instance: u64, count: u64, location: &str) -> serde_json::Value {
    serde_json::json!({"id": instance, "template_id": 57, "count": count, "location": location, "created_tick": 1, "revision": 0})
}

fn row(
    owner_kind: &str,
    owner: u128,
    epoch: u64,
    tick: u64,
    op: &str,
    instance: u64,
    delta: serde_json::Value,
) -> OwnerLedgerRow {
    OwnerLedgerRow {
        owner_kind: owner_kind.into(),
        owner_id: if owner_kind == "refund" {
            OWNER
        } else {
            Uuid::from_u128(owner)
        },
        zone_id: 1,
        epoch,
        tick,
        revision: 0,
        op: op.into(),
        instance_id: instance,
        operation_ordinal: 0,
        delta,
        receipt_ref: None,
    }
}

fn born(owner_kind: &str, owner: u128, epoch: u64, tick: u64, instance: u64) -> OwnerLedgerRow {
    let (op, location) = if owner_kind == "refund" {
        ("transfer_in", "refund")
    } else {
        ("loot", "ground")
    };
    row(
        owner_kind,
        owner,
        epoch,
        tick,
        op,
        instance,
        serde_json::json!({"before": null, "after": item(instance, 3, location), "causes": [format!("loot:npc@{tick}")]}),
    )
}

fn ended(owner: u128, epoch: u64, tick: u64, instance: u64, cause: &str) -> OwnerLedgerRow {
    row(
        "ground",
        owner,
        epoch,
        tick,
        "destroy",
        instance,
        serde_json::json!({"before": item(instance, 3, "ground"), "after": null, "causes": [cause]}),
    )
}

fn write(epoch: u64, tick: u64, ledger: Vec<OwnerLedgerRow>) -> EconomyCheckpointWrite {
    EconomyCheckpointWrite {
        zone_id: 1,
        epoch,
        tick,
        ledger,
        ..Default::default()
    }
}

/// Epoch 1 crashed with ground 101 and refund 103 live (102 picked up, 104 decayed); epoch 2
/// crashed after a graceful-close row for 105 but with 106 live. Both hold a lease.
async fn fixture<R: EconomyCheckpointRepository + LeaseRepository>(repo: &R) {
    for w in [
        write(
            1,
            10,
            vec![
                born("ground", 1, 1, 10, 101),
                born("ground", 2, 1, 10, 102),
                born("refund", 0, 1, 10, 103),
                born("ground", 4, 1, 10, 104),
            ],
        ),
        write(
            1,
            11,
            vec![row(
                "ground",
                2,
                1,
                11,
                "transfer_out",
                102,
                serde_json::json!({"before": item(102, 3, "ground"), "after": null, "receipt": "receipt:pick_up_item:k"}),
            )],
        ),
        write(1, 12, vec![ended(4, 1, 12, 104, "decay:12")]),
        write(2, 5, vec![born("ground", 5, 2, 5, 105), born("ground", 6, 2, 5, 106)]),
        write(2, 6, vec![ended(5, 2, 6, 105, "epoch_close:2")]),
    ] {
        repo.checkpoint_economy(&w).await.unwrap();
    }
    for (epoch, n) in [(1, 1_u128), (2, 2)] {
        repo.acquire(&LeaseGrant {
            zone_id: 1,
            epoch,
            account_id: Some(ACCOUNT),
            operation: "warehouse_deposit".into(),
            request_key: Uuid::from_u128(n),
            keys: vec![LeaseKey {
                kind: LeaseKind::Warehouse,
                owner: n,
            }],
        })
        .await
        .unwrap();
    }
}

async fn contract<R>(repo: Arc<R>)
where
    R: EconomyCheckpointRepository + LeaseRepository + EconomyReadRepository + 'static,
{
    fixture(repo.as_ref()).await;
    let port = Arc::new(Port::default());
    let reconciler = EpochReconciler::new(repo.clone(), repo.clone(), port.clone());
    let report = reconciler.before_admission(1, 3).await.unwrap();
    assert_eq!(report.reconciled, vec![(1, 2), (2, 1)], "epoch 1: 101 and 103; epoch 2: 106");
    assert_eq!(report.released.len(), 2, "both abandoned epochs' leases");
    assert!(repo.lease_epochs(1, 3).await.unwrap().is_empty());
    let jobs = port.jobs.lock().clone();
    assert_eq!(
        jobs.iter().map(|j| j.job).collect::<Vec<_>>(),
        vec![
            SystemJob::EpochReconcile(1, 1),
            SystemJob::EpochReconcile(1, 2)
        ]
    );
    for job in &jobs {
        assert_eq!(job.zone, (1, 3), "the next epoch hosts the reconcile");
        assert!(job.frozen.leases.is_empty(), "EpochReconcile holds no lease");
        let Phase5Command::EpochReconcile {
            request_key,
            abandoned_epoch,
        } = job.command
        else {
            panic!("not a reconcile")
        };
        assert_eq!(request_key, job.job.request_key());
        // The host zone writes one destroy row per manifest entry (here: the reference rows).
        let manifest = frozen_manifest(&job.frozen).unwrap();
        let rows = void_rows(1, 3, 1, abandoned_epoch, &manifest);
        assert!(rows.iter().all(|r| r.op == "destroy"
            && r.delta["after"].is_null()
            && r.delta["causes"][0] == format!("epoch_void:{abandoned_epoch}")));
        repo.checkpoint_economy(&write(3, 1, rows)).await.unwrap();
    }
    let instances: Vec<u64> = jobs
        .iter()
        .flat_map(|j| frozen_manifest(&j.frozen).unwrap())
        .map(|c| c.instance)
        .collect();
    assert_eq!(instances, vec![101, 103, 106], "exactly the live instances, once each");
    assert!(
        repo.open_ground_refund(1, 4).await.unwrap().is_empty(),
        "every live instance has its terminal row"
    );
    // A second admission finds nothing to do; a graceful close releases its own epoch.
    assert_eq!(reconciler.before_admission(1, 3).await.unwrap(), ReconcileReport::default());
    repo.acquire(&LeaseGrant {
        zone_id: 1,
        epoch: 3,
        account_id: Some(ACCOUNT),
        operation: "warehouse_deposit".into(),
        request_key: Uuid::from_u128(3),
        keys: vec![LeaseKey {
            kind: LeaseKind::Warehouse,
            owner: 3,
        }],
    })
    .await
    .unwrap();
    assert_eq!(reconciler.close(1, 3).await.unwrap().len(), 1);
    assert!(repo.lease_epochs(1, 4).await.unwrap().is_empty());
}

#[tokio::test]
async fn memory_abandoned_epochs_get_one_terminal_row_per_live_instance() {
    contract(Arc::new(InMemoryEconomyStore::default())).await;
}

#[tokio::test]
async fn postgres_abandoned_epochs_get_one_terminal_row_per_live_instance() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    common::pg::insert_account(&pool, ACCOUNT).await;
    contract(Arc::new(PgEconomyRepository::new(db))).await;
    let voided: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM item_ledger WHERE delta->'causes'->>0 LIKE 'epoch_void:%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(voided, 3);
}

/// A fenced V7 snapshot of zone 1 epoch 1 at tick 11 with one ground item (instance `item`).
fn fenced_snapshot(item: u64) -> (nightfall_api::domain::zone::ZoneSnapshot, Uuid) {
    use nightfall_api::domain::zone::*;
    use nightfall_api::infrastructure::{class_data, economy_data, item_data, rules_data};
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let seed = ZoneSeed {
        zone: ZoneId(1),
        epoch: 1,
    };
    let state = ZoneState::new_phase5(
        seed,
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        class_data::load_classes(&class_data::ClassSource::embedded())
            .unwrap()
            .registry,
        (*bundle.skills).clone(),
        bundle.items.clone(),
        ItemInstanceRange::new(1, 1000, 2000).unwrap(),
        economy_data::embedded_phase5_rules().unwrap(),
    )
    .unwrap();
    let mut snapshot = state.snapshot();
    snapshot.tick = Tick(11);
    let instance = ItemInstance {
        creation_origin: Some(ItemCreationOrigin {
            zone: ZoneId(1),
            epoch: 1,
        }),
        id: ItemInstanceId::new(item).unwrap(),
        template_id: 57,
        count: 3,
        location: ItemLocation::Ground,
        slot: None,
        enchant: 0,
        augment_id: None,
        charges: 0,
        created_tick: Tick(1),
        revision: 0,
    };
    let entity = GroundItem::entity_id(seed.zone, seed.epoch, instance.id, Tick(10));
    let ground = GroundItem {
        entity,
        item: instance,
        pos: Vec2Fixed::from_tiles(10, 10),
        dropped_tick: Tick(10),
        source: GroundSource::NpcLoot {
            npc: EntityId::from_uuid(Uuid::from_u128(900)),
        },
        loot_owner: None,
        drop_protection: None,
        decay_due: Some(Tick(6010)),
    };
    snapshot
        .phase5
        .as_mut()
        .unwrap()
        .ground
        .insert(entity, ground);
    (snapshot, entity.as_uuid())
}

async fn store_with(
    snapshot: &nightfall_api::domain::zone::ZoneSnapshot,
) -> Arc<nightfall_api::infrastructure::eventlog::InMemoryZoneSnapshotStore> {
    use nightfall_api::application::replay_log::{
        encode_snapshot, ZoneSnapshotRow, ZoneSnapshotStore,
    };
    let store =
        Arc::new(nightfall_api::infrastructure::eventlog::InMemoryZoneSnapshotStore::default());
    let row = ZoneSnapshotRow {
        zone: nightfall_api::domain::zone::ZoneId(1),
        epoch: 1,
        snapshot: encode_snapshot(snapshot).unwrap(),
        snapshot_seq: nightfall_api::application::replay_log::Seq(1),
        first_seq: None,
        time_origin_ms: 0,
        build_id: String::new(),
        config_hash: String::new(),
        schema_version: snapshot.meta.schema_version,
    };
    store.insert(&row).await.unwrap();
    store
}

/// P5-D23: a fenced ground instance unknown to the ledger is an invariant failure. Nothing is
/// released and no reconcile runs. A fenced instance that is live in the ledger agrees.
#[tokio::test]
async fn a_fenced_snapshot_that_disagrees_with_the_ledger_fails_closed() {
    let repo = Arc::new(InMemoryEconomyStore::default());
    let (snapshot, entity) = fenced_snapshot(201);
    // The ledger knows instance 201 under its ground entity, live; 202 is absent.
    repo.checkpoint_economy(&write(
        1,
        10,
        vec![OwnerLedgerRow {
            owner_id: entity,
            ..born("ground", 0, 1, 10, 201)
        }],
    ))
    .await
    .unwrap();
    repo.acquire(&LeaseGrant {
        zone_id: 1,
        epoch: 1,
        account_id: Some(ACCOUNT),
        operation: "warehouse_deposit".into(),
        request_key: Uuid::from_u128(9),
        keys: vec![LeaseKey {
            kind: LeaseKind::Warehouse,
            owner: 9,
        }],
    })
    .await
    .unwrap();
    let port = Arc::new(Port::default());
    let agrees = EpochReconciler::new(repo.clone(), repo.clone(), port.clone())
        .with_snapshots(store_with(&snapshot).await);
    let (unknown, _) = fenced_snapshot(202);
    let disagrees = EpochReconciler::new(repo.clone(), repo.clone(), port.clone())
        .with_snapshots(store_with(&unknown).await);
    let refused = disagrees.before_admission(1, 3).await.unwrap_err();
    assert!(
        matches!(&refused, nightfall_api::application::economy::EconomyError::Constraint(n) if n.starts_with("epoch_reconcile_snapshot_mismatch:1:ground:")),
        "{refused:?}"
    );
    assert!(port.jobs.lock().is_empty(), "no reconcile");
    assert_eq!(repo.lease_epochs(1, 3).await.unwrap(), vec![1], "nothing released");
    // An instance whose chain ended before the snapshot tick but is still fenced also disagrees.
    let closed = Arc::new(InMemoryEconomyStore::default());
    closed
        .checkpoint_economy(&write(
            1,
            10,
            vec![OwnerLedgerRow {
                owner_id: entity,
                ..born("ground", 0, 1, 10, 201)
            }],
        ))
        .await
        .unwrap();
    closed
        .checkpoint_economy(&write(
            1,
            11,
            vec![OwnerLedgerRow {
                owner_id: entity,
                ..ended(0, 1, 11, 201, "decay:11")
            }],
        ))
        .await
        .unwrap();
    let stale = EpochReconciler::new(closed.clone(), closed.clone(), port.clone())
        .with_snapshots(store_with(&snapshot).await);
    assert!(stale.before_admission(1, 3).await.is_err());
    // The agreeing snapshot reconciles exactly the live instance.
    let report = agrees.before_admission(1, 3).await.unwrap();
    assert_eq!(report.reconciled, vec![(1, 1)]);
    assert_eq!(report.released.len(), 1);
}
