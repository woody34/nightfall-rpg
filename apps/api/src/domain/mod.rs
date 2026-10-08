//! Domain layer: entities, value objects, domain events, and domain errors.
//!
//! Rules (docs/engineering/architecture.md):
//! * No dependency on frameworks, databases, transports, or the async runtime.
//! * Invariants are enforced in constructors so an instance is always valid.
//! * Everything here is unit-testable with plain `cargo test` and no I/O.

pub mod account;
pub mod character;
pub mod events;
pub(crate) mod ids;
pub mod session;
pub mod zone;

pub use account::AccountId;
pub use character::{BaseStats, Character, CharacterId, CharacterName, Position, Race};
pub use events::DomainEvent;
pub use session::{PlayTicket, SessionGeneration, TicketHash};
