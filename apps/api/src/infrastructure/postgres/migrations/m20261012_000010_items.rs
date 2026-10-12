//! Durable item projections, pending requests, allocator ranges and frozen receipts.
use sea_orm_migration::prelude::*;

/// Additive Phase 4 persistence migration.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[allow(elided_lifetimes_in_paths)] // Match the migration trait signature.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261012_000010_items.sql"))
            .await?;
        Ok(())
    }
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "item persistence cannot be rolled back without losing ownership or audit history"
                .into(),
        ))
    }
}
