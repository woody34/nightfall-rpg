//! Replay-log adapters (architecture.md §2.5): `JetStream` for real, memory for tests.
//!
//! Subjects (token counts never collide with `NF_EVENTS`' `nightfall.*.*`):
//!
//! | Subject | Stream | Payload |
//! |---------|--------|---------|
//! | `nightfall.zone.<zone>.<epoch>.snapshot` | `NF_ZONES` | JSON `ZoneSnapshot` |
//! | `nightfall.zone.<zone>.<epoch>.applied` | `NF_ZONES` | protobuf `AppliedTickRecord`, one per tick |
//! | `nightfall.zone.<zone>.<epoch>.watermark` | `NF_ZONES` | JSON `Watermark` |
//! | `nightfall.session.<session>.in` / `.out` | `NF_SESSIONS` | protobuf audit frames |

mod jetstream;
mod memory;
mod metrics;

pub use jetstream::{JetStreamEventLog, RETENTION, SESSIONS_STREAM, ZONES_STREAM};
pub use memory::{InMemoryEventLog, InMemoryZoneSnapshotStore};

use uuid::Uuid;

use crate::domain::zone::{Tick, ZoneId};

fn zone_prefix(zone: ZoneId, epoch: u64) -> String {
    format!("nightfall.zone.{}.{epoch}", zone.0)
}

/// Subject of an epoch's applied-tick records.
#[must_use]
pub fn applied_subject(zone: ZoneId, epoch: u64) -> String {
    format!("{}.applied", zone_prefix(zone, epoch))
}

/// Subject of an epoch's start snapshot.
#[must_use]
pub fn snapshot_subject(zone: ZoneId, epoch: u64) -> String {
    format!("{}.snapshot", zone_prefix(zone, epoch))
}

/// Subject of an epoch's completion watermark.
#[must_use]
pub fn watermark_subject(zone: ZoneId, epoch: u64) -> String {
    format!("{}.watermark", zone_prefix(zone, epoch))
}

/// Subject of one session's audit frames in one direction (`in` or `out`).
#[must_use]
pub fn session_subject(session: Uuid, direction: &str) -> String {
    format!("nightfall.session.{}.{direction}", session.simple())
}

/// `Nats-Msg-Id` of a tick's record: `<zone>/<epoch>/<tick>`. A retried append of the same
/// tick is dropped by the broker inside the duplicate window.
#[must_use]
pub fn applied_msg_id(zone: ZoneId, epoch: u64, tick: Tick) -> String {
    format!("{}/{epoch}/{}", zone.0, tick.0)
}

/// The epoch of a `nightfall.zone.<zone>.<epoch>.snapshot` subject, if it is one for `zone`.
fn snapshot_epoch(zone: ZoneId, subject: &str) -> Option<u64> {
    let rest = subject.strip_prefix(&format!("nightfall.zone.{}.", zone.0))?;
    rest.strip_suffix(".snapshot")?.parse().ok()
}
