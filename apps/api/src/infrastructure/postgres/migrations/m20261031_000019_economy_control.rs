//! Phase 5 container leases, pending deliveries and economy counters (design §11.4, §10.0, B8).
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[allow(elided_lifetimes_in_paths)] // Match the migration trait signature.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261031_000019_economy_control.sql"))
            .await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261031_000019_economy_control_down.sql"))
            .await?;
        Ok(())
    }
}
