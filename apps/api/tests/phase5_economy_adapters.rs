//! Phase 5 stories 4.3/4.4: lease acquisition, the economy checkpoint CAS and the delivery/counter
//! ports on the in-memory and the Postgres adapters (the Postgres half runs only with
//! `DATABASE_URL`). Covers: all-or-nothing acquisition with nothing inserted on a held key,
//! opposite-role MailSend order, concurrent acquires with exactly one winner, release and epoch
//! recovery, the CAS-miss `economy_lease_violation` fence (transaction rolled back), a forced
//! transient failure (injected on memory; a real `40P01` deadlock on Postgres), the delivery
//! queue order, counter idempotence by fact id, `expires_at` computed in the insert and the outbox
//! subjects.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::doc_markdown,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

mod common;

use std::sync::Arc;

use nightfall_api::application::economy::{
    sets, ContainerDelta, DeliveryCompletion, DeliveryKind, DeliveryRepository, DeliveryRow,
    DeltaChange, EconomyCheckpointRepository, EconomyCheckpointWrite, EconomyCounterRepository,
    EconomyError, LeaseGrant, LeaseKey, LeaseKind, LeaseRepository, OutboxFact, OwnerLedgerRow,
    SystemJob,
};
use nightfall_api::infrastructure::memory::{InMemoryEconomyStore, StoredRow};
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgEconomyRepository,
};
use sea_orm_migration::MigratorTrait;
use sqlx::{PgPool, Row};
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0x5000);
const B: Uuid = Uuid::from_u128(0x5001);
const ACCOUNT: Uuid = Uuid::from_u128(0x5100);
const MESSAGE: Uuid = Uuid::from_u128(0x7100);
const DELIVERY: Uuid = Uuid::from_u128(0x7200);

fn grant(keys: Vec<LeaseKey>, request_key: Uuid) -> LeaseGrant {
    LeaseGrant {
        zone_id: 1,
        epoch: 1,
        account_id: Some(ACCOUNT),
        operation: "test".into(),
        request_key,
        keys,
    }
}

fn columns(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

/// The shared contract over any adapter that implements all four ports.
async fn lease_contract<R: LeaseRepository>(repo: &R) {
    // c-review-4 P2-2: a grant whose keys are not strictly ascending is refused as Constraint, nothing inserted.
    let mut descending = sets::mail_send(A, B);
    descending.reverse();
    let refused = repo
        .acquire(&grant(descending.clone(), Uuid::from_u128(0xdead)))
        .await
        .unwrap_err();
    assert!(matches!(refused, EconomyError::Constraint(_)), "{refused:?}");
    assert!(repo.held(&descending).await.unwrap().is_empty());
    // All-or-nothing: the second grant shares one key and gets Busy with nothing inserted.
    let first = grant(sets::mail_send(A, B), Uuid::from_u128(1));
    repo.acquire(&first).await.unwrap();
    let second = grant(sets::mail_cancel(MESSAGE, B, A), Uuid::from_u128(2));
    let busy = repo.acquire(&second).await.unwrap_err();
    assert!(matches!(&busy, EconomyError::Busy(held) if held.len() == 2), "{busy:?}");
    assert!(busy.is_retryable());
    assert!(
        repo.held(&[LeaseKey::of_uuid(LeaseKind::Mail, MESSAGE)])
            .await
            .unwrap()
            .is_empty(),
        "the free key of a refused set was not inserted"
    );
    let held = repo.held(&first.keys).await.unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].request_key, first.request_key);
    assert_eq!(held[0].keys, first.keys);
    // Opposite roles acquire in the same order: the same set is busy whichever side asks.
    assert_eq!(sets::mail_send(B, A), first.keys);
    // Release the first; the second now acquires.
    repo.release(1, 1, &first.keys).await.unwrap();
    repo.acquire(&second).await.unwrap();
    // A release for another epoch does not touch it.
    repo.release(1, 2, &second.keys).await.unwrap();
    assert_eq!(repo.held(&second.keys).await.unwrap().len(), 1);
    // Epoch recovery releases everything of (zone, epoch) and reports the grants.
    let released = repo.release_epoch(1, 1).await.unwrap();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].request_key, second.request_key);
    assert!(repo.held(&second.keys).await.unwrap().is_empty());
    // System-job keys are accepted as request keys (no account).
    let job = LeaseGrant {
        account_id: None,
        operation: "deliver_pending".into(),
        ..grant(
            sets::deliver_pending(DELIVERY, DeliveryKind::AuctionAward, A),
            SystemJob::DeliverPending(DELIVERY).request_key(),
        )
    };
    repo.acquire(&job).await.unwrap();
    assert_eq!(job.keys[0].kind, LeaseKind::Delivery);
    assert_eq!(job.keys[1].kind, LeaseKind::Warehouse);
    repo.release_epoch(1, 1).await.unwrap();
}

