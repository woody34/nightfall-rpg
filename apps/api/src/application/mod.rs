//! Application layer: use cases and the ports they need.
//!
//! A use case is one unit of business work (one gRPC/HTTP endpoint maps to exactly one). It
//! depends only on the domain and on port traits; adapters in `infrastructure` implement the
//! ports. Use cases are unit-tested with in-memory adapters and no I/O.

pub mod checkpoint;
pub mod economy;
pub mod economy_facts;
pub mod economy_reads;
pub mod economy_scheduler;
pub mod economy_zone_reads;
pub mod error;
pub mod ports;
pub mod replay;
pub mod replay_log;
pub mod session;
pub mod trace;
pub mod use_cases;
pub mod zone_actor;
pub mod zone_admission;
pub mod zone_bootstrap;
pub mod zone_registry;

pub use economy_reads::EconomyReadRepository;
pub use economy_zone_reads::ZoneEconomyReads;
pub use error::{parse_id, AppError};
pub use ports::{
    AccountRepository, CharacterCheckpoint, CharacterRepository, CheckpointError,
    CheckpointOutcome, Clock, CreateOutcome, EventBus, IdempotencyKey, ProgressionState,
    SecretGenerator, SessionAudit, SessionAuditContext, SessionRepository, TokenVerifier,
};
