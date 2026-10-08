//! The `zone_snapshots` epoch index (Story 3.4).

use sea_orm_migration::prelude::*;

/// Creates `zone_snapshots`.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// The trait declares `&SchemaManager` with an elided lifetime; matching it exactly is required.
#[allow(elided_lifetimes_in_paths)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261007_000004_zone_snapshots.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS zone_snapshots;")
            .await?;
        Ok(())
    }
}
