//! D76 row-position ledger ordinals replace the Consume ordinal-0 CHECK.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[allow(elided_lifetimes_in_paths)] // Match the migration trait signature.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261012_000013_item_ledger_row_ordinal.sql"))
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261012_000013_item_ledger_row_ordinal_down.sql"))
            .await?;
        Ok(())
    }
}
