//! Per-test Postgres schema, shared by the adapter tests. Skips (returns `None`) without
//! `DATABASE_URL`.

#![allow(dead_code, missing_docs, unreachable_pub, clippy::unwrap_used)]

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

/// A pool whose connections all use a brand-new empty schema (no migrations applied).
pub async fn empty_schema_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("test_{}", Uuid::now_v7().simple());
    admin
        .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}"))))
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |conn, _| {
            let schema = schema.clone();
            Box::pin(async move {
                conn.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                    "SET search_path TO {schema}"
                ))))
                .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    Some(pool)
}

/// A throwaway database (and optionally a login role with a known password) on the server
/// `DATABASE_URL` points at, for tests that exercise `connect` itself. Dropped by [`Self::drop_db`].
pub struct FreshDb {
    pub url: String,
    admin: PgPool,
    db: String,
    role: Option<String>,
}

async fn exec(pool: &PgPool, sql: String) {
    pool.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(sql)))
        .await
        .unwrap();
}

/// Creates `test_db_<uuid>`; with `password`, also a role owning it that authenticates with
/// that password, and `url` uses that role.
pub async fn fresh_database(password: Option<&str>) -> Option<FreshDb> {
    let base = std::env::var("DATABASE_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&base)
        .await
        .unwrap();
    let id = Uuid::now_v7().simple().to_string();
    let db = format!("test_db_{id}");
    let role = password.map(|_| format!("test_role_{id}"));
    if let (Some(role), Some(password)) = (&role, password) {
        exec(&admin, format!("CREATE ROLE {role} LOGIN PASSWORD '{password}'")).await;
    }
    let owner = role
        .as_deref()
        .map_or(String::new(), |r| format!(" OWNER {r}"));
    exec(&admin, format!("CREATE DATABASE {db}{owner}")).await;

    // postgres://user:pass@host:port/dbname[?query] -> swap the credentials and database.
    let (scheme, rest) = base.split_once("://").unwrap();
    let hostport = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    let hostport = hostport.split(['/', '?']).next().unwrap();
    let creds = match (&role, password) {
        (Some(r), Some(p)) => format!("{r}:{p}@"),
        _ => rest
            .rsplit_once('@')
            .map_or(String::new(), |(c, _)| format!("{c}@")),
    };
    Some(FreshDb {
        url: format!("{scheme}://{creds}{hostport}/{db}"),
        admin,
        db,
        role,
    })
}

impl FreshDb {
    pub async fn drop_db(self) {
        exec(&self.admin, format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.db)).await;
        if let Some(role) = &self.role {
            exec(&self.admin, format!("DROP ROLE IF EXISTS {role}")).await;
        }
    }
}

/// [`empty_schema_pool`] with every migration applied.
pub async fn migrated_pool() -> Option<PgPool> {
    use nightfall_api::infrastructure::postgres::{connection_from_pool, Migrator};
    use sea_orm_migration::MigratorTrait;

    let pool = empty_schema_pool().await?;
    Migrator::up(&connection_from_pool(&pool), None)
        .await
        .unwrap();
    Some(pool)
}

/// Inserts an account row directly (foreign-key target for `account_sessions`).
pub async fn insert_account(pool: &PgPool, id: Uuid) {
    sqlx::query("INSERT INTO accounts (id) VALUES ($1)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}
