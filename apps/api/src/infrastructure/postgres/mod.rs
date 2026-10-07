//! Postgres adapters via sqlx. See docs/engineering/database-guidelines.md.

mod character_repository;

pub use character_repository::PgCharacterRepository;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Embedded migrations from `apps/api/migrations`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Opens a bounded pool and runs pending migrations.
///
/// Pool size: Postgres performs best with few connections; 10 is plenty for one game server
/// process and keeps us far below the default `max_connections = 100`.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    tracing::info!("postgres connected, migrations applied");
    Ok(pool)
}