/// c-review-4 P3-1: two overlapping multi-key sets ({a,b} vs {b,c}) race; the loser inserted neither key.
async fn overlapping_sets_leave_exactly_one_whole_set<R: LeaseRepository + 'static>(repo: Arc<R>) {
    const C: Uuid = Uuid::from_u128(0x5002);
    let left = sets::mail_send(A, B);
    let right = sets::mail_send(B, C);
    let mut all: Vec<LeaseKey> = left.iter().chain(right.iter()).copied().collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 3, "the sets must overlap on exactly one key");
    for round in 0..8_u128 {
        let (ra, rb) = (Arc::clone(&repo), Arc::clone(&repo));
        let (la, rc) = (left.clone(), right.clone());
        let ta = tokio::spawn(async move {
            ra.acquire(&grant(la, Uuid::from_u128(0x2000 + round)))
                .await
        });
        let tb = tokio::spawn(async move {
            rb.acquire(&grant(rc, Uuid::from_u128(0x3000 + round)))
                .await
        });
        let outcomes = [ta.await.unwrap(), tb.await.unwrap()];
        for outcome in &outcomes {
            assert!(
                matches!(outcome, Ok(()) | Err(EconomyError::Busy(_) | EconomyError::Transient(_))),
                "{outcome:?}"
            );
        }
        let winners = outcomes.iter().filter(|o| o.is_ok()).count();
        let held = repo.held(&all).await.unwrap();
        assert!(winners <= 1, "{winners} winners");
        if winners == 1 {
            assert_eq!(held.len(), 1, "exactly one grant holds keys");
            assert!(held[0].keys == left || held[0].keys == right);
            // The loser's exclusive key was never inserted.
            let exclusive = if held[0].keys == left {
                right[1]
            } else {
                left[0]
            };
            assert!(repo.held(&[exclusive]).await.unwrap().is_empty());
        } else {
            assert!(held.is_empty());
        }
        repo.release_epoch(1, 1).await.unwrap();
    }
}

async fn concurrent_acquires_have_exactly_one_winner<R: LeaseRepository + 'static>(repo: Arc<R>) {
    let keys = sets::warehouse(A);
    let mut tasks = Vec::new();
    for i in 0..8_u128 {
        let repo = Arc::clone(&repo);
        let keys = keys.clone();
        tasks.push(tokio::spawn(async move {
            repo.acquire(&grant(keys, Uuid::from_u128(0x100 + i))).await
        }));
    }
    let mut winners = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(()) => winners += 1,
            Err(EconomyError::Busy(_) | EconomyError::Transient(_)) => {},
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!(winners, 1);
    repo.release_epoch(1, 1).await.unwrap();
}

fn ledger_row(
    owner_kind: &str,
    owner: Uuid,
    op: &str,
    ordinal: u32,
    after: serde_json::Value,
) -> OwnerLedgerRow {
    OwnerLedgerRow {
        owner_kind: owner_kind.into(),
        owner_id: owner,
        zone_id: 1,
        epoch: 1,
        tick: 30,
        revision: 1,
        op: op.into(),
        instance_id: 900_001,
        operation_ordinal: ordinal,
        delta: serde_json::json!({"before": {"count": 5}, "after": after}),
        receipt_ref: None,
    }
}

#[tokio::test]
async fn memory_leases_follow_the_contract() {
    let store = InMemoryEconomyStore::default();
    lease_contract(&store).await;
    concurrent_acquires_have_exactly_one_winner(Arc::new(InMemoryEconomyStore::default())).await;
    overlapping_sets_leave_exactly_one_whole_set(Arc::new(InMemoryEconomyStore::default())).await;
}

