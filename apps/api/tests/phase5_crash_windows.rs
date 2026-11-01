//! Story 4.8 (design §14.4 "Crash windows"): the system-job and lease rows, incl. the rev 5–7
//! additions, on the memory and the Postgres adapters (the Postgres half needs `DATABASE_URL`;
//! a skipped adapter is not a pass, so the suite runs with the owned stack). The
//! character-checkpoint rows (pickup, party split, deposit with merge, auction bid, mail send)
//! are `commit_crash_windows` in `phase5_economy_settlement.rs`; the epoch reconcile row is
//! `phase5_epoch_reconcile.rs`.
//!
//! Rows here: delivery into a warehouse (merge and insert) with a concurrent deposit refused
//! busy; AuctionSchedule Finish with the escrow destroy around the log ack; racing MailSend
//! busy, then the frozen 240 inbox count; opposite-role mailbox key order; busy re-queue holding
//! nothing and the one-per-tick delivery queue in id order; system-job lease recovery; a forced
//! transient (40P01 on Postgres) in a preflight and in a checkpoint. The flood-gate row (E35) is
//! zone state only and has no adapter half.
//!
//! A crash is modelled the way recovery sees it: the zone took the job (leases held), and either
//! nothing committed (the epoch's leases are released by recovery and the job re-runs) or its
//! checkpoint committed (the commit marker is set and the job never re-runs).
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::doc_markdown,
    clippy::similar_names,
    clippy::expect_used
)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use nightfall_api::application::economy::{
    auction_bid_owner, sets, AuctionTransition, ContainerDelta, DeliveryCompletion, DeliveryKind,
    DeliveryRepository, DeliveryRow, DeltaChange, EconomyCheckpointRepository,
    EconomyCheckpointWrite, EconomyError, LeaseGrant, LeaseKey, LeaseKind, LeaseRepository,
    OwnerLedgerRow, SystemJob,
};
use nightfall_api::application::economy_reads::EconomyReadRepository;
use nightfall_api::application::economy_scheduler::{
    EconomyScheduler, FixtureAuction, HostZone, JobRefused, ScheduledJob, SystemJobPort,
};
use nightfall_api::infrastructure::memory::InMemoryEconomyStore;
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgEconomyRepository,
};
use parking_lot::Mutex;
use sea_orm_migration::MigratorTrait;
use sqlx::PgPool;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0x5000);
const B: Uuid = Uuid::from_u128(0x5001);
const R: Uuid = Uuid::from_u128(0x5002);
const ACCOUNT: Uuid = Uuid::from_u128(0x5100);
const ZONE: (u32, u64) = (1, 1);
const WAREHOUSE_STACK: u64 = 900_001;

trait Adapter:
    LeaseRepository + EconomyReadRepository + DeliveryRepository + EconomyCheckpointRepository + 'static
{
}
impl<
        T: LeaseRepository
            + EconomyReadRepository
            + DeliveryRepository
            + EconomyCheckpointRepository
            + 'static,
    > Adapter for T
{
}

/// The stub zone port: every host runs, enqueued jobs are recorded, refusals on demand.
#[derive(Default)]
struct Zones {
    enqueued: Mutex<Vec<ScheduledJob>>,
    refuse: Mutex<bool>,
}

#[async_trait]
impl SystemJobPort for Zones {
    async fn host(&self, _: &HostZone) -> Option<(u32, u64)> {
        Some(ZONE)
    }
    async fn enqueue(&self, job: ScheduledJob) -> Result<(), JobRefused> {
        if *self.refuse.lock() {
            return Err(JobRefused("stopping".into()));
        }
        self.enqueued.lock().push(job);
        Ok(())
    }
}

