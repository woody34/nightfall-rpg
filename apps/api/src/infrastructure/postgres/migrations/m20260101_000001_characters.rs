//! Characters, idempotency keys, and the transactional outbox.

use sea_orm_migration::prelude::*;

/// Creates the initial schema.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// The trait declares `&SchemaManager` with an elided lifetime; matching it exactly is required.
#[allow(elided_lifetimes_in_paths)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Baseline: databases created by the pre-SeaORM sqlx migrator already have this schema
        // (and an `_sqlx_migrations` table we leave alone). Record the migration as applied
        // without touching their data.
        if manager.has_table("characters").await? {
            return Ok(());
        }
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20260101_000001_characters.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS outbox; \
                 DROP TABLE IF EXISTS idempotency_keys; \
                 DROP TABLE IF EXISTS characters;",
            )
            .await?;
        Ok(())
    }
}
