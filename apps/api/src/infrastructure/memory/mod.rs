//! In-memory adapters. Behaviour mirrors the Postgres adapters closely enough that use-case
//! tests written against these hold against the real database.

mod account_repository;
mod character_repository;
mod clock;
mod event_bus;
mod secrets;
mod session_repository;
mod token_verifier;

pub use account_repository::InMemoryAccountRepository;
pub use character_repository::InMemoryCharacterRepository;
pub use clock::ManualClock;
pub use event_bus::InMemoryEventBus;
pub use secrets::{FixedSecretGenerator, SequentialSecretGenerator};
pub use session_repository::InMemorySessionRepository;
pub use token_verifier::TestTokenVerifier;
