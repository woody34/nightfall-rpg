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
