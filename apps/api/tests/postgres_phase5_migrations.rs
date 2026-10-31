//! Phase 5 story 4.1 (design §14.4 "Migrations"): 000015..000019 forward on a seeded Phase 4
//! database, idempotence, the V7 CHECKs and the owner-keyed T1 trigger, and the guarded rollback.
//! Runs only when `DATABASE_URL` is set.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

mod common;

use nightfall_api::infrastructure::postgres::{connection_from_pool, Migrator};
use sea_orm_migration::MigratorTrait;
use sqlx::{Executor, PgPool, Row};
use uuid::Uuid;

const PHASE4_MIGRATIONS: u32 = 14;
const PHASE5_MIGRATIONS: u32 = 19;
const CHARACTER: Uuid = Uuid::from_u128(0x5000);
const PEER: Uuid = Uuid::from_u128(0x5001);
const ACCOUNT: Uuid = Uuid::from_u128(0x5100);
const PEER_ACCOUNT: Uuid = Uuid::from_u128(0x5101);

async fn seed_phase4(pool: &PgPool) {
    for (id, account, name) in [(CHARACTER, ACCOUNT, "Owner"), (PEER, PEER_ACCOUNT, "Peer")] {
        sqlx::query("INSERT INTO accounts(id) VALUES($1)")
            .bind(account)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$3,'human',1,1,1,1,1,1,0,0,'human_fighter',true)")
            .bind(id)
            .bind(account)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,created_tick,revision) VALUES(1,$1,'inventory',NULL,57,500,0,1),(2,$1,'paperdoll',6,2369,1,0,1)")
        .bind(CHARACTER)
        .execute(pool)
        .await
        .unwrap();
    // Phase 4 ledger shape: a grant row and a consume aggregate with ordinal n-1 (D76).
    sqlx::query("INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES($1,1,1,10,1,'grant',1,0,'{\"before\":null,\"after\":{\"count\":500}}'),($1,1,1,11,2,'consume',1,0,'{\"before\":{\"count\":500},\"after\":{\"count\":480}}')")
        .bind(CHARACTER)
        .execute(pool)
        .await
        .unwrap();
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(pool)
        .await
        .unwrap()
        .get::<i64, _>(0)
}

#[tokio::test]
async fn forward_on_a_seeded_phase4_database_keeps_rows_and_is_rerunnable() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(PHASE4_MIGRATIONS)).await.unwrap();
    seed_phase4(&pool).await;
    Migrator::up(&db, None).await.unwrap();
    Migrator::up(&db, None).await.unwrap(); // idempotent
    assert_eq!(
        count(&pool, "SELECT count(*) FROM seaql_migrations").await,
        i64::from(PHASE5_MIGRATIONS)
    );
    assert_eq!(Migrator::migrations().len(), PHASE5_MIGRATIONS as usize);
    // Phase 4 rows survive with the character owner derived from their character_id.
    let rows = sqlx::query(
        "SELECT owner_kind, owner_id, owner_key, character_id FROM item_ledger ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.get::<String, _>("owner_kind"), "character");
        assert_eq!(row.get::<Option<Uuid>, _>("owner_id"), None);
        assert_eq!(row.get::<Uuid, _>("owner_key"), CHARACTER);
        assert_eq!(row.get::<Option<Uuid>, _>("character_id"), Some(CHARACTER));
    }
    let items =
        sqlx::query("SELECT owner_kind, location, container_id FROM item_instances ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .all(|r| r.get::<String, _>("owner_kind") == "character"
            && r.get::<Option<Uuid>, _>("container_id").is_none()));
    assert_eq!(
        count(&pool, "SELECT count(*) FROM characters WHERE phase5_checkpoint IS NULL").await,
        2
    );
    for table in [
        "mail_messages",
        "item_auctions",
        "item_auction_bids",
        "manor_seed_production",
        "manor_crop_procure",
        "economy_leases",
        "economy_deliveries",
        "economy_counters",
        "economy_counter_facts",
    ] {
        assert_eq!(count(&pool, &format!("SELECT count(*) FROM {table}")).await, 0, "{table}");
    }
}

