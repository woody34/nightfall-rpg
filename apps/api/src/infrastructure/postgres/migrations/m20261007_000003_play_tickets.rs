//! Play tickets, session generations, and generic idempotency keys (Story 1.4).

use sea_orm_migration::prelude::*;

/// Creates `play_tickets` and `account_sessions`; rekeys `idempotency_keys`.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// The trait declares `&SchemaManager` with an elided lifetime; matching it exactly is required.
#[allow(elided_lifetimes_in_paths)]
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("m20261007_000003_play_tickets.sql"))
            .await?;
        Ok(())
    }

    /// Restores the character-only idempotency table. Keys of other operations are dropped:
    /// the old shape cannot represent them.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TABLE IF EXISTS account_sessions; \
                 DROP TABLE IF EXISTS play_tickets; \
                 DELETE FROM idempotency_keys WHERE operation <> 'create_character'; \
                 ALTER TABLE idempotency_keys ADD COLUMN character_id uuid; \
                 UPDATE idempotency_keys SET character_id = (response->>'character_id')::uuid; \
                 ALTER TABLE idempotency_keys \
                     ALTER COLUMN character_id SET NOT NULL, \
                     DROP CONSTRAINT idempotency_keys_pkey, \
                     DROP CONSTRAINT idempotency_keys_operation_format, \
                     DROP COLUMN account_id, \
                     DROP COLUMN operation, \
                     DROP COLUMN response, \
                     ADD CONSTRAINT idempotency_keys_pkey PRIMARY KEY (key);",
            )
            .await?;
        Ok(())
    }
}
