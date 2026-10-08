//! Applies migrations to `DATABASE_URL`, optionally inside a scratch schema.
//!
//! Used by `moon run api:entities` so entity generation runs against a clean, throwaway schema
//! instead of whatever happens to be in the dev database.
//!
//! ```text
//! nightfall-migrate up [SCHEMA]    create SCHEMA (if given) and migrate into it
//! nightfall-migrate drop SCHEMA    drop SCHEMA cascade
//! ```

use anyhow::{bail, Context};
use nightfall_api::infrastructure::postgres::Migrator;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::MigratorTrait;
use sqlx::postgres::PgPoolOptions;

fn valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let schema = args.next();
    if let Some(s) = &schema {
        if !valid_ident(s) {
            bail!("schema must match [a-z0-9_]+");
        }
    }

    // Every pooled connection pins its search_path so migrations (and the seaql_migrations
    // bookkeeping table) land in the scratch schema.
    let search_path = schema.clone();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |conn, _| {
            let search_path = search_path.clone();
            Box::pin(async move {
                if let Some(s) = search_path {
                    // Identifier validated above; quoted regardless.
                    sqlx::query(sqlx::AssertSqlSafe(format!("SET search_path TO \"{s}\"")))
                        .execute(&mut *conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect(&url)
        .await?;
    let db = sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(pool);

    match (cmd.as_str(), schema) {
        ("up", schema) => {
            if let Some(s) = &schema {
                let sql = format!("CREATE SCHEMA IF NOT EXISTS \"{s}\"");
                db.execute_raw(Statement::from_string(db.get_database_backend(), sql))
                    .await?;
            }
            Migrator::up(&db, None).await?;
        },
        ("drop", Some(s)) => {
            let sql = format!("DROP SCHEMA IF EXISTS \"{s}\" CASCADE");
            db.execute_raw(Statement::from_string(db.get_database_backend(), sql))
                .await?;
        },
        _ => bail!("usage: nightfall-migrate up [SCHEMA] | drop SCHEMA"),
    }
    Ok(())
}
