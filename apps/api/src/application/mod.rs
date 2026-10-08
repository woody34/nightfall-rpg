//! Application layer: use cases and the ports they need.
//!
//! A use case is one unit of business work (one gRPC/HTTP endpoint maps to exactly one). It
//! depends only on the domain and on port traits; adapters in `infrastructure` implement the
//! ports. Use cases are unit-tested with in-memory adapters and no I/O.

pub mod error;
pub mod ports;
pub mod session;
pub mod trace;
pub mod use_cases;
pub mod zone_actor;
pub mod zone_registry;

pub use error::AppError;
pub use ports::{
    AccountRepository, CharacterRepository, Clock, CreateOutcome, EventBus, IdempotencyKey,
    SecretGenerator, SessionAudit, SessionRepository, TokenVerifier,
};
