//! Story 4.5: schedulers and system-job injection against a stub zone port (design §10.0,
//! §10.3–10.5; E31, E38, E41). Memory adapter; the Postgres read side is covered in
//! `phase5_economy_reads`.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::items_after_statements
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use nightfall_api::application::economy::{
    sets, AuctionTransition, ContainerDelta, DeliveryKind, DeliveryRow, DeltaChange,
    EconomyCheckpointWrite, LeaseGrant, LeaseKey, LeaseKind, LeaseRepository, SystemJob,
};
use nightfall_api::application::economy_reads::{AuctionRow, EconomyReadRepository};
use nightfall_api::application::economy_scheduler::{
    auction_transitions, notify_new_mail, EconomyScheduler, FixtureAuction, HostZone, JobRefused,
    MailNotificationView, MailNotifier, ScheduledJob, SystemJobPort,
};
use nightfall_api::domain::zone::phase5_command::Phase5Command;
use nightfall_api::infrastructure::memory::{InMemoryEconomyStore, StoredRow, MEMORY_EPOCH_MS};
use parking_lot::Mutex;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0x5000);
const B: Uuid = Uuid::from_u128(0x5001);
const ZONE: (u32, u64) = (1, 7);
const HOUR: i64 = 3_600_000;

/// The stub zone port: which hosts run, what was enqueued, and optional refusals.
#[derive(Default)]
struct StubZones {
    admitted: Mutex<Vec<Uuid>>,
    economy_host: Mutex<bool>,
    auctioneers: Mutex<Vec<u32>>,
    refuse: Mutex<bool>,
    enqueued: Mutex<Vec<ScheduledJob>>,
}

#[async_trait]
impl SystemJobPort for StubZones {
    async fn host(&self, host: &HostZone) -> Option<(u32, u64)> {
        let running = match host {
            HostZone::Character(c) => self.admitted.lock().contains(c),
            HostZone::EconomyHost => *self.economy_host.lock(),
            HostZone::Auctioneer(i) => self.auctioneers.lock().contains(i),
        };
        running.then_some(ZONE)
    }
    async fn enqueue(&self, job: ScheduledJob) -> Result<(), JobRefused> {
        if *self.refuse.lock() {
            return Err(JobRefused("stopping".into()));
        }
        self.enqueued.lock().push(job);
        Ok(())
    }
}

fn delivery(n: u128, target: Uuid, kind: DeliveryKind) -> DeliveryRow {
    DeliveryRow {
        id: Uuid::from_u128(n),
        target_character: target,
        kind,
        payload: serde_json::json!({"adena": 5}),
    }
}

fn scheduler(
    store: &Arc<InMemoryEconomyStore>,
    zones: &Arc<StubZones>,
    schedule: Vec<FixtureAuction>,
) -> EconomyScheduler {
    EconomyScheduler::new(store.clone(), store.clone(), store.clone(), zones.clone(), schedule, 14)
}

/// A checkpoint that completes a delivery, as the host zone's commit would.
async fn commit_delivery(store: &InMemoryEconomyStore, job: &ScheduledJob) {
    use nightfall_api::application::economy::{DeliveryCompletion, EconomyCheckpointRepository};
    let SystemJob::DeliverPending(id) = job.job else {
        panic!("not a delivery")
    };
    let write = EconomyCheckpointWrite {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        tick: 10,
        completions: vec![DeliveryCompletion {
            id,
            expected_revision: 0,
            zone_id: ZONE.0,
            epoch: ZONE.1,
            tick: 10,
        }],
        lease_releases: job.frozen.leases.clone(),
        ..Default::default()
    };
    store.checkpoint_economy(&write).await.unwrap();
}

