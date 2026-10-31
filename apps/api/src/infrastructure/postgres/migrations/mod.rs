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
mod m20261010_000008_phase3_combat;
mod m20261010_000009_phase3_effect_identity;
mod m20261012_000010_items;
mod m20261012_000011_pending_item_backfill;
mod m20261012_000012_pending_item_scope;
mod m20261012_000013_item_ledger_row_ordinal;
mod m20261014_000014_lane_quarantine;
mod m20261031_000015_economy_ledger_owners;
mod m20261031_000016_economy_items;
mod m20261031_000017_economy_mail;
mod m20261031_000018_economy_auction_manor;
mod m20261031_000019_economy_control;

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
            Box::new(m20261010_000008_phase3_combat::Migration),
            Box::new(m20261010_000009_phase3_effect_identity::Migration),
            Box::new(m20261012_000010_items::Migration),
            Box::new(m20261012_000011_pending_item_backfill::Migration),
            Box::new(m20261012_000012_pending_item_scope::Migration),
            Box::new(m20261012_000013_item_ledger_row_ordinal::Migration),
            Box::new(m20261014_000014_lane_quarantine::Migration),
            Box::new(m20261031_000015_economy_ledger_owners::Migration),
            Box::new(m20261031_000016_economy_items::Migration),
            Box::new(m20261031_000017_economy_mail::Migration),
            Box::new(m20261031_000018_economy_auction_manor::Migration),
            Box::new(m20261031_000019_economy_control::Migration),
        ]
    }
}
