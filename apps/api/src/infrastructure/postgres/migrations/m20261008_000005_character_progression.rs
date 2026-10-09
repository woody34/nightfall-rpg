//! E4.1: progression columns on `characters`.

use sea_orm_migration::prelude::*;

/// Adds progression columns to `characters`.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// The trait declares `&SchemaManager` with an elided lifetime; matching it exactly is required.
#[allow(elided_lifetimes_in_paths)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261008_000005_character_progression.sql"))
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE characters
                    DROP CONSTRAINT characters_level_range,
                    ADD CONSTRAINT characters_level_check CHECK (level BETWEEN 1 AND 80),
                    DROP COLUMN xp, DROP COLUMN hp, DROP COLUMN mp,
                    DROP COLUMN class_profile, DROP COLUMN alive, DROP COLUMN revision;",
            )
            .await?;
        Ok(())
    }
}
