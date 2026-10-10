//! Paused Phase 3 combat persistence beside the existing class ledger.

use sea_orm_migration::prelude::*;

/// Adds PvP/death/fraction scalars and normalized retained timer/effect rows.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[allow(elided_lifetimes_in_paths)] // Match the migration trait's signature.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261010_000008_phase3_combat.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "combat checkpoints cannot be rolled back without losing retained state".into(),
        ))
    }
}