#[tokio::test]
async fn memory_checkpoint_cas_fence_transient_and_queues() {
    let store = InMemoryEconomyStore::default();
    store.put_row(
        "item_instance",
        "900001",
        StoredRow {
            revision: 0,
            columns: columns(&[("count", serde_json::json!(5))]),
        },
    );
    let warehouse = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
    store
        .acquire(&grant(vec![warehouse], Uuid::from_u128(9)))
        .await
        .unwrap();
    let mut write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 30,
        deltas: vec![ContainerDelta {
            lease: warehouse,
            row_kind: "item_instance".into(),
            row_key: "900001".into(),
            change: DeltaChange::Update,
            expected_revision: Some(0),
            columns: columns(&[("count", serde_json::json!(4))]),
        }],
        ledger: vec![ledger_row(
            "warehouse",
            A,
            "transfer_out",
            0,
            serde_json::Value::Null,
        )],
        lease_releases: vec![warehouse],
        outbox: vec![OutboxFact {
            subject: "withdrawn".into(),
            fact_id: Uuid::from_u128(0x900),
            payload: serde_json::json!({"count": 1}),
        }],
        ..Default::default()
    };
    // Forced transient: the write is refused once and nothing changes; the retry applies it.
    store.fail_next_transient(1);
    let transient = store.checkpoint_economy(&write).await.unwrap_err();
    assert!(matches!(transient, EconomyError::Transient(_)) && transient.is_retryable());
    assert_eq!(store.row("item_instance", "900001").unwrap().revision, 0);
    assert_eq!(store.held(&[warehouse]).await.unwrap().len(), 1);
    store.checkpoint_economy(&write).await.unwrap();
    let row = store.row("item_instance", "900001").unwrap();
    assert_eq!((row.revision, row.columns["count"].as_i64()), (1, Some(4)));
    assert!(
        store.held(&[warehouse]).await.unwrap().is_empty(),
        "lease released by the commit"
    );
    assert_eq!(store.ledger().len(), 1);
    assert_eq!(store.outbox()[0].subject, "withdrawn");
    // CAS miss: the stale expected revision is the permanent fence; nothing is applied.
    let miss = store.checkpoint_economy(&write).await.unwrap_err();
    assert!(matches!(miss, EconomyError::LeaseViolation(k) if k == warehouse), "{miss:?}");
    assert!(!miss.is_retryable());
    assert_eq!(store.row("item_instance", "900001").unwrap().revision, 1);
    assert_eq!(store.ledger().len(), 1);
    // Deliveries: inserted pending, queued by id, completed by CAS.
    write.deltas.clear();
    write.ledger.clear();
    write.outbox.clear();
    write.deliveries = vec![
        DeliveryRow {
            id: Uuid::from_u128(0x7202),
            target_character: A,
            kind: DeliveryKind::CodPayment,
            payload: serde_json::json!({"adena": 500}),
        },
        DeliveryRow {
            id: Uuid::from_u128(0x7201),
            target_character: A,
            kind: DeliveryKind::AuctionAward,
            payload: serde_json::json!({"template": 9901}),
        },
        DeliveryRow {
            id: Uuid::from_u128(0x7203),
            target_character: B,
            kind: DeliveryKind::MailExpiryReturn,
            payload: serde_json::json!({}),
        },
    ];
    write.deltas.clear();
    write.lease_releases.clear();
    store.checkpoint_economy(&write).await.unwrap();
    let queue = store.pending(A).await.unwrap();
    assert_eq!(
        queue.iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![Uuid::from_u128(0x7201), Uuid::from_u128(0x7202)]
    );
    assert_eq!(store.head(A).await.unwrap().unwrap().id, Uuid::from_u128(0x7201));
    write.deliveries.clear();
    write.completions = vec![DeliveryCompletion {
        id: Uuid::from_u128(0x7201),
        zone_id: 1,
        epoch: 1,
        tick: 31,
        expected_revision: 0,
    }];
    store.checkpoint_economy(&write).await.unwrap();
    assert_eq!(store.head(A).await.unwrap().unwrap().id, Uuid::from_u128(0x7202));
    assert!(
        matches!(
            store.checkpoint_economy(&write).await.unwrap_err(),
            EconomyError::LeaseViolation(_)
        ),
        "a completed delivery cannot complete again"
    );
    // Counters: idempotent by fact id.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    assert!(store
        .apply_fact(Uuid::from_u128(0x900), day, "adena_sink", 60)
        .await
        .unwrap());
    assert!(!store
        .apply_fact(Uuid::from_u128(0x900), day, "adena_sink", 60)
        .await
        .unwrap());
    assert!(store
        .apply_fact(Uuid::from_u128(0x901), day, "adena_sink", 40)
        .await
        .unwrap());
    assert_eq!(store.counter(day, "adena_sink").await.unwrap(), 100);
}

