//! Schema migrations, run by `Migrator::up` at startup.
//!
//! Each migration executes plain SQL (kept next to its module) so constraints, partial indexes
//! and `GENERATED ALWAYS AS IDENTITY` stay exactly as reviewed. Entities are generated from the
//! migrated schema (`moon run api:entities`), never hand-edited.

use sea_orm_migration::prelude::*;

mod m20260101_000001_characters;
mod m20261007_000002_accounts;
mod m20261007_000003_play_tickets;
mod m20261007_000004_zone_snapshots;
mod m20261008_000005_character_progression;
mod m20261009_000006_zone_epochs;
mod m20261009_000007_character_classes;

/// All migrations in application order.
#[derive(Debug, Clone, Copy)]
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260101_000001_characters::Migration),
            Box::new(m20261007_000002_accounts::Migration),
            Box::new(m20261007_000003_play_tickets::Migration),
            Box::new(m20261007_000004_zone_snapshots::Migration),
            Box::new(m20261008_000005_character_progression::Migration),
            Box::new(m20261009_000006_zone_epochs::Migration),
            Box::new(m20261009_000007_character_classes::Migration),
        ]
    }
}
