//! Infrastructure layer: adapters that implement application ports.
//!
//! * `auth`: bearer-token verification (Keycloak JWKS).
//! * `memory`: in-process adapters for unit tests, integration tests, and dependency-free dev.
//! * `postgres`: sqlx adapters. Every mutating method is one transaction.
//! * `nats`: event bus over NATS core publish.
//! * `telemetry`: tracing subscriber, OpenTelemetry export, metric catalogue.
//! * `outbox`: relay from the `outbox` table to `JetStream`; the only publisher of domain events.
//! * `secrets`: the OS random source for play tickets.
//! * `eventlog`: the zone replay log on `JetStream` (and in memory).
//! * `zone_data`: zone definitions from `packages/data/zones/`.

pub mod auth;
pub mod eventlog;
pub mod memory;
pub mod nats;
pub mod outbox;
pub mod postgres;
pub mod secrets;
pub mod telemetry;
pub mod zone_data;

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
