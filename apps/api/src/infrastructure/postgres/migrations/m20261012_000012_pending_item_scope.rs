//! Account/operation pending identity; no direct Grant completion rows (D41).
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[allow(elided_lifetimes_in_paths)] // Match the migration trait signature.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261012_000012_pending_item_scope.sql"))
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261012_000012_pending_item_scope_down.sql"))
            .await?;
        Ok(())
    }
}
