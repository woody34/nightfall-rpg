//! Transactional-outbox relay: drains the `outbox` table to `JetStream`.
//!
//! The relay is the only publisher of domain events. See
//! docs/engineering/architecture.md §2.3.

mod publisher;
mod relay;

pub use publisher::{JetStreamPublisher, OutboxPublisher, EVENTS_STREAM};
pub use relay::{OutboxRelay, RelayStats};
