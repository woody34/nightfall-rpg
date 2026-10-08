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
