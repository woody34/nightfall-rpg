use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::application::Clock;

/// A clock that only moves when told to. Starts at 2026-01-01T00:00:00Z.
pub struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    /// A clock frozen at `t`.
    #[must_use]
    pub fn at(t: DateTime<Utc>) -> Self {
        Self { now: Mutex::new(t) }
    }

    /// Moves the clock forward (or back, for a negative duration). Saturates: a move past
    /// the representable range leaves the clock where it was.
    pub fn advance(&self, by: chrono::Duration) {
        let mut now = self.now.lock();
        if let Some(t) = now.checked_add_signed(by) {
            *now = t;
        }
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::at(DateTime::from_timestamp(1_767_225_600, 0).unwrap_or_default())
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock()
    }
}
