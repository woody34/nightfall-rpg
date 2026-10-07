//! Liveness probe that also lets the client measure clock offset.

use std::sync::Arc;

use crate::application::Clock;

/// Output of [`Ping`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingOutput {
    /// Server crate version.
    pub server_version: &'static str,
    /// Server wall clock in Unix milliseconds.
    pub server_time_ms: i64,
}

/// The ping use case.
pub struct Ping {
    clock: Arc<dyn Clock>,
}

impl Ping {
    /// Builds the use case.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }

    /// Executes. Never fails.
    #[must_use]
    pub fn execute(&self, client_version: &str) -> PingOutput {
        tracing::debug!(client_version, "ping");
        PingOutput {
            server_version: env!("CARGO_PKG_VERSION"),
            server_time_ms: self.clock.now().timestamp_millis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;

    struct FixedClock(DateTime<Utc>);
    impl Clock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[test]
    fn returns_clock_time_and_crate_version() {
        let t = DateTime::from_timestamp_millis(1_700_000_000_123).unwrap_or_default();
        let out = Ping::new(Arc::new(FixedClock(t))).execute("0.0.1");
        assert_eq!(out.server_time_ms, 1_700_000_000_123);
        assert_eq!(out.server_version, env!("CARGO_PKG_VERSION"));
    }
}
