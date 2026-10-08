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
        "characters",
        "idempotency_keys",
        "outbox",
        "seaql_migrations",
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
    assert_eq!(applied, 1);
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
    assert_eq!(applied, 1);
}
