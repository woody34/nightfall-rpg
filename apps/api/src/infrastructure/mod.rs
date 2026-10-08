//! Infrastructure layer: adapters that implement application ports.
//!
//! * `memory`: in-process adapters for unit tests, integration tests, and dependency-free dev.
//! * `postgres`: sqlx adapters. Every mutating method is one transaction.
//! * `nats`: event bus over NATS core publish.
//! * `telemetry`: tracing subscriber, OpenTelemetry export, metric catalogue.
//! * `outbox`: relay from the `outbox` table to `JetStream`; the only publisher of domain events.

pub mod memory;
pub mod nats;
pub mod outbox;
pub mod postgres;
pub mod telemetry;

use chrono::{DateTime, Utc};

use crate::application::Clock;

/// The real wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