#[tokio::test]
async fn v7_rows_follow_the_owner_container_and_t1_rules() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    seed_phase4(&pool).await;
    let ground = Uuid::from_u128(0x6000);
    // Ground-owned loot row: no character, owner_id required; owner_key follows owner_id.
    sqlx::query("INSERT INTO item_ledger(owner_kind,owner_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('ground',$1,1,1,20,0,'loot',4294967296,0,'{\"before\":null,\"after\":{\"count\":3}}')")
        .bind(ground)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query("SELECT owner_key FROM item_ledger WHERE op = 'loot'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<Uuid, _>(0),
        ground
    );
    for bad in [
        // a non-character owner must carry an owner_id
        "INSERT INTO item_ledger(owner_kind,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('ground',1,1,21,0,'loot',4294967297,0,'{}')",
        // a character row must not carry a foreign owner kind
        "INSERT INTO item_ledger(owner_kind,character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('warehouse','00000000-0000-0000-0000-000000005000',1,1,21,0,'transfer_in',1,0,'{}')",
        // an op outside the Phase 4 + V7 vocabulary
        "INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('00000000-0000-0000-0000-000000005000',1,1,21,0,'drop',1,0,'{}')",
        // a receipt scope outside the 13 + 29 names
        "INSERT INTO item_mutation_receipts(account_id,operation,key,character_id,fingerprint,response) VALUES('00000000-0000-0000-0000-000000005100','pickup','00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000005000','f','{}')",
        // a lease key must be the 39-digit decimal form (E42)
        "INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,operation,request_key) VALUES('mail','1',1,1,'mail_send','00000000-0000-0000-0000-000000000001')",
        // a mail item row needs its container and mail_id
        "INSERT INTO item_instances(id,location,slot,template_id,count,created_tick,revision,owner_kind) VALUES(4294967300,'mail',NULL,1864,1,0,0,'mail')",
        // a warehouse row keeps its character
        "INSERT INTO item_instances(id,location,slot,template_id,count,created_tick,revision,owner_kind,container_id) VALUES(4294967301,'warehouse',NULL,1864,1,0,0,'warehouse','00000000-0000-0000-0000-000000006000')",
        // a delivered delivery needs its zone/epoch/tick
        "INSERT INTO economy_deliveries(id,target_character,kind,payload,status) VALUES('00000000-0000-0000-0000-000000007000','00000000-0000-0000-0000-000000005000','cod_payment','{}','delivered')",
    ] {
        assert!(pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(bad.to_owned()))).await.is_err(), "{bad}");
    }
    // A warehouse row (character kept) and a mail with an attachment row (container = message).
    sqlx::query("INSERT INTO item_instances(id,character_id,location,slot,template_id,count,created_tick,revision,owner_kind) VALUES(4294967302,$1,'warehouse',NULL,1864,5,0,0,'warehouse')")
        .bind(CHARACTER)
        .execute(&pool)
        .await
        .unwrap();
    let mail = Uuid::from_u128(0x7100);
    sqlx::query("INSERT INTO mail_messages(id,sender_id,receiver_id,subject,body,cod_adena,has_attachments,expires_at) VALUES($1,$2,$3,'s','b',500,true,now() + interval '12 hours')")
        .bind(mail)
        .bind(CHARACTER)
        .bind(PEER)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO item_instances(id,location,slot,template_id,count,created_tick,revision,owner_kind,container_id,mail_id) VALUES(4294967303,'mail',NULL,1864,3,0,0,'mail',$1,$1)")
        .bind(mail)
        .execute(&pool)
        .await
        .unwrap();
    // RESTRICT: the attachment row pins its message.
    assert!(sqlx::query("DELETE FROM mail_messages WHERE id = $1")
        .bind(mail)
        .execute(&pool)
        .await
        .is_err());
    // Lease keys sort bytewise in COLLATE "C": `mail:` < `mailbox:`.
    let key = format!("{:039}", mail.as_u128());
    sqlx::query("INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,operation,request_key) VALUES('mailbox',$1,1,1,'mail_send',$2),('mail',$1,1,1,'mail_mark_read',$2)")
        .bind(&key)
        .bind(Uuid::from_u128(0x7200))
        .execute(&pool)
        .await
        .unwrap();
    let order: Vec<String> = sqlx::query("SELECT owner_kind || ':' || owner_id AS k FROM economy_leases ORDER BY owner_kind, owner_id")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("k"))
        .collect();
    assert_eq!(order[0], format!("mail:{key}"));
    assert_eq!(order[1], format!("mailbox:{key}"));
    assert!(sqlx::query("INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,operation,request_key) VALUES('mail',$1,1,1,'x',$2)")
        .bind(&key)
        .bind(Uuid::from_u128(0x7201))
        .execute(&pool)
        .await
        .is_err(), "a held lease key refuses a second holder");
    // T1 on a non-character owner: [transfer_in, consume aggregate, terminal transfer_out] = ordinals 0,1,2.
    let warehouse = CHARACTER;
    let mut tx = pool.begin().await.unwrap();
    for (op, ordinal, after) in [
        ("transfer_in", 0, "{\"count\":5}"),
        ("consume", 1, "{\"count\":4}"),
        ("transfer_out", 2, "null"),
    ] {
        sqlx::query("INSERT INTO item_ledger(owner_kind,owner_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('warehouse',$1,1,1,30,0,$2,4294967302,$3,$4::jsonb)")
            .bind(warehouse)
            .bind(op)
            .bind(ordinal)
            .bind(format!("{{\"before\":{{\"count\":5}},\"after\":{after}}}"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    // T1 violated: two rows after the consume aggregate.
    let mut tx = pool.begin().await.unwrap();
    for (op, ordinal, after) in [
        ("consume", 0, "{\"count\":4}"),
        ("transfer_out", 1, "null"),
        ("destroy", 2, "null"),
    ] {
        sqlx::query("INSERT INTO item_ledger(owner_kind,owner_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('warehouse',$1,1,1,31,0,$2,4294967302,$3,$4::jsonb)")
            .bind(warehouse)
            .bind(op)
            .bind(ordinal)
            .bind(format!("{{\"before\":{{\"count\":4}},\"after\":{after}}}"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    assert!(
        tx.commit().await.is_err(),
        "T1: at most one terminal row after the Consume aggregate"
    );
    // The same owner key under two kinds is two owners (the logical key includes owner_kind).
    sqlx::query("INSERT INTO item_ledger(owner_kind,owner_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('freight',$1,1,1,30,0,'transfer_in',4294967302,0,'{\"before\":null,\"after\":{\"count\":5}}')")
        .bind(warehouse)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn rollback_refuses_with_v7_rows_and_succeeds_when_empty() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    seed_phase4(&pool).await;
    sqlx::query("INSERT INTO manor_seed_production(castle_id,seed_id,price,start_amount,amount) VALUES(1,5016,5,8100,8100)")
        .execute(&pool)
        .await
        .unwrap();
    // 000019 (control) is empty and rolls back; 000018 holds a manor row and refuses.
    Migrator::down(&db, Some(1)).await.unwrap();
    assert!(Migrator::down(&db, Some(1)).await.is_err());
    sqlx::query("DELETE FROM manor_seed_production")
        .execute(&pool)
        .await
        .unwrap();
    Migrator::down(&db, Some(4)).await.unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM seaql_migrations").await,
        i64::from(PHASE4_MIGRATIONS)
    );
    // Phase 4 rows and the Phase 4 constraints are back.
    assert_eq!(
        count(&pool, "SELECT count(*) FROM item_ledger WHERE character_id IS NOT NULL").await,
        2
    );
    assert!(sqlx::query("INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES($1,1,1,40,0,'loot',1,0,'{}')")
        .bind(CHARACTER)
        .execute(&pool)
        .await
        .is_err());
    let columns = count(&pool, "SELECT count(*) FROM information_schema.columns WHERE table_name = 'item_ledger' AND table_schema = current_schema() AND column_name IN ('owner_kind','owner_id','owner_key')").await;
    assert_eq!(columns, 0);
    // Forward again from the Phase 4 shape.
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM seaql_migrations").await,
        i64::from(PHASE5_MIGRATIONS)
    );
    // A V7 ledger row blocks the ledger rollback.
    sqlx::query("INSERT INTO item_ledger(owner_kind,owner_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES('ground',$1,1,1,50,0,'loot',4294967296,0,'{\"before\":null,\"after\":{\"count\":1}}')")
        .bind(Uuid::from_u128(0x6001))
        .execute(&pool)
        .await
        .unwrap();
    Migrator::down(&db, Some(4)).await.unwrap();
    assert!(Migrator::down(&db, Some(1)).await.is_err());
}
