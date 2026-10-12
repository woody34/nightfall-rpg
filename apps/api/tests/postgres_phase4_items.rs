//! Dependency-backed Phase 4 schema, pending-kit and allocator-boundary checks.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api::infrastructure::postgres::{connection_from_pool, Migrator};
use sea_orm_migration::MigratorTrait;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

const BACKFILL: &str = include_str!(
    "../src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql"
);

async fn seed(pool: &PgPool, ordinal: u128, lineage: (&str, i32, i32, &str), alive: bool) -> Uuid {
    let (race, base, current, profile) = lineage;
    let id = Uuid::from_u128(ordinal);
    sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$3,$4,1,1,1,1,1,1,$5,$6,$7,$8)")
        .bind(id).bind(Uuid::nil()).bind(format!("Kit{ordinal:03}")).bind(race).bind(base).bind(current).bind(profile).bind(alive).execute(pool).await.unwrap();
    id
}

#[tokio::test]
async fn backfill_preserves_every_base_kit_without_allocating_owned_items_and_is_rerunnable() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(10)).await.unwrap();
    let classes = [
        ("human", 0, 3, "human_fighter"),
        ("human", 10, 15, "human_mystic"),
        ("elf", 18, 21, "elven_fighter"),
        ("elf", 25, 28, "elven_mystic"),
        ("dark_elf", 31, 34, "dark_fighter"),
        ("dark_elf", 38, 41, "dark_mystic"),
        ("orc", 44, 46, "orc_fighter"),
        ("orc", 49, 51, "orc_mystic"),
        ("dwarf", 53, 55, "dwarven_fighter"),
    ];
    for (i, (race, base, current, profile)) in classes.into_iter().enumerate() {
        seed(
            &pool,
            u128::try_from(i).unwrap() + 1,
            (race, base, current, profile),
            i % 2 == 0,
        )
        .await;
    }
    Migrator::up(&db, None).await.unwrap();
    let first: Vec<(Uuid,i32,String,serde_json::Value)> = sqlx::query_as("SELECT request_id,manifest_version,manifest_hash,payload FROM pending_item_requests ORDER BY character_id").fetch_all(&pool).await.unwrap();
    assert_eq!(first.len(), 9);
    for (_, version, hash, payload) in &first {
        assert_eq!(*version, 1);
        assert_eq!(hash.len(), 64);
        let base = payload["source_class_id"].as_u64().unwrap();
        let items = payload["items"].as_array().unwrap();
        assert_eq!(items[items.len() - 2]["count"], 5);
        assert_eq!(items[items.len() - 1]["count"], 10);
        if base == 44 {
            assert_eq!(items[0]["template_id"], 2369);
            assert_eq!(items[0]["equipped"], false);
            assert_eq!(items[1]["template_id"], 2368);
            assert_eq!(items[1]["equipped"], true);
        } else if base == 49 {
            assert_eq!(items[0]["template_id"], 2368);
        } else if base == 53 {
            assert_eq!(items[0]["template_id"], 2370);
        }
    }
    for query in [
        "SELECT count(*) FROM item_instances",
        "SELECT count(*) FROM item_ledger",
        "SELECT count(*) FROM item_id_ranges",
    ] {
        let count: i64 = sqlx::query_scalar(query).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }
    pool.execute(sqlx::raw_sql(BACKFILL)).await.unwrap();
    let second: Vec<(Uuid,i32,String,serde_json::Value)> = sqlx::query_as("SELECT request_id,manifest_version,manifest_hash,payload FROM pending_item_requests ORDER BY character_id").fetch_all(&pool).await.unwrap();
    assert_eq!(second, first);
    sqlx::query(
        "UPDATE pending_item_requests SET manifest_hash=repeat('0',64) WHERE character_id=$1",
    )
    .bind(Uuid::from_u128(1))
    .execute(&pool)
    .await
    .unwrap();
    assert!(pool.execute(sqlx::raw_sql(BACKFILL)).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pending_item_requests")
            .fetch_one(&pool)
            .await
            .unwrap(),
        9
    );
}

