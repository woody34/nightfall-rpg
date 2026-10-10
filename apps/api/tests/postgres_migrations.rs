//! Runs only when `DATABASE_URL` is set. Fresh install and upgrade-from-sqlx paths.
#![allow(missing_docs, unreachable_pub, clippy::unwrap_used)]

mod common;

use nightfall_api::infrastructure::postgres::{connect, connection_from_pool, migrate, Migrator};
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::MigratorTrait;
use sqlx::Executor;

/// The schema exactly as the retired sqlx migrator created it.
const LEGACY_SQL: &str =
    include_str!("../src/infrastructure/postgres/migrations/m20260101_000001_characters.sql");

async fn table_names(pool: &sqlx::PgPool) -> Vec<String> {
    let db = connection_from_pool(pool);
    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT table_name::text AS t FROM information_schema.tables \
             WHERE table_schema = current_schema() ORDER BY 1",
        ))
        .await
        .unwrap();
    rows.iter().map(|r| r.try_get("", "t").unwrap()).collect()
}

#[tokio::test]
async fn fresh_install_creates_schema_and_is_rerunnable() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let tables = table_names(&pool).await;
    for t in [
        "account_sessions",
        "accounts",
        "characters",
        "idempotency_keys",
        "outbox",
        "play_tickets",
        "seaql_migrations",
        "zone_snapshots",
        "zone_epochs",
    ] {
        assert!(tables.contains(&t.to_owned()), "missing {t}: {tables:?}");
    }
}

#[tokio::test]
async fn upgrade_from_sqlx_schema_keeps_data() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(LEGACY_SQL.to_owned())))
        .await
        .unwrap();
    pool.execute(sqlx::raw_sql(
        "INSERT INTO outbox (subject, payload) VALUES ('nightfall.test', '{}')",
    ))
    .await
    .unwrap();

    Migrator::up(&connection_from_pool(&pool), None)
        .await
        .unwrap();

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "baseline migration must not touch existing data");
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM seaql_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(applied, i64::try_from(Migrator::migrations().len()).unwrap());
}

#[tokio::test]
async fn character_idempotency_keys_are_rekeyed_with_their_account() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(LEGACY_SQL.to_owned())))
        .await
        .unwrap();
    pool.execute(sqlx::raw_sql(
        "INSERT INTO characters (id, account_id, name, name_normalized, race, str, dex, con, \
             \"int\", wit, men) \
         VALUES ('0190a7e2-0000-7000-8000-00000000000c', '0190a7e2-0000-7000-8000-00000000000a', \
             'Durin', 'durin', 'dwarf', 39, 29, 45, 20, 10, 27); \
         INSERT INTO idempotency_keys (key, fingerprint, character_id) \
         VALUES ('0190a7e2-0000-7000-8000-0000000000ee', 'fp', \
             '0190a7e2-0000-7000-8000-00000000000c');",
    ))
    .await
    .unwrap();

    Migrator::up(&connection_from_pool(&pool), None)
        .await
        .unwrap();

    let (account, operation, response): (uuid::Uuid, String, serde_json::Value) =
        sqlx::query_as("SELECT account_id, operation, response FROM idempotency_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(account.to_string(), "0190a7e2-0000-7000-8000-00000000000a");
    assert_eq!(operation, "create_character");
    assert_eq!(
        response,
        serde_json::json!({ "character_id": "0190a7e2-0000-7000-8000-00000000000c" })
    );
}

#[tokio::test]
async fn concurrent_connects_on_a_fresh_database_all_succeed() {
    let Some(db) = common::pg::fresh_database(None).await else {
        return;
    };
    let attempts = (0..6).map(|_| {
        tokio::spawn({
            let url = db.url.clone();
            async move { connect(&url).await.map(|_| ()) }
        })
    });
    for attempt in attempts {
        attempt.await.unwrap().unwrap();
    }
    db.drop_db().await;
}

