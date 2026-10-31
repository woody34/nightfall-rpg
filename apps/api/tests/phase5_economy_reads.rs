//! Story 4.6 / 4.5: the economy **read** port on the in-memory and the Postgres adapters (the
//! Postgres half runs only with `DATABASE_URL`). Both adapters hold the same logical rows; every
//! read method must return the same answer, DB times aside (memory has its own clock). Covers
//! container items, mail lists with expiry and side deletion, attachments, auctions and bids,
//! manor rows, deliveries, mailbox counts and the per-lease pre-images (E12: no `now()` time).
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::panic,
    clippy::type_complexity,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss
)]

mod common;

use nightfall_api::application::economy::{
    auction_bid_owner, sets, ContainerDelta, DeliveryKind, DeliveryRow, DeltaChange,
    EconomyCheckpointRepository, EconomyCheckpointWrite, LeaseGrant, LeaseKey, LeaseKind,
    LeaseRepository,
};
use nightfall_api::application::economy_reads::{EconomyReadRepository, MailRow};
use nightfall_api::infrastructure::memory::{InMemoryEconomyStore, StoredRow, MEMORY_EPOCH_MS};
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgEconomyRepository,
};
use sea_orm_migration::MigratorTrait;
use sqlx::PgPool;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0x5000);
const B: Uuid = Uuid::from_u128(0x5001);
const M_ATTACH: Uuid = Uuid::from_u128(0x7100);
const M_PLAIN: Uuid = Uuid::from_u128(0x7101);
const M_DELETED: Uuid = Uuid::from_u128(0x7102);
const M_EXPIRED: Uuid = Uuid::from_u128(0x7103);
const DELIVERY: Uuid = Uuid::from_u128(0x7200);
const HOUR: i64 = 3_600_000;
const START: i64 = 1_793_404_800_000;

/// `(id, owner_kind, character, container, template, count, enchant)`.
fn items() -> Vec<(i64, &'static str, Option<Uuid>, Option<Uuid>, i32, i64, i64)> {
    vec![
        (900_002, "warehouse", Some(A), None, 57, 100, 0),
        (900_001, "warehouse", Some(A), None, 1864, 5, 3),
        (900_003, "freight", Some(A), None, 1865, 2, 0),
        (900_010, "mail", None, Some(M_ATTACH), 57, 500, 0),
        (900_020, "auction_bid", None, Some(auction_bid_owner(1, A)), 57, 110_000, 0),
    ]
}

/// `(id, sender, receiver, subject, has_attachments, deleted_by_receiver, created_h, expires_h)`
/// with hours relative to DB now.
fn mails() -> Vec<(Uuid, Uuid, Uuid, &'static str, bool, bool, i64, i64)> {
    vec![
        (M_ATTACH, B, A, "adena", true, false, -1, 359),
        (M_PLAIN, B, A, "plain", false, false, -2, 358),
        (M_DELETED, B, A, "gone", false, true, -3, 357),
        (M_EXPIRED, A, B, "old", false, false, -400, -40),
    ]
}

fn cols(pairs: Vec<(&str, serde_json::Value)>) -> serde_json::Map<String, serde_json::Value> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