fn cols(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn grant(keys: Vec<LeaseKey>, n: u128, account: Option<Uuid>) -> LeaseGrant {
    LeaseGrant {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        account_id: account,
        operation: "seed".into(),
        request_key: Uuid::from_u128(n),
        keys,
    }
}

/// Writes rows through the single writer under a seed lease (E31: never a direct insert).
async fn seed<R: Adapter>(
    repo: &R,
    n: u128,
    deltas: Vec<ContainerDelta>,
    deliveries: Vec<DeliveryRow>,
) {
    let mut keys: Vec<LeaseKey> = deltas.iter().map(|d| d.lease).collect();
    keys.sort();
    keys.dedup();
    if !keys.is_empty() {
        repo.acquire(&grant(keys.clone(), n, None)).await.unwrap();
    }
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 1,
        deltas,
        deliveries,
        lease_releases: keys,
        ..Default::default()
    })
    .await
    .unwrap();
}

fn warehouse_row(owner: Uuid, id: u64, count: u64) -> ContainerDelta {
    ContainerDelta {
        lease: LeaseKey::of_uuid(LeaseKind::Warehouse, owner),
        row_kind: "item_instance".into(),
        row_key: id.to_string(),
        change: DeltaChange::Insert,
        expected_revision: None,
        columns: cols(&[
            ("character_id", serde_json::json!(owner.to_string())),
            ("owner_kind", serde_json::json!("warehouse")),
            ("location", serde_json::json!("warehouse")),
            ("template_id", serde_json::json!(57)),
            ("count", serde_json::json!(count)),
            ("created_tick", serde_json::json!(1)),
        ]),
    }
}

fn award(n: u128, target: Uuid) -> DeliveryRow {
    DeliveryRow {
        id: Uuid::from_u128(n),
        target_character: target,
        kind: DeliveryKind::AuctionAward,
        payload: serde_json::json!({"template_id": 57, "count": 10}),
    }
}

fn completion(job: &ScheduledJob, tick: u64) -> DeliveryCompletion {
    let SystemJob::DeliverPending(id) = job.job else {
        panic!("not a delivery")
    };
    DeliveryCompletion {
        id,
        expected_revision: 0,
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick,
    }
}

fn scheduler<R: Adapter>(
    repo: &Arc<R>,
    zones: &Arc<Zones>,
    schedule: Vec<FixtureAuction>,
) -> EconomyScheduler {
    EconomyScheduler::new(repo.clone(), repo.clone(), repo.clone(), zones.clone(), schedule, 14)
}