#[tokio::test]
async fn concurrent_upgrades_from_the_sqlx_schema_all_succeed() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(LEGACY_SQL.to_owned())))
        .await
        .unwrap();
    let attempts = (0..6).map(|_| {
        tokio::spawn({
            let pool = pool.clone();
            async move { migrate(&pool).await }
        })
    });
    for attempt in attempts {
        attempt.await.unwrap().unwrap();
    }
    let applied: i64 = sqlx::query_scalar("SELECT count(*) FROM seaql_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(applied, i64::try_from(Migrator::migrations().len()).unwrap());
}

#[tokio::test]
async fn phase_two_backfill_preserves_all_existing_progression_and_exact_race_paths() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(6)).await.unwrap();
    let profiles = [
        ("human", "human_fighter", 0),
        ("human", "human_mystic", 10),
        ("elf", "elven_fighter", 18),
        ("elf", "elven_mystic", 25),
        ("dark_elf", "dark_fighter", 31),
        ("dark_elf", "dark_mystic", 38),
        ("orc", "orc_fighter", 44),
        ("orc", "orc_mystic", 49),
        ("dwarf", "dwarven_fighter", 53),
    ];
    let mut ids = Vec::new();
    for (index, (race, profile, _)) in profiles.iter().enumerate() {
        let id = uuid::Uuid::now_v7();
        ids.push(id);
        let name = format!("Hero{}", char::from(b'a' + u8::try_from(index).unwrap()));
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,level,str,dex,con,\"int\",wit,men,pos_x,pos_y,xp,hp,mp,alive,class_profile,revision) VALUES($1,$2,$3,$4,$5,85,40,30,43,21,11,25,12.5,15.25,16890558727,17,11,false,$6,9)")
            .bind(id).bind(uuid::Uuid::nil()).bind(&name).bind(name.to_ascii_lowercase()).bind(*race).bind(*profile).execute(&pool).await.unwrap();
    }
    Migrator::up(&db, None).await.unwrap();
    for (id, (_, profile, class_id)) in ids.iter().zip(profiles) {
        let values:(i32,i32,i32,i64,Option<i32>,Option<i32>,bool,f32,f32,i64,String)=sqlx::query_as("SELECT base_class_id,current_class_id,level,xp,hp,mp,alive,pos_x,pos_y,revision,class_profile FROM characters WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            (
                values.0, values.1, values.2, values.3, values.4, values.5, values.6, values.9,
                values.10
            ),
            (
                class_id,
                class_id,
                85,
                16890558727,
                Some(17),
                Some(11),
                false,
                9,
                profile.into()
            )
        );
        assert_eq!(values.7.to_bits(), 12.5_f32.to_bits());
        assert_eq!(values.8.to_bits(), 15.25_f32.to_bits());
    }
}

#[tokio::test]
async fn phase_two_backfill_refuses_unknown_or_wrong_race_profiles_without_rewriting() {
    for profile in ["unknown_profile", "elven_mystic"] {
        let Some(pool) = common::pg::empty_schema_pool().await else {
            return;
        };
        let db = connection_from_pool(&pool);
        Migrator::up(&db, Some(6)).await.unwrap();
        sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,level,str,dex,con,\"int\",wit,men,xp,hp,mp,class_profile) VALUES($1,$2,'Hero','hero','human',85,40,30,43,21,11,25,16890558727,17,11,$3)")
            .bind(uuid::Uuid::now_v7()).bind(uuid::Uuid::nil()).bind(profile).execute(&pool).await.unwrap();
        let error = Migrator::up(&db, None).await.unwrap_err();
        assert!(error.to_string().contains("unknown or wrong-race"), "{error}");
        let values: (String, i32, i64, Option<i32>, Option<i32>) =
            sqlx::query_as("SELECT class_profile,level,xp,hp,mp FROM characters")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(values, (profile.into(), 85, 16890558727, Some(17), Some(11)));
    }
}