fn memory() -> InMemoryEconomyStore {
    let store = InMemoryEconomyStore::default();
    store.set_character_name(A, "Owner");
    store.set_character_name(B, "Peer");
    for (id, kind, character, container, template, count, enchant) in items() {
        let mut c = cols(vec![
            ("template_id", template.into()),
            ("count", count.into()),
            ("enchant", enchant.into()),
            ("charge_bits", 0.into()),
            ("location", kind.into()),
            ("owner_kind", kind.into()),
            ("created_tick", 0.into()),
        ]);
        if let Some(character) = character {
            c.insert("character_id".into(), character.to_string().into());
        }
        if let Some(container) = container {
            c.insert("container_id".into(), container.to_string().into());
            if kind == "mail" {
                c.insert("mail_id".into(), container.to_string().into());
            }
        }
        store.put_row(
            "item_instance",
            &id.to_string(),
            StoredRow {
                revision: 1,
                columns: c,
            },
        );
    }
    for (id, sender, receiver, subject, attachments, deleted, created, expires) in mails() {
        store.put_row(
            "mail_message",
            &id.to_string(),
            StoredRow {
                revision: 0,
                columns: cols(vec![
                    ("sender_id", sender.to_string().into()),
                    ("receiver_id", receiver.to_string().into()),
                    ("subject", subject.into()),
                    ("body", "body".into()),
                    ("cod_adena", 0.into()),
                    ("system_kind", 0.into()),
                    ("has_attachments", attachments.into()),
                    ("returned", false.into()),
                    ("read", false.into()),
                    ("deleted_by_sender", false.into()),
                    ("deleted_by_receiver", deleted.into()),
                ]),
            },
        );
        store.set_mail_times(
            id,
            MEMORY_EPOCH_MS + created * HOUR,
            MEMORY_EPOCH_MS + expires * HOUR,
        );
    }
    store.put_row(
        "item_auction",
        "1",
        StoredRow {
            revision: 2,
            columns: cols(vec![
                ("instance_npc", 32320.into()),
                ("item_index", 0.into()),
                ("state", 1.into()),
                ("extend_state", 0.into()),
                ("last_bidder", B.to_string().into()),
                ("highest_bidder", B.to_string().into()),
                ("highest_bid", 120_000.into()),
                ("start_unix_ms", START.into()),
                ("end_unix_ms", (START + 5 * HOUR).into()),
            ]),
        },
    );
    for (who, bid) in [(A, 110_000), (B, 120_000)] {
        store.put_row(
            "item_auction_bid",
            &format!("1:{who}"),
            StoredRow {
                revision: 0,
                columns: cols(vec![
                    ("auction_id", 1.into()),
                    ("character_id", who.to_string().into()),
                    ("last_bid", bid.into()),
                ]),
            },
        );
    }
    for (seed, amount) in [(10_000, 7), (5016, 8000)] {
        store.put_row(
            "manor_seed_production",
            &format!("1:{seed}"),
            StoredRow {
                revision: 0,
                columns: cols(vec![
                    ("castle_id", 1.into()),
                    ("seed_id", seed.into()),
                    ("price", 5.into()),
                    ("start_amount", 8100.into()),
                    ("amount", amount.into()),
                ]),
            },
        );
    }
    store.put_row(
        "manor_crop_procure",
        "1:5073",
        StoredRow {
            revision: 0,
            columns: cols(vec![
                ("castle_id", 1.into()),
                ("crop_id", 5073.into()),
                ("price", 50.into()),
                ("start_amount", 9000.into()),
                ("amount", 9000.into()),
                ("reward_type", 1.into()),
            ]),
        },
    );
    store.put_delivery(DeliveryRow {
        id: DELIVERY,
        target_character: A,
        kind: DeliveryKind::CodPayment,
        payload: serde_json::json!({"adena": 500}),
    });
    store
}

