//! The replay-log policy's metrics, onto the telemetry catalogue.

use crate::application::replay_log::ReplayLogMetrics;
use crate::infrastructure::telemetry::Metrics;

impl ReplayLogMetrics for Metrics {
    fn append_failed(&self) {
        self.eventlog_append_failures_total.add(1, &[]);
    }

    fn zone_paused(&self, paused: bool) {
        self.zones_paused.add(if paused { 1 } else { -1 }, &[]);
    }

    fn audit_dropped(&self, count: u64) {
        self.eventlog_audit_dropped_total.add(count, &[]);
    }
}