/// Rev 5 (E33): DeliverPending into a warehouse that holds a stack of the template (merge) and
/// one that does not (insert), with a concurrent WarehouseDeposit refused CONTAINER_BUSY while
/// the delivery lease is held; then a crash before commit re-runs it, after commit never.
async fn delivery_into_warehouse_merge_and_insert<R: Adapter>(repo: Arc<R>) {
    seed(
        repo.as_ref(),
        1,
        vec![warehouse_row(A, WAREHOUSE_STACK, 5)],
        vec![award(0x10, A), award(0x11, B)],
    )
    .await;
    let zones = Arc::new(Zones::default());
    let mut s = scheduler(&repo, &zones, vec![]);
    let report = s.tick().await.unwrap();
    assert_eq!(report.enqueued.len(), 2, "{report:?}");
    let jobs = zones.enqueued.lock().clone();
    let (merge, insert) = (&jobs[0], &jobs[1]);
    assert_eq!(
        merge.frozen.leases,
        sets::deliver_pending(Uuid::from_u128(0x10), DeliveryKind::AuctionAward, A)
    );
    let stack = merge
        .frozen
        .rows
        .iter()
        .find(|r| r.row_kind == "item_instance")
        .expect("the warehouse pre-image (E33)");
    assert_eq!((stack.row_key.as_str(), stack.revision), ("900001", 0));
    assert!(
        insert
            .frozen
            .rows
            .iter()
            .all(|r| r.row_kind != "item_instance"),
        "B's warehouse is empty"
    );
    // A concurrent deposit into A's warehouse is busy while the delivery holds it.
    let deposit = grant(sets::warehouse(A), 0x77, Some(ACCOUNT));
    assert!(matches!(repo.acquire(&deposit).await, Err(EconomyError::Busy(_))));
    // Crash before the delivery's checkpoint: recovery releases the epoch and re-queues both.
    let released = repo.release_epoch(ZONE.0, ZONE.1).await.unwrap();
    let recovered = s.recover(&released).await.unwrap();
    assert_eq!(recovered.requeued.len(), 2);
    assert!(recovered.committed.is_empty());
    zones.enqueued.lock().clear();
    assert_eq!(s.tick().await.unwrap().enqueued.len(), 2, "re-run after recovery");
    let jobs = zones.enqueued.lock().clone();
    // The host zone commits both: a merge Update (CAS on the frozen revision) and an insert.
    let key = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 20,
        deltas: vec![ContainerDelta {
            lease: key,
            row_kind: "item_instance".into(),
            row_key: WAREHOUSE_STACK.to_string(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: cols(&[
                ("character_id", serde_json::json!(A.to_string())),
                ("owner_kind", serde_json::json!("warehouse")),
                ("location", serde_json::json!("warehouse")),
                ("count", serde_json::json!(15)),
            ]),
        }],
        completions: vec![completion(&jobs[0], 20)],
        lease_releases: jobs[0].frozen.leases.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    let b_key = LeaseKey::of_uuid(LeaseKind::Warehouse, B);
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 20,
        deltas: vec![ContainerDelta {
            lease: b_key,
            ..warehouse_row(B, 900_002, 10)
        }],
        completions: vec![completion(&jobs[1], 20)],
        lease_releases: jobs[1].frozen.leases.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    for job in &jobs {
        assert!(s.marker(job.job).await.unwrap(), "committed marker");
    }
    // A crash after the commit: the marker holds, so recovery reports it and never re-runs it.
    let grants: Vec<LeaseGrant> = jobs
        .iter()
        .map(|j| LeaseGrant {
            zone_id: ZONE.0,
            epoch: ZONE.1,
            account_id: None,
            operation: "deliver_pending".into(),
            request_key: j.job.request_key(),
            keys: j.frozen.leases.clone(),
        })
        .collect();
    let after = s.recover(&grants).await.unwrap();
    assert_eq!(after.committed.len(), 2);
    assert!(after.requeued.is_empty());
    assert_eq!(repo.container_items("warehouse", A).await.unwrap()[0].count, 15, "merged once");
    assert_eq!(
        repo.container_items("warehouse", B).await.unwrap()[0].count,
        10,
        "inserted once"
    );
    // With the delivery lease gone, the deposit proceeds.
    repo.acquire(&deposit).await.unwrap();
}

fn fixture_auction() -> Vec<FixtureAuction> {
    vec![FixtureAuction {
        auction_id: 1,
        instance: 32320,
        round: 0,
        start_unix_ms: 1_000,
        end_unix_ms: 2_000,
        item_index: Some(0),
    }]
}

/// Rev 5: AuctionSchedule{Finish} with the winner's escrow destroyed, crashing before and after
/// the log ack (before: nothing committed, re-run; after: committed marker, never re-run).
async fn auction_finish_around_log_ack<R: Adapter>(repo: Arc<R>) {
    let escrow_owner = auction_bid_owner(1, A);
    let auction_key = LeaseKey {
        kind: LeaseKind::Auction,
        owner: 1,
    };
    seed(
        repo.as_ref(),
        2,
        vec![
            ContainerDelta {
                lease: auction_key,
                row_kind: "item_auction".into(),
                row_key: "1".into(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: cols(&[
                    ("instance_npc", serde_json::json!(32320)),
                    ("item_index", serde_json::json!(0)),
                    ("state", serde_json::json!(1)),
                    ("start_unix_ms", serde_json::json!(1_000)),
                    ("end_unix_ms", serde_json::json!(2_000)),
                ]),
            },
            ContainerDelta {
                lease: auction_key,
                row_kind: "item_auction_bid".into(),
                row_key: format!("1:{A}"),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: cols(&[
                    ("auction_id", serde_json::json!(1)),
                    ("character_id", serde_json::json!(A.to_string())),
                    ("last_bid", serde_json::json!(150_000)),
                ]),
            },
            ContainerDelta {
                lease: auction_key,
                row_kind: "item_instance".into(),
                row_key: "900050".into(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: cols(&[
                    ("owner_kind", serde_json::json!("auction_bid")),
                    ("location", serde_json::json!("auction_bid")),
                    ("container_id", serde_json::json!(escrow_owner.to_string())),
                    ("template_id", serde_json::json!(57)),
                    ("count", serde_json::json!(150_000)),
                    ("created_tick", serde_json::json!(1)),
                ]),
            },
        ],
        vec![],
    )
    .await;
    // Make A the winner (the Start/Bid writer's job), revision 1.
    repo.acquire(&grant(vec![auction_key], 3, None))
        .await
        .unwrap();
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 2,
        deltas: vec![ContainerDelta {
            lease: auction_key,
            row_kind: "item_auction".into(),
            row_key: "1".into(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: cols(&[
                ("instance_npc", serde_json::json!(32320)),
                ("item_index", serde_json::json!(0)),
                ("state", serde_json::json!(1)),
                ("highest_bidder", serde_json::json!(A.to_string())),
                ("last_bidder", serde_json::json!(A.to_string())),
                ("highest_bid", serde_json::json!(150_000)),
            ]),
        }],
        lease_releases: vec![auction_key],
        ..Default::default()
    })
    .await
    .unwrap();
    let zones = Arc::new(Zones::default());
    let mut s = scheduler(&repo, &zones, fixture_auction());
    let first = s.tick().await.unwrap();
    assert!(
        first
            .enqueued
            .contains(&SystemJob::AuctionSchedule(1, AuctionTransition::Finish)),
        "{first:?}"
    );
    let finish = zones
        .enqueued
        .lock()
        .iter()
        .find(|j| j.job == SystemJob::AuctionSchedule(1, AuctionTransition::Finish))
        .cloned()
        .unwrap();
    assert!(finish.frozen.leases.contains(&auction_key));
    assert!(
        finish.frozen.rows.iter().any(|r| r.row_key == "900050"),
        "the escrow is frozen (E33)"
    );
    // Crash before the log ack: nothing committed; recovery re-queues.
    let released = repo.release_epoch(ZONE.0, ZONE.1).await.unwrap();
    let before = s.recover(&released).await.unwrap();
    assert!(before.requeued.contains(&finish.job) && !before.committed.contains(&finish.job));
    zones.enqueued.lock().clear();
    s.tick().await.unwrap();
    let finish = zones
        .enqueued
        .lock()
        .iter()
        .find(|j| j.job == finish.job)
        .cloned()
        .unwrap();
    // Acked and committed: state FINISHED, the winning escrow destroyed (sink), the award delivery.
    let delivery = finish
        .frozen
        .leases
        .iter()
        .find(|k| k.kind == LeaseKind::Delivery)
        .unwrap()
        .owner;
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 30,
        deltas: vec![
            ContainerDelta {
                lease: auction_key,
                row_kind: "item_auction".into(),
                row_key: "1".into(),
                change: DeltaChange::Update,
                expected_revision: Some(1),
                columns: cols(&[
                    ("instance_npc", serde_json::json!(32320)),
                    ("item_index", serde_json::json!(0)),
                    ("state", serde_json::json!(2)),
                    ("highest_bidder", serde_json::json!(A.to_string())),
                    ("last_bidder", serde_json::json!(A.to_string())),
                    ("highest_bid", serde_json::json!(150_000)),
                ]),
            },
            ContainerDelta {
                lease: auction_key,
                row_kind: "item_instance".into(),
                row_key: "900050".into(),
                change: DeltaChange::Delete,
                expected_revision: Some(0),
                columns: serde_json::Map::new(),
            },
        ],
        deliveries: vec![DeliveryRow { id: Uuid::from_u128(delivery), target_character: A, kind: DeliveryKind::AuctionAward, payload: serde_json::json!({"template_id": 9901, "count": 1}) }],
        ledger: vec![OwnerLedgerRow {
            owner_kind: "auction_bid".into(),
            owner_id: escrow_owner,
            zone_id: ZONE.0,
            epoch: ZONE.1,
            tick: 30,
            revision: 1,
            op: "destroy".into(),
            instance_id: 900_050,
            operation_ordinal: 0,
            delta: serde_json::json!({"before": {"template_id": 57, "count": 150_000}, "after": null, "causes": ["auction_win:1"]}),
            receipt_ref: None,
        }],
        lease_releases: finish.frozen.leases.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    // Crash after the log ack: the marker holds; never re-run.
    let grant = LeaseGrant {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        account_id: None,
        operation: "auction_schedule".into(),
        request_key: finish.job.request_key(),
        keys: finish.frozen.leases.clone(),
    };
    let after = s.recover(&[grant]).await.unwrap();
    assert_eq!(after.committed, vec![finish.job]);
    assert!(repo
        .container_items("auction_bid", A)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(repo.auctions(32320).await.unwrap()[0].state, 2);
    assert!(
        repo.delivery(Uuid::from_u128(delivery))
            .await
            .unwrap()
            .unwrap()
            .pending,
        "the award waits for its target"
    );
}

/// Rev 5 (DR5 nit 7) and rev 7 (E42): a racing second MailSend to the same receiver is
/// CONTAINER_BUSY while the first holds the receiver's mailbox; on retry the frozen inbox count
/// is 240 (the zone refuses MAILBOX_FULL). Opposite-role sends acquire in one bytewise order.
async fn racing_mail_send_busy_then_full<R: Adapter>(repo: Arc<R>) {
    let first = grant(sets::mail_send(A, R), 0x31, Some(ACCOUNT));
    let second = grant(sets::mail_send(B, R), 0x32, Some(ACCOUNT));
    repo.acquire(&first).await.unwrap();
    let busy = repo.acquire(&second).await.unwrap_err();
    assert!(
        matches!(&busy, EconomyError::Busy(held) if held == &vec![LeaseKey::of_uuid(LeaseKind::Mailbox, R)]),
        "{busy:?}"
    );
    assert!(
        repo.held(&[LeaseKey::of_uuid(LeaseKind::Mailbox, B)])
            .await
            .unwrap()
            .is_empty(),
        "nothing of the refused set is held"
    );
    // The first send commits the receiver's 240th message (239 already there).
    let mut deltas = Vec::new();
    for n in 0..240_u128 {
        deltas.push(ContainerDelta {
            lease: LeaseKey::of_uuid(LeaseKind::Mailbox, R),
            row_kind: "mail_message".into(),
            row_key: Uuid::from_u128(0x9000 + n).to_string(),
            change: DeltaChange::Insert,
            expected_revision: None,
            columns: cols(&[
                ("sender_id", serde_json::json!(A.to_string())),
                ("receiver_id", serde_json::json!(R.to_string())),
                ("subject", serde_json::json!("s")),
                ("body", serde_json::json!("b")),
                ("cod_adena", serde_json::json!(0)),
                ("has_attachments", serde_json::json!(false)),
            ]),
        });
    }
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 5,
        deltas,
        lease_releases: first.keys.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    repo.acquire(&second).await.unwrap();
    assert_eq!(repo.mailbox_counts(R).await.unwrap().0, 240, "the frozen inbox count at retry");
    repo.release(ZONE.0, ZONE.1, &second.keys).await.unwrap();
    // E42: A→B and B→A take the two mailbox keys in the same order.
    assert_eq!(sets::mail_send(A, B), sets::mail_send(B, A));
}

/// Rev 5 (E38) and rev 7 (E41): a system job refused CONTAINER_BUSY re-queues to the next tick
/// holding nothing; three deliveries for one target complete one per tick in id order, each
/// within one cadence of reaching the head (a busy re-queue keeps it at the head).
async fn delivery_queue_order_and_busy_requeue<R: Adapter>(repo: Arc<R>) {
    let cod = |n: u128| DeliveryRow {
        id: Uuid::from_u128(n),
        target_character: A,
        kind: DeliveryKind::CodPayment,
        payload: serde_json::json!({"adena": 5}),
    };
    seed(repo.as_ref(), 4, vec![], vec![cod(0x23), cod(0x21), cod(0x22)]).await;
    let zones = Arc::new(Zones::default());
    let mut s = scheduler(&repo, &zones, vec![]);
    // Someone else holds the head delivery's lease: busy, nothing held, it stays at the head.
    let head = LeaseKey::of_uuid(LeaseKind::Delivery, Uuid::from_u128(0x21));
    repo.acquire(&grant(vec![head], 0x40, Some(ACCOUNT)))
        .await
        .unwrap();
    let busy = s.tick().await.unwrap();
    assert_eq!(busy.busy, vec![SystemJob::DeliverPending(Uuid::from_u128(0x21))]);
    assert!(busy.enqueued.is_empty(), "no later delivery jumps the head");
    repo.release(ZONE.0, ZONE.1, &[head]).await.unwrap();
    // A refused enqueue releases the whole set at once.
    *zones.refuse.lock() = true;
    let refused = s.tick().await.unwrap();
    assert_eq!(refused.waiting, vec![SystemJob::DeliverPending(Uuid::from_u128(0x21))]);
    assert!(repo.held(&[head]).await.unwrap().is_empty());
    *zones.refuse.lock() = false;
    let mut order = Vec::new();
    for tick in 10..13 {
        let report = s.tick().await.unwrap();
        assert_eq!(report.enqueued.len(), 1, "one delivery per target per tick ({tick})");
        let job = zones.enqueued.lock().last().cloned().unwrap();
        repo.checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: ZONE.0,
            epoch: ZONE.1,
            tick,
            completions: vec![completion(&job, tick)],
            lease_releases: job.frozen.leases.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
        s.completed(job.job);
        order.push(job.job);
    }
    assert_eq!(
        order,
        vec![0x21, 0x22, 0x23]
            .into_iter()
            .map(|n| SystemJob::DeliverPending(Uuid::from_u128(n)))
            .collect::<Vec<_>>()
    );
    assert!(repo.pending(A).await.unwrap().is_empty());
}

/// Rev 7 (DR6 nit 10): a crash after a system job's leases are taken but before its marker
/// commits releases the leases and re-enqueues it; a committed job is not re-run.
async fn system_job_lease_recovery<R: Adapter>(repo: Arc<R>) {
    let cod = |n: u128, target| DeliveryRow {
        id: Uuid::from_u128(n),
        target_character: target,
        kind: DeliveryKind::CodPayment,
        payload: serde_json::json!({"adena": 5}),
    };
    seed(repo.as_ref(), 5, vec![], vec![cod(0x51, A), cod(0x52, B)]).await;
    let zones = Arc::new(Zones::default());
    let mut s = scheduler(&repo, &zones, vec![]);
    assert_eq!(s.tick().await.unwrap().enqueued.len(), 2);
    let jobs = zones.enqueued.lock().clone();
    // Job 0x51 committed before the crash; 0x52 did not.
    repo.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 40,
        completions: vec![completion(&jobs[0], 40)],
        lease_releases: jobs[0].frozen.leases.clone(),
        ..Default::default()
    })
    .await
    .unwrap();
    let mut released = repo.release_epoch(ZONE.0, ZONE.1).await.unwrap();
    released.push(LeaseGrant {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        account_id: None,
        operation: "deliver_pending".into(),
        request_key: jobs[0].job.request_key(),
        keys: jobs[0].frozen.leases.clone(),
    });
    let report = s.recover(&released).await.unwrap();
    assert_eq!(report.committed, vec![jobs[0].job]);
    assert_eq!(report.requeued, vec![jobs[1].job]);
    assert!(
        repo.held(&jobs[1].frozen.leases).await.unwrap().is_empty(),
        "its leases are released"
    );
    zones.enqueued.lock().clear();
    assert_eq!(
        s.tick().await.unwrap().enqueued,
        vec![jobs[1].job],
        "only the uncommitted job re-runs"
    );
}

async fn contract<R: Adapter>(
    make: impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Arc<R>>>>,
) {
    delivery_into_warehouse_merge_and_insert(make().await).await;
    auction_finish_around_log_ack(make().await).await;
    racing_mail_send_busy_then_full(make().await).await;
    delivery_queue_order_and_busy_requeue(make().await).await;
    system_job_lease_recovery(make().await).await;
}

#[tokio::test]
async fn memory_system_job_crash_windows() {
    contract(|| Box::pin(async { Arc::new(InMemoryEconomyStore::default()) })).await;
}

/// Rev 5 (E39): a forced transient (40P01 on Postgres, injected on memory) in a preflight and
/// in a checkpoint is retried as transient, never a fence, and leaves nothing behind.
#[tokio::test]
async fn memory_forced_transient_in_preflight_and_checkpoint_is_retryable() {
    let repo = InMemoryEconomyStore::default();
    let keys = sets::warehouse(A);
    repo.fail_next_transient(1);
    assert!(matches!(
        repo.acquire(&grant(keys.clone(), 0x61, Some(ACCOUNT)))
            .await,
        Err(EconomyError::Transient(_))
    ));
    assert!(repo.held(&keys).await.unwrap().is_empty());
    repo.acquire(&grant(keys.clone(), 0x61, Some(ACCOUNT)))
        .await
        .unwrap();
    let write = EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 3,
        deltas: vec![warehouse_row(A, 900_070, 1)],
        lease_releases: keys.clone(),
        ..Default::default()
    };
    repo.fail_next_transient(1);
    let failed = repo.checkpoint_economy(&write).await.unwrap_err();
    assert!(failed.is_retryable(), "{failed:?}");
    assert!(
        repo.container_items("warehouse", A)
            .await
            .unwrap()
            .is_empty(),
        "nothing written"
    );
    repo.checkpoint_economy(&write).await.unwrap();
    assert_eq!(repo.container_items("warehouse", A).await.unwrap().len(), 1);
}