#[tokio::test]
async fn bigint_u32_slot_fk_and_nonoverlapping_allocator_constraints_are_enforced() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let id = seed(&pool, 1, ("human", 0, 0, "human_fighter"), true).await;
    let insert="INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,created_tick,revision) VALUES($1,$2,$3,$4,1,$5,$6,$7,0,1)";
    sqlx::query(insert)
        .bind(1_i64)
        .bind(id)
        .bind("paperdoll")
        .bind(Some(5_i16))
        .bind(1_i64)
        .bind(i64::from(u32::MAX))
        .bind(Some(i64::from(u32::MAX)))
        .execute(&pool)
        .await
        .unwrap();
    let values: (i64, i64) =
        sqlx::query_as("SELECT enchant,augment_id FROM item_instances WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(values, (i64::from(u32::MAX), i64::from(u32::MAX)));
    for (instance, owner, location, slot, count, enchant, augment) in [
        (0, id, "inventory", None, 1, 0, None),
        (2, Uuid::from_u128(99), "inventory", None, 1, 0, None),
        (2, id, "inventory", Some(5), 1, 0, None),
        (2, id, "paperdoll", None, 1, 0, None),
        (2, id, "paperdoll", Some(25), 1, 0, None),
        (2, id, "paperdoll", Some(5), 1, 0, None),
        (2, id, "inventory", None, 0, 0, None),
        (2, id, "inventory", None, 1, -1, None),
        (2, id, "inventory", None, 1, 4_294_967_296_i64, None),
        (2, id, "inventory", None, 1, 0, Some(4_294_967_296_i64)),
    ] {
        assert!(sqlx::query(insert)
            .bind(instance)
            .bind(owner)
            .bind(location)
            .bind(slot)
            .bind(count)
            .bind(enchant)
            .bind(augment)
            .execute(&pool)
            .await
            .is_err());
    }
    let range="INSERT INTO item_id_ranges(zone_id,epoch,range_start,range_end_exclusive,next_reserved) VALUES($1,1,$2,$3,$2)";
    sqlx::query(range)
        .bind(1)
        .bind(4_294_967_296_i64)
        .bind(8_589_934_592_i64)
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query(range)
        .bind(2)
        .bind(4_294_967_297_i64)
        .bind(8_589_934_593_i64)
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query(range)
        .bind(2)
        .bind(1_i64)
        .bind(4_294_967_296_i64)
        .execute(&pool)
        .await
        .is_err());
    sqlx::query(range)
        .bind(2)
        .bind(8_589_934_592_i64)
        .bind(i64::MAX)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE item_id_ranges SET next_reserved=range_end_exclusive WHERE zone_id=2")
        .execute(&pool)
        .await
        .unwrap(); // next=end is exhausted, valid persisted state
    assert!(sqlx::query(
        "UPDATE item_id_ranges SET next_reserved=range_end_exclusive+1 WHERE zone_id=2"
    )
    .execute(&pool)
    .await
    .is_err());
}

#[tokio::test]
async fn consumption_is_one_append_only_row_per_instance_per_tick_even_after_destruction() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let owner = seed(&pool, 1, ("human", 0, 0, "human_fighter"), true).await;
    let insert="INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta) VALUES($1,1,1,10,1,$2,77,$3,'{}')";
    sqlx::query(insert)
        .bind(owner)
        .bind("consume")
        .bind(0_i32)
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query(insert)
        .bind(owner)
        .bind("consume")
        .bind(0_i32)
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query(insert)
        .bind(owner)
        .bind("consume")
        .bind(1_i32)
        .execute(&pool)
        .await
        .is_err());
    sqlx::query(insert)
        .bind(owner)
        .bind("destroy")
        .bind(1_i32)
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE item_ledger SET delta='{}'")
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM item_ledger")
        .execute(&pool)
        .await
        .is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM item_ledger WHERE instance_id=77")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM item_instances")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