async fn seeded(pool: &PgPool) {
    for (id, account, name) in [(A, ACCOUNT, "Owner"), (B, Uuid::from_u128(0x5101), "Peer")] {
        sqlx::query("INSERT INTO accounts(id) VALUES($1)")
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$3,'human',1,1,1,1,1,1,0,0,'human_fighter',true)")
            .bind(id).bind(account).bind(name).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,created_tick,revision,owner_kind) VALUES(900001,$1,'warehouse',NULL,1864,5,0,0,'warehouse')")
        .bind(A).execute(pool).await.unwrap();
}

#[tokio::test]
async fn postgres_leases_follow_the_contract_and_concurrent_acquires_have_one_winner() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    seeded(&pool).await;
    let repo = Arc::new(PgEconomyRepository::new(db));
    lease_contract(repo.as_ref()).await;
    concurrent_acquires_have_exactly_one_winner(Arc::clone(&repo)).await;
    overlapping_sets_leave_exactly_one_whole_set(Arc::clone(&repo)).await;
    // Nothing is left in the lease table after the recoveries.
    let left: i64 = sqlx::query("SELECT count(*) FROM economy_leases")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0);
}

#[tokio::test]
async fn postgres_checkpoint_cas_fence_rolls_back_and_writes_everything_or_nothing() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    seeded(&pool).await;
    let repo = PgEconomyRepository::new(db);
    let warehouse = LeaseKey::of_uuid(LeaseKind::Warehouse, A);
    let mailbox_a = LeaseKey::of_uuid(LeaseKind::Mailbox, A);
    let mailbox_b = LeaseKey::of_uuid(LeaseKind::Mailbox, B);
    repo.acquire(&grant(vec![warehouse], Uuid::from_u128(9)))
        .await
        .unwrap();
    repo.acquire(&grant(sets::mail_send(A, B), Uuid::from_u128(10)))
        .await
        .unwrap();
    let write = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 30,
        deltas: vec![
            ContainerDelta {
                lease: warehouse,
                row_kind: "item_instance".into(),
                row_key: "900001".into(),
                change: DeltaChange::Update,
                expected_revision: Some(0),
                columns: columns(&[
                    ("character_id", serde_json::json!(A.to_string())),
                    ("location", serde_json::json!("warehouse")),
                    ("count", serde_json::json!(4)),
                    ("owner_kind", serde_json::json!("warehouse")),
                ]),
            },
            ContainerDelta {
                lease: mailbox_b,
                row_kind: "mail_message".into(),
                row_key: MESSAGE.to_string(),
                change: DeltaChange::Insert,
                expected_revision: None,
                columns: columns(&[
                    ("sender_id", serde_json::json!(A.to_string())),
                    ("receiver_id", serde_json::json!(B.to_string())),
                    ("subject", serde_json::json!("paycheck")),
                    ("body", serde_json::json!("b")),
                    ("cod_adena", serde_json::json!(500)),
                    ("has_attachments", serde_json::json!(true)),
                ]),
            },
        ],
        deliveries: vec![DeliveryRow {
            id: DELIVERY,
            target_character: A,
            kind: DeliveryKind::CodPayment,
            payload: serde_json::json!({"adena": 500}),
        }],
        completions: vec![],
        ledger: vec![
            ledger_row("warehouse", A, "transfer_in", 0, serde_json::json!({"count": 5})),
            ledger_row("warehouse", A, "consume", 1, serde_json::json!({"count": 4})),
        ],
        lease_releases: vec![mailbox_a, mailbox_b, warehouse],
        outbox: vec![OutboxFact {
            subject: "mailed".into(),
            fact_id: Uuid::from_u128(0x900),
            payload: serde_json::json!({"cod": 500}),
        }],
    };
    repo.checkpoint_economy(&write).await.unwrap();
    let row = sqlx::query("SELECT count, revision FROM item_instances WHERE id = 900001")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((row.get::<i64, _>("count"), row.get::<i64, _>("revision")), (4, 1));
    // expires_at is computed in the insert: 12 h for a COD mail (360 h otherwise).
    let hours: f64 = sqlx::query("SELECT (extract(epoch FROM (expires_at - created_at)) / 3600)::float8 AS h FROM mail_messages WHERE id = $1").bind(MESSAGE).fetch_one(&pool).await.unwrap().get("h");
    assert!((hours - 12.0).abs() < 0.01, "{hours}");
    let subject: String = sqlx::query("SELECT subject FROM outbox ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(subject, "nightfall.item.mailed");
    let ledger: i64 =
        sqlx::query("SELECT count(*) FROM item_ledger WHERE owner_kind = 'warehouse'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(ledger, 2);
    let leases: i64 = sqlx::query("SELECT count(*) FROM economy_leases")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(leases, 0, "leases deleted in the same transaction");
    assert_eq!(repo.head(A).await.unwrap().unwrap().id, DELIVERY);
    // CAS miss (stale revision) is the permanent fence and the whole transaction rolls back: no
    // second mail row, no ledger rows, no outbox fact.
    let mut stale = write.clone();
    stale.deltas[1].row_key = Uuid::from_u128(0x7101).to_string();
    stale.deliveries.clear();
    stale.ledger = vec![ledger_row(
        "warehouse",
        A,
        "transfer_out",
        0,
        serde_json::Value::Null,
    )];
    stale.ledger[0].tick = 31;
    let fence = repo.checkpoint_economy(&stale).await.unwrap_err();
    assert!(matches!(fence, EconomyError::LeaseViolation(k) if k == warehouse), "{fence:?}");
    assert!(!fence.is_retryable());
    let mails: i64 = sqlx::query("SELECT count(*) FROM mail_messages")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(mails, 1);
    let ledger: i64 = sqlx::query("SELECT count(*) FROM item_ledger")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(ledger, 2);
    // A completion with the right revision applies; a repeat is a violation.
    let mut complete = EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 32,
        ..Default::default()
    };
    complete.completions = vec![DeliveryCompletion {
        id: DELIVERY,
        zone_id: 1,
        epoch: 1,
        tick: 32,
        expected_revision: 0,
    }];
    repo.checkpoint_economy(&complete).await.unwrap();
    assert!(repo.head(A).await.unwrap().is_none());
    assert!(matches!(
        repo.checkpoint_economy(&complete).await.unwrap_err(),
        EconomyError::LeaseViolation(_)
    ));
    // Counters: idempotent by fact id; numeric(39,0) holds large values.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    assert!(repo
        .apply_fact(Uuid::from_u128(0x900), day, "adena_sink", 60)
        .await
        .unwrap());
    assert!(!repo
        .apply_fact(Uuid::from_u128(0x900), day, "adena_sink", 60)
        .await
        .unwrap());
    assert!(repo
        .apply_fact(Uuid::from_u128(0x901), day, "adena_sink", 1 << 70)
        .await
        .unwrap());
    assert_eq!(repo.counter(day, "adena_sink").await.unwrap(), 60 + (1_i128 << 70));
    assert_eq!(repo.counter(day, "unknown").await.unwrap(), 0);
}