async fn pg() -> Option<(PgPool, Arc<PgEconomyRepository>)> {
    let pool = common::pg::empty_schema_pool().await?;
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    sqlx::query("INSERT INTO accounts(id) VALUES($1)")
        .bind(ACCOUNT)
        .execute(&pool)
        .await
        .unwrap();
    for (id, name) in [(A, "CrashA"), (B, "CrashB"), (R, "CrashR")] {
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$3,'human',1,1,1,1,1,1,0,0,'human_fighter',true)")
            .bind(id).bind(ACCOUNT).bind(name).execute(&pool).await.unwrap();
    }
    Some((pool, Arc::new(PgEconomyRepository::new(db))))
}

#[tokio::test]
async fn postgres_system_job_crash_windows() {
    if common::pg::empty_schema_pool().await.is_none() {
        return;
    }
    contract(|| Box::pin(async { pg().await.unwrap().1 })).await;
}

/// Rev 5 (E39) on Postgres: a real 40P01 in a preflight (lease acquire) and in a checkpoint.
/// The adapter is the deadlock victim (it waits first), classifies it Transient, holds nothing,
/// and the retry succeeds.
#[tokio::test]
async fn postgres_forced_40p01_in_preflight_and_checkpoint_is_retryable() {
    let Some((pool, repo)) = pg().await else {
        return;
    };
    let k1 = LeaseKey::of_uuid(LeaseKind::Freight, A);
    let k2 = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
    let owner = |k: LeaseKey| format!("{:039}", k.owner);
    // Preflight: the other session holds k2 (uncommitted); the adapter inserts k1 and waits on
    // k2; the other session then waits on k1. The adapter waited first, so its deadlock check
    // fires first and it is the victim.
    let other = pool.clone();
    let (k1o, k2o) = (owner(k1), owner(k2));
    let blocker = tokio::spawn(async move {
        let mut tx = other.begin().await.unwrap();
        sqlx::query("INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,operation,request_key) VALUES('warehouse',$1,1,1,'blocker',$2)")
            .bind(&k2o).bind(Uuid::from_u128(0xB1)).execute(&mut *tx).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let r = sqlx::query("INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,operation,request_key) VALUES('freight',$1,1,1,'blocker',$2) ON CONFLICT DO NOTHING")
            .bind(&k1o).bind(Uuid::from_u128(0xB1)).execute(&mut *tx).await;
        let _ = tx.rollback().await;
        r.err()
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut keys = vec![k1, k2];
    keys.sort();
    let set = grant(keys.clone(), 0x71, Some(ACCOUNT));
    let first = repo.acquire(&set).await;
    let other_error = blocker.await.unwrap();
    assert!(
        matches!(&first, Err(EconomyError::Transient(m)) if m.contains("deadlock")),
        "{first:?} / other {other_error:?}"
    );
    assert!(
        repo.held(&keys).await.unwrap().is_empty(),
        "nothing held after the victim's rollback"
    );
    repo.acquire(&set).await.unwrap();
    // Checkpoint: the adapter updates the warehouse row, then deletes its lease rows; the other
    // session locks the k1 lease row, then wants the warehouse row.
    seed(repo.as_ref(), 0x72, vec![warehouse_row(B, 900_080, 1)], vec![]).await;
    let row_lease = LeaseKey::of_uuid(LeaseKind::Warehouse, B);
    repo.acquire(&grant(vec![row_lease], 0x73, Some(ACCOUNT)))
        .await
        .unwrap();
    let write = EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 9,
        deltas: vec![ContainerDelta {
            lease: row_lease,
            row_kind: "item_instance".into(),
            row_key: "900080".into(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: cols(&[
                ("character_id", serde_json::json!(B.to_string())),
                ("owner_kind", serde_json::json!("warehouse")),
                ("location", serde_json::json!("warehouse")),
                ("count", serde_json::json!(2)),
            ]),
        }],
        lease_releases: vec![row_lease],
        ..Default::default()
    };
    let other = pool.clone();
    let lease_owner = owner(row_lease);
    let blocker = tokio::spawn(async move {
        let mut tx = other.begin().await.unwrap();
        sqlx::query(
            "SELECT 1 FROM economy_leases WHERE owner_kind='warehouse' AND owner_id=$1 FOR UPDATE",
        )
        .bind(&lease_owner)
        .execute(&mut *tx)
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let r = sqlx::query("UPDATE item_instances SET count=count WHERE id=900080")
            .execute(&mut *tx)
            .await;
        let _ = tx.rollback().await;
        r.err()
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let failed = repo.checkpoint_economy(&write).await;
    let _ = blocker.await.unwrap();
    assert!(
        matches!(&failed, Err(EconomyError::Transient(m)) if m.contains("deadlock")),
        "{failed:?}"
    );
    assert_eq!(repo.container_items("warehouse", B).await.unwrap()[0].count, 1, "rolled back");
    assert_eq!(
        repo.held(&[row_lease]).await.unwrap().len(),
        1,
        "a transient is not a fence: the lease is still held"
    );
    repo.checkpoint_economy(&write).await.unwrap();
    assert_eq!(repo.container_items("warehouse", B).await.unwrap()[0].count, 2);
}
