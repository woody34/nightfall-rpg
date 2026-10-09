//! Durable epoch discovery independent of replay retention.

use sea_orm_migration::prelude::*;

/// Adds the durable recovery index.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// The trait declares `&SchemaManager` with an elided lifetime; matching it exactly is required.
#[allow(elided_lifetimes_in_paths)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261009_000006_zone_epochs.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("durable epoch history cannot be rolled back".into()))
    }
}