#[tokio::test]
async fn deliveries_run_one_per_target_per_tick_lowest_id_first() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    for (n, target) in [(3, A), (1, A), (2, A), (9, B)] {
        store.put_delivery(delivery(n, target, DeliveryKind::CodPayment));
    }
    let mut s = scheduler(&store, &zones, vec![]);
    // Not admitted anywhere: the deliveries wait, nothing is held.
    let idle = s.tick().await.unwrap();
    assert_eq!(idle.waiting.len(), 2);
    assert!(store
        .held(&[LeaseKey::of_uuid(LeaseKind::Delivery, Uuid::from_u128(1))])
        .await
        .unwrap()
        .is_empty());
    zones.admitted.lock().extend([A, B]);
    let first = s.tick().await.unwrap();
    assert_eq!(
        first.enqueued,
        vec![
            SystemJob::DeliverPending(Uuid::from_u128(1)),
            SystemJob::DeliverPending(Uuid::from_u128(9))
        ]
    );
    // E41: while A's head is in flight no second delivery of A is enqueued.
    let second = s.tick().await.unwrap();
    assert!(second.enqueued.is_empty(), "{second:?}");
    let job = zones.enqueued.lock()[0].clone();
    assert!(
        matches!(&job.command, Phase5Command::DeliverPending { delivery_id, .. } if *delivery_id == Uuid::from_u128(1))
    );
    assert_eq!(
        job.frozen.leases,
        vec![LeaseKey::of_uuid(LeaseKind::Delivery, Uuid::from_u128(1))]
    );
    assert!(
        job.frozen
            .rows
            .iter()
            .any(|r| r.row_kind == "economy_delivery"),
        "the delivery row is frozen"
    );
    commit_delivery(&store, &job).await;
    s.completed(job.job);
    let third = s.tick().await.unwrap();
    assert_eq!(
        third.enqueued,
        vec![SystemJob::DeliverPending(Uuid::from_u128(2))],
        "the next head, one per tick"
    );
}

#[tokio::test]
async fn a_busy_preflight_holds_nothing_and_requeues_to_the_next_tick() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    zones.admitted.lock().push(A);
    store.put_delivery(delivery(1, A, DeliveryKind::MailExpiryReturn));
    // A player intent holds A's warehouse (DeliverPending for a warehouse destination needs it).
    let player = LeaseGrant {
        zone_id: 1,
        epoch: 7,
        account_id: Some(A),
        operation: "warehouse_deposit".into(),
        request_key: Uuid::from_u128(77),
        keys: sets::warehouse(A),
    };
    store.acquire(&player).await.unwrap();
    let mut s = scheduler(&store, &zones, vec![]);
    let busy = s.tick().await.unwrap();
    assert_eq!(busy.busy, vec![SystemJob::DeliverPending(Uuid::from_u128(1))]);
    assert!(
        store
            .held(&[LeaseKey::of_uuid(LeaseKind::Delivery, Uuid::from_u128(1))])
            .await
            .unwrap()
            .is_empty(),
        "all-or-nothing: the free delivery key was not taken"
    );
    assert!(s.in_flight().is_empty());
    store.release(1, 7, &player.keys).await.unwrap();
    let retried = s.tick().await.unwrap();
    assert_eq!(retried.enqueued, vec![SystemJob::DeliverPending(Uuid::from_u128(1))]);
    let job = zones.enqueued.lock()[0].clone();
    assert_eq!(
        job.frozen.leases,
        sets::deliver_pending(Uuid::from_u128(1), DeliveryKind::MailExpiryReturn, A)
    );
    // A host that refuses the enqueue gets nothing held either.
    let store2 = Arc::new(InMemoryEconomyStore::default());
    let zones2 = Arc::new(StubZones::default());
    zones2.admitted.lock().push(A);
    *zones2.refuse.lock() = true;
    store2.put_delivery(delivery(5, A, DeliveryKind::CodPayment));
    let mut s2 = scheduler(&store2, &zones2, vec![]);
    assert_eq!(s2.tick().await.unwrap().waiting.len(), 1);
    assert!(store2
        .held(&[LeaseKey::of_uuid(LeaseKind::Delivery, Uuid::from_u128(5))])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_admission_slot_takes_only_the_lowest_id_delivery_and_never_blocks() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    store.put_delivery(delivery(4, A, DeliveryKind::AuctionAward));
    store.put_delivery(delivery(2, A, DeliveryKind::AuctionAward));
    let mut s = scheduler(&store, &zones, vec![]);
    let slot = s.admission_delivery(A, ZONE).await.unwrap().unwrap();
    assert_eq!(slot.job, SystemJob::DeliverPending(Uuid::from_u128(2)));
    assert!(s.admission_delivery(A, ZONE).await.unwrap().is_none(), "one in the slot");
    // A busy head does not block admission: the slot is simply empty.
    let mut fresh = scheduler(&store, &zones, vec![]);
    assert!(fresh.admission_delivery(A, ZONE).await.unwrap().is_none());
}

