//! Schema migrations, run by `Migrator::up` at startup.
//!
//! Each migration executes plain SQL (kept next to its module) so constraints, partial indexes
//! and `GENERATED ALWAYS AS IDENTITY` stay exactly as reviewed. Entities are generated from the
//! migrated schema (`moon run api:entities`), never hand-edited.

use sea_orm_migration::prelude::*;

mod m20260101_000001_characters;

/// All migrations in application order.
#[derive(Debug, Clone, Copy)]
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20260101_000001_characters::Migration)]
    }
}
