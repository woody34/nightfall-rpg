//! In-memory adapters. Behaviour mirrors the Postgres adapters closely enough that use-case
//! tests written against these hold against the real database.

mod character_repository;
mod event_bus;

pub use character_repository::InMemoryCharacterRepository;
pub use event_bus::InMemoryEventBus;