#[tokio::test]
async fn mail_expiry_goes_to_the_economy_host_with_its_delivery_and_marker() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    let message = Uuid::from_u128(0x7100);
    store.put_row(
        "mail_message",
        &message.to_string(),
        StoredRow {
            revision: 0,
            columns: [
                ("sender_id", serde_json::json!(A.to_string())),
                ("receiver_id", serde_json::json!(B.to_string())),
                ("subject", serde_json::json!("paycheck")),
                ("body", serde_json::json!("b")),
                ("cod_adena", serde_json::json!(0)),
                ("has_attachments", serde_json::json!(true)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        },
    );
    store.set_mail_times(message, MEMORY_EPOCH_MS - 400 * HOUR, MEMORY_EPOCH_MS - 40 * HOUR);
    let mut s = scheduler(&store, &zones, vec![]);
    assert_eq!(
        s.tick().await.unwrap().waiting,
        vec![SystemJob::MailExpire(message)],
        "no economy host: wait"
    );
    *zones.economy_host.lock() = true;
    let report = s.tick().await.unwrap();
    assert_eq!(report.enqueued, vec![SystemJob::MailExpire(message)]);
    let job = zones.enqueued.lock()[0].clone();
    let delivery = Uuid::new_v5(
        &nightfall_api::application::economy::NS_DELIVERY,
        format!("{message}:expiry").as_bytes(),
    );
    assert_eq!(job.frozen.leases, sets::mail_expire(message, A, B, Some(delivery)));
    assert_eq!(job.frozen.decisions["delivery_id"], serde_json::json!(delivery.to_string()));
    assert!(
        job.frozen
            .rows
            .iter()
            .all(|r| !r.columns.contains_key("expires_at")),
        "no DB timestamp is frozen (E12)"
    );
    // Commit marker: the row is still there, so after a fenced release the job is re-enqueued.
    let released = store.release_epoch(ZONE.0, ZONE.1).await.unwrap();
    let recovery = s.recover(&released).await.unwrap();
    assert_eq!(recovery.requeued, vec![SystemJob::MailExpire(message)]);
    assert_eq!(
        s.tick().await.unwrap().enqueued,
        vec![SystemJob::MailExpire(message)],
        "re-enqueued"
    );
    // Committed (row hard-deleted by the zone checkpoint): the marker is present.
    use nightfall_api::application::economy::EconomyCheckpointRepository;
    let job = zones.enqueued.lock().last().cloned().unwrap();
    store
        .checkpoint_economy(&EconomyCheckpointWrite {
            zone_id: ZONE.0,
            epoch: ZONE.1,
            tick: 20,
            deltas: vec![ContainerDelta {
                lease: LeaseKey::of_uuid(LeaseKind::Mail, message),
                row_kind: "mail_message".into(),
                row_key: message.to_string(),
                change: DeltaChange::Delete,
                expected_revision: Some(0),
                columns: serde_json::Map::new(),
            }],
            lease_releases: job.frozen.leases.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(s.marker(SystemJob::MailExpire(message)).await.unwrap());
}

#[tokio::test]
async fn recovery_distinguishes_committed_from_uncommitted_deliveries() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    zones.admitted.lock().extend([A, B]);
    store.put_delivery(delivery(1, A, DeliveryKind::CodPayment));
    store.put_delivery(delivery(2, B, DeliveryKind::CodPayment));
    let mut s = scheduler(&store, &zones, vec![]);
    assert_eq!(s.tick().await.unwrap().enqueued.len(), 2);
    let a_job = zones.enqueued.lock()[0].clone();
    commit_delivery(&store, &a_job).await; // committed before the crash; its lease is gone
    let released = store.release_epoch(ZONE.0, ZONE.1).await.unwrap();
    let mut grants = released.clone();
    // The committed job's grant is reported by the zone's own record (its lease already went).
    grants.push(LeaseGrant {
        zone_id: ZONE.0,
        epoch: ZONE.1,
        account_id: None,
        operation: "deliver_pending".into(),
        request_key: a_job.job.request_key(),
        keys: a_job.frozen.leases.clone(),
    });
    let report = s.recover(&grants).await.unwrap();
    assert_eq!(report.committed, vec![SystemJob::DeliverPending(Uuid::from_u128(1))]);
    assert_eq!(report.requeued, vec![SystemJob::DeliverPending(Uuid::from_u128(2))]);
    assert_eq!(
        s.tick().await.unwrap().enqueued,
        vec![SystemJob::DeliverPending(Uuid::from_u128(2))]
    );
}

fn fixture() -> Vec<FixtureAuction> {
    vec![
        FixtureAuction {
            auction_id: 1,
            instance: 32320,
            round: 0,
            start_unix_ms: 10 * HOUR,
            end_unix_ms: 15 * HOUR,
            item_index: None,
        },
        FixtureAuction {
            auction_id: 4,
            instance: 32320,
            round: 1,
            start_unix_ms: 200 * HOUR,
            end_unix_ms: 205 * HOUR,
            item_index: Some(3),
        },
    ]
}

fn row(id: u64, state: u8, start: i64, end: i64) -> AuctionRow {
    AuctionRow {
        id,
        instance_npc: 32320,
        item_index: 0,
        state,
        highest_bidder: None,
        highest_bid: None,
        start_unix_ms: start,
        end_unix_ms: end,
        revision: 0,
    }
}

#[test]
fn auction_transitions_follow_the_fixture_schedule_on_db_time() {
    let day14 = 14 * 24 * HOUR;
    // No rows: only round 0 is created (round 1 waits for round 0's row).
    assert_eq!(
        auction_transitions(&fixture(), &[], 0, day14)
            .iter()
            .map(|(e, t)| (e.auction_id, *t))
            .collect::<Vec<_>>(),
        vec![(1, AuctionTransition::Create)]
    );
    let created = [row(1, 0, 10 * HOUR, 15 * HOUR)];
    assert_eq!(
        auction_transitions(&fixture(), &created, 9 * HOUR, day14)
            .iter()
            .map(|(e, t)| (e.auction_id, *t))
            .collect::<Vec<_>>(),
        vec![(4, AuctionTransition::Create)]
    );
    assert!(auction_transitions(&fixture(), &created, 10 * HOUR, day14)
        .iter()
        .any(|(e, t)| e.auction_id == 1 && *t == AuctionTransition::Start));
    // An extended end (P21) is the row's end, not the fixture's.
    let started = [
        row(1, 1, 10 * HOUR, 16 * HOUR),
        row(4, 0, 200 * HOUR, 205 * HOUR),
    ];
    assert!(
        auction_transitions(&fixture(), &started, 15 * HOUR, day14).is_empty(),
        "still running until the extended end"
    );
    assert_eq!(
        auction_transitions(&fixture(), &started, 16 * HOUR, day14)
            .iter()
            .map(|(e, t)| (e.auction_id, *t))
            .collect::<Vec<_>>(),
        vec![(1, AuctionTransition::Finish)]
    );
    let finished = [
        row(1, 2, 10 * HOUR, 16 * HOUR),
        row(4, 0, 200 * HOUR, 205 * HOUR),
    ];
    let purge_of_1 = |now| {
        auction_transitions(&fixture(), &finished, now, day14)
            .iter()
            .any(|(e, t)| e.auction_id == 1 && *t == AuctionTransition::Purge)
    };
    assert!(!purge_of_1(10 * HOUR + day14 - 1), "14 days after the start, not before");
    assert!(purge_of_1(10 * HOUR + day14));
    assert!(auction_transitions(&fixture(), &finished, 10 * HOUR + day14, day14)
        .iter()
        .any(|(e, t)| e.auction_id == 1 && *t == AuctionTransition::Purge));
    // A fixture auction whose end has passed is never created late.
    assert!(auction_transitions(&fixture(), &[], 15 * HOUR, day14).is_empty());
}

#[tokio::test]
async fn auction_jobs_go_to_the_auctioneer_zone_and_finish_leases_its_delivery() {
    let store = Arc::new(InMemoryEconomyStore::default());
    let zones = Arc::new(StubZones::default());
    store.set_now_ms(16 * HOUR);
    store.put_row(
        "item_auction",
        "1",
        StoredRow {
            revision: 3,
            columns: [
                ("instance_npc", serde_json::json!(32320)),
                ("item_index", serde_json::json!(5)),
                ("state", serde_json::json!(1)),
                ("start_unix_ms", serde_json::json!(10 * HOUR)),
                ("end_unix_ms", serde_json::json!(15 * HOUR)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        },
    );
    let mut s = scheduler(&store, &zones, fixture());
    assert_eq!(
        s.tick().await.unwrap().waiting.len(),
        2,
        "no auctioneer zone: Finish(1) and Create(4) wait"
    );
    zones.auctioneers.lock().push(32320);
    let report = s.tick().await.unwrap();
    assert!(
        report
            .enqueued
            .contains(&SystemJob::AuctionSchedule(1, AuctionTransition::Finish)),
        "{report:?}"
    );
    assert!(report
        .enqueued
        .contains(&SystemJob::AuctionSchedule(4, AuctionTransition::Create)));
    let jobs = zones.enqueued.lock().clone();
    let finish = jobs
        .iter()
        .find(|j| j.job == SystemJob::AuctionSchedule(1, AuctionTransition::Finish))
        .unwrap();
    assert_eq!(finish.frozen.leases.len(), 2);
    assert_eq!(
        finish.frozen.leases[0],
        LeaseKey {
            kind: LeaseKind::Auction,
            owner: 1
        }
    );
    assert_eq!(finish.frozen.leases[1].kind, LeaseKind::Delivery);
    assert!(
        finish
            .frozen
            .rows
            .iter()
            .any(|r| r.row_kind == "item_auction" && r.revision == 3),
        "the auction row is frozen"
    );
    let create = jobs
        .iter()
        .find(|j| j.job == SystemJob::AuctionSchedule(4, AuctionTransition::Create))
        .unwrap();
    assert_eq!(create.frozen.decisions["item_index"], serde_json::json!(3));
    assert_eq!(create.frozen.decisions["start_unix_ms"], serde_json::json!(200 * HOUR));
    // Commit markers: Finish needs state FINISHED; Create needs the row.
    assert!(!s
        .marker(SystemJob::AuctionSchedule(1, AuctionTransition::Finish))
        .await
        .unwrap());
    assert!(!s
        .marker(SystemJob::AuctionSchedule(4, AuctionTransition::Create))
        .await
        .unwrap());
    assert!(s
        .marker(SystemJob::AuctionSchedule(1, AuctionTransition::Start))
        .await
        .unwrap());
}

#[derive(Default)]
struct Sessions {
    admitted: Vec<Uuid>,
    sent: Mutex<BTreeMap<Uuid, Vec<MailNotificationView>>>,
}

#[async_trait]
impl MailNotifier for Sessions {
    async fn notify(&self, receiver: Uuid, notification: MailNotificationView) -> bool {
        if !self.admitted.contains(&receiver) {
            return false;
        }
        self.sent
            .lock()
            .entry(receiver)
            .or_default()
            .push(notification);
        true
    }
}

#[tokio::test]
async fn mail_notification_gateway_notifies_admitted_receivers_from_the_committed_row() {
    use nightfall_api::application::economy::EconomyCheckpointRepository;
    let store = InMemoryEconomyStore::default();
    store.set_character_name(A, "Skilla");
    store.set_character_name(B, "Skillb");
    let message = Uuid::from_u128(0x7100);
    let mailbox_b = LeaseKey::of_uuid(LeaseKind::Mailbox, B);
    let grant = LeaseGrant {
        zone_id: 1,
        epoch: 7,
        account_id: Some(A),
        operation: "mail_send".into(),
        request_key: Uuid::from_u128(5),
        keys: sets::mail_send(A, B),
    };
    store.acquire(&grant).await.unwrap();
    let write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 7,
        tick: 3,
        deltas: vec![ContainerDelta {
            lease: mailbox_b,
            row_kind: "mail_message".into(),
            row_key: message.to_string(),
            change: DeltaChange::Insert,
            expected_revision: None,
            columns: [
                ("sender_id", serde_json::json!(A.to_string())),
                ("receiver_id", serde_json::json!(B.to_string())),
                ("subject", serde_json::json!("paycheck")),
                ("body", serde_json::json!("b")),
                ("cod_adena", serde_json::json!(500)),
                ("has_attachments", serde_json::json!(true)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        }],
        lease_releases: sets::mail_send(A, B),
        ..Default::default()
    };
    store.checkpoint_economy(&write).await.unwrap();
    let offline = Sessions::default();
    assert!(
        notify_new_mail(&write, &store, &offline)
            .await
            .unwrap()
            .is_empty(),
        "not admitted: nothing sent, nothing written"
    );
    let online = Sessions {
        admitted: vec![B],
        ..Sessions::default()
    };
    assert_eq!(notify_new_mail(&write, &store, &online).await.unwrap(), vec![message]);
    let sent = online.sent.lock().clone();
    assert_eq!(
        sent[&B],
        vec![MailNotificationView {
            message_id: message,
            sender_name: "Skilla".into(),
            subject: "paycheck".into(),
            has_attachments: true,
            cod_adena: 500
        }]
    );
    // The COD mail expires in 12 h of DB time (computed in the insert).
    assert_eq!(
        store
            .mail(B, message)
            .await
            .unwrap()
            .unwrap()
            .expires_unix_ms,
        MEMORY_EPOCH_MS + 12 * HOUR
    );
}