#[tokio::test]
async fn postgres_deadlock_is_classified_transient_not_a_fence() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    seeded(&pool).await;
    // Two counters rows, two sessions updating them in opposite order: Postgres raises 40P01 in
    // one of them. The adapter's classifier turns the DbErr into `Transient`.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    sqlx::query("INSERT INTO economy_counters(day,counter,value) VALUES($1,'x',0),($1,'y',0)")
        .bind(day)
        .execute(&pool)
        .await
        .unwrap();
    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let left = tokio::spawn(async move {
        let mut tx = pool_a.begin().await.unwrap();
        sqlx::query("UPDATE economy_counters SET value = value + 1 WHERE counter = 'x'")
            .execute(&mut *tx)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let second =
            sqlx::query("UPDATE economy_counters SET value = value + 1 WHERE counter = 'y'")
                .execute(&mut *tx)
                .await;
        let _ = tx.rollback().await;
        second.err()
    });
    let right = tokio::spawn(async move {
        let mut tx = pool_b.begin().await.unwrap();
        sqlx::query("UPDATE economy_counters SET value = value + 1 WHERE counter = 'y'")
            .execute(&mut *tx)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let second =
            sqlx::query("UPDATE economy_counters SET value = value + 1 WHERE counter = 'x'")
                .execute(&mut *tx)
                .await;
        let _ = tx.rollback().await;
        second.err()
    });
    let errors: Vec<sqlx::Error> = [left.await.unwrap(), right.await.unwrap()]
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(errors.len(), 1, "exactly one session is the deadlock victim");
    let db_error = sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(Arc::new(
        errors.into_iter().next().unwrap(),
    )));
    let classified = nightfall_api::infrastructure::postgres::classify_economy_error(db_error);
    assert!(
        matches!(&classified, EconomyError::Transient(message) if message.contains("deadlock")),
        "{classified:?}"
    );
    assert!(classified.is_retryable());
}