async fn postgres(pool: &PgPool) -> PgEconomyRepository {
    let db = connection_from_pool(pool);
    Migrator::up(&db, None).await.unwrap();
    for (id, account, name) in [
        (A, Uuid::from_u128(0x5100), "Owner"),
        (B, Uuid::from_u128(0x5101), "Peer"),
    ] {
        sqlx::query("INSERT INTO accounts(id) VALUES($1)")
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$3,'human',1,1,1,1,1,1,0,0,'human_fighter',true)")
            .bind(id).bind(account).bind(name).execute(pool).await.unwrap();
    }
    for (id, sender, receiver, subject, attachments, deleted, created, expires) in mails() {
        sqlx::query("INSERT INTO mail_messages(id,sender_id,receiver_id,subject,body,has_attachments,deleted_by_receiver,created_at,expires_at) VALUES($1,$2,$3,$4,'body',$5,$6,now() + $7 * interval '1 hour',now() + $8 * interval '1 hour')")
            .bind(id).bind(sender).bind(receiver).bind(subject).bind(attachments).bind(deleted).bind(created as f64).bind(expires as f64)
            .execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO economy_deliveries(id,target_character,kind,payload) VALUES($1,$2,'cod_payment',$3)")
        .bind(DELIVERY).bind(A).bind(serde_json::json!({"adena": 500})).execute(pool).await.unwrap();
    for (id, kind, character, container, template, count, enchant) in items() {
        sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,created_tick,revision,owner_kind,container_id,mail_id) VALUES($1,$2,$3,NULL,$4,$5,$6,0,1,$3,$7,$8)")
            .bind(id).bind(character).bind(kind).bind(template).bind(count).bind(enchant).bind(container)
            .bind(if kind == "mail" { container } else { None })
            .execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO item_auctions(id,instance_npc,item_index,state,extend_state,last_bidder,highest_bidder,highest_bid,start_at,end_at,revision) VALUES(1,32320,0,1,0,$1,$1,120000,'epoch'::timestamptz + $2 * interval '1 millisecond','epoch'::timestamptz + $3 * interval '1 millisecond',2)")
        .bind(B).bind(START as f64).bind((START + 5 * HOUR) as f64).execute(pool).await.unwrap();
    for (who, bid) in [(A, 110_000_i64), (B, 120_000)] {
        sqlx::query(
            "INSERT INTO item_auction_bids(auction_id,character_id,last_bid) VALUES(1,$1,$2)",
        )
        .bind(who)
        .bind(bid)
        .execute(pool)
        .await
        .unwrap();
    }
    for (seed, amount) in [(10_000, 7_i64), (5016, 8000)] {
        sqlx::query("INSERT INTO manor_seed_production(castle_id,seed_id,price,start_amount,amount) VALUES(1,$1,5,8100,$2)")
            .bind(seed).bind(amount).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO manor_crop_procure(castle_id,crop_id,price,start_amount,amount,reward_type) VALUES(1,5073,50,9000,9000,1)")
        .execute(pool).await.unwrap();
    PgEconomyRepository::new(db)
}

/// Mail rows without their DB times (each adapter has its own clock).
fn timeless(rows: Vec<MailRow>) -> Vec<MailRow> {
    rows.into_iter()
        .map(|mut m| {
            m.created_unix_ms = 0;
            m.expires_unix_ms = 0;
            m
        })
        .collect()
}

/// Every read method, as one comparable transcript.
async fn transcript(repo: &dyn EconomyReadRepository) -> Vec<String> {
    let mut out = Vec::new();
    for (kind, who) in [("warehouse", A), ("freight", A), ("warehouse", B)] {
        out.push(format!(
            "items {kind} {who}: {:?}",
            repo.container_items(kind, who).await.unwrap()
        ));
    }
    for (who, outbox) in [(A, false), (A, true), (B, false), (B, true)] {
        let rows = timeless(repo.mail_list(who, outbox).await.unwrap());
        out.push(format!("mail {who} outbox={outbox}: {rows:?}"));
    }
    for (who, id) in [
        (A, M_ATTACH),
        (A, M_DELETED),
        (B, M_DELETED),
        (B, M_ATTACH),
        (A, M_EXPIRED),
        (A, Uuid::from_u128(1)),
    ] {
        let row = repo.mail(who, id).await.unwrap().map(|m| timeless(vec![m]));
        out.push(format!("mail {who} {id}: {row:?}"));
    }
    out.push(format!("attachments: {:?}", repo.mail_attachments(M_ATTACH).await.unwrap()));
    out.push(format!("no attachments: {:?}", repo.mail_attachments(M_PLAIN).await.unwrap()));
    out.push(format!("expired: {:?}", timeless(repo.expired_mail(10).await.unwrap())));
    out.push(format!("expired limit 0: {:?}", repo.expired_mail(0).await.unwrap()));
    for id in [M_EXPIRED, Uuid::from_u128(1)] {
        out.push(format!("exists {id}: {}", repo.mail_exists(id).await.unwrap()));
    }
    out.push(format!("auctions: {:?}", repo.auctions(32320).await.unwrap()));
    out.push(format!("no auctions: {:?}", repo.auctions(32321).await.unwrap()));
    out.push(format!("bids: {:?}", repo.bids(1).await.unwrap()));
    out.push(format!("manor 1: {:?}", repo.manor(1).await.unwrap()));
    out.push(format!("manor 2: {:?}", repo.manor(2).await.unwrap()));
    out.push(format!("delivery: {:?}", repo.delivery(DELIVERY).await.unwrap()));
    out.push(format!("no delivery: {:?}", repo.delivery(Uuid::from_u128(1)).await.unwrap()));
    out.push(format!("targets: {:?}", repo.delivery_targets().await.unwrap()));
    for who in [A, B] {
        out.push(format!("counts {who}: {:?}", repo.mailbox_counts(who).await.unwrap()));
    }
    let keys = [
        LeaseKey::of_uuid(LeaseKind::Warehouse, A),
        LeaseKey::of_uuid(LeaseKind::Freight, A),
        LeaseKey::of_uuid(LeaseKind::Mail, M_ATTACH),
        LeaseKey::of_uuid(LeaseKind::Delivery, DELIVERY),
        LeaseKey {
            kind: LeaseKind::Auction,
            owner: 1,
        },
        LeaseKey {
            kind: LeaseKind::Manor,
            owner: 1,
        },
        LeaseKey::of_uuid(LeaseKind::Mailbox, A),
    ];
    for key in keys {
        out.push(format!(
            "pre-image {}: {:?}",
            key.canonical(),
            repo.pre_image(&[key]).await.unwrap()
        ));
    }
    out
}

fn assert_expected(transcript: &[String]) {
    let find = |prefix: &str| {
        transcript
            .iter()
            .find(|l| l.starts_with(prefix))
            .unwrap()
            .clone()
    };
    assert!(
        find(&format!("items warehouse {A}"))
            .find("900001")
            .unwrap()
            < find(&format!("items warehouse {A}"))
                .find("900002")
                .unwrap()
    );
    assert!(find(&format!("items warehouse {B}")).ends_with("[]"));
    let inbox = find(&format!("mail {A} outbox=false"));
    assert!(
        inbox.find("\"adena\"").unwrap() < inbox.find("\"plain\"").unwrap(),
        "newest first: {inbox}"
    );
    assert!(
        !inbox.contains("\"gone\"") && !inbox.contains("\"old\""),
        "deleted and expired hidden: {inbox}"
    );
    assert!(find(&format!("mail {A} {M_DELETED}")).ends_with("None"), "receiver deleted it");
    assert!(
        find(&format!("mail {B} {M_DELETED}")).contains("\"gone\""),
        "the sender still sees it"
    );
    assert!(find(&format!("mail {A} {M_EXPIRED}")).ends_with("None"));
    assert!(find("expired:").contains("\"old\""));
    assert!(find(&format!("exists {M_EXPIRED}")).ends_with("true"));
    assert!(find("bids:").contains("110000") && find("bids:").contains("120000"));
    assert!(find("targets:").contains(&A.to_string()));
    assert!(
        find(&format!("counts {A}")).ends_with("(2, 1)"),
        "{}",
        find(&format!("counts {A}"))
    );
    let manor = find("pre-image manor:");
    assert!(
        manor.find("5016").unwrap() < manor.find("10000").unwrap(),
        "seeds by id: {manor}"
    );
    let auction = find("pre-image auction:");
    assert!(auction.contains("900020"), "the escrow item: {auction}");
    assert!(
        !transcript.iter().any(|l| l.starts_with("pre-image")
            && [
                "created_at",
                "expires_at",
                "created_unix",
                "expires_unix",
                "created_ms",
                "expires_ms"
            ]
            .iter()
            .any(|t| l.contains(t))),
        "E12"
    );
}

#[tokio::test]
async fn memory_read_port_answers_the_contract() {
    let store = memory();
    let transcript = transcript(&store).await;
    assert_expected(&transcript);
}

#[tokio::test]
async fn postgres_read_port_matches_the_memory_adapter() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let pg = postgres(&pool).await;
    let pg_lines = transcript(&pg).await;
    assert_expected(&pg_lines);
    let memory_lines = transcript(&memory()).await;
    for (pg_line, memory_line) in pg_lines.iter().zip(&memory_lines) {
        assert_eq!(pg_line, memory_line);
    }
    assert_eq!(pg_lines.len(), memory_lines.len());
    // The writer stores an AuctionCreate window from integer epoch ms exactly (no float).
    let window = (START + 123, START + 5 * HOUR + 7);
    let keys = sets::auction(2);
    pg.acquire(&LeaseGrant {
        zone_id: 1,
        epoch: 1,
        account_id: None,
        operation: "auction_schedule".into(),
        request_key: Uuid::from_u128(0x99),
        keys: keys.clone(),
    })
    .await
    .unwrap();
    pg.checkpoint_economy(&EconomyCheckpointWrite {
        zone_id: 1,
        epoch: 1,
        tick: 5,
        deltas: vec![ContainerDelta {
            lease: keys[0],
            row_kind: "item_auction".into(),
            row_key: "2".into(),
            change: DeltaChange::Insert,
            expected_revision: None,
            columns: cols(vec![
                ("instance_npc", 32320.into()),
                ("item_index", 1.into()),
                ("state", 0.into()),
                ("start_unix_ms", window.0.into()),
                ("end_unix_ms", window.1.into()),
            ]),
        }],
        lease_releases: keys,
        ..Default::default()
    })
    .await
    .unwrap();
    let created = pg
        .auctions(32320)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.id == 2)
        .unwrap();
    assert_eq!(
        (created.start_unix_ms, created.end_unix_ms, created.state),
        (window.0, window.1, 0)
    );
    // DB time is Postgres now(): the attachment mail expires about 359 h from it.
    let now = pg.db_now_ms().await.unwrap();
    let mail = pg.mail(A, M_ATTACH).await.unwrap().unwrap();
    assert!(
        (mail.expires_unix_ms - now - 359 * HOUR).abs() < 60_000,
        "{}",
        mail.expires_unix_ms - now
    );
}
