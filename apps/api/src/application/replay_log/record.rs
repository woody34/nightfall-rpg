//! What the replay log stores: one [`AppliedTickRecord`] per tick, the epoch's start
//! [`ZoneSnapshot`], its completion [`Watermark`], and per-session audit frames.
//!
//! Encodings (architecture.md §2.5):
//!
//! * Applied-tick records, per-player outputs and session audit frames are **protobuf**,
//!   encoded with hand-derived `prost` messages (`codec.rs`, schema in its module docs).
//!   `serde` + `bincode`/`postcard` was rejected: the zone's serde enums are internally tagged
//!   (`#[serde(tag = "type")]`), which non-self-describing formats cannot decode, and protobuf
//!   is compact, deterministic for these messages (fields in tag order, no maps) and readable
//!   from any language. Replay compares the encoded bytes.
//! * The snapshot and the watermark are canonical **JSON** (`serde_json` over types whose maps
//!   are all ordered), written once per epoch and readable by a human during an incident. The
//!   same snapshot bytes go to `JetStream` and to Postgres.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::codec;
use crate::domain::zone::{
    AppliedCommand, AppliedTick, Disposition, EntityId, ObserverOutput, Tick, ZoneId, ZoneSnapshot,
};

/// A position in the log: the `JetStream` stream sequence (or the in-memory adapter's
/// counter). Ordered within one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(pub u64);

/// One player's ordered output for one tick, encoded (see [`encode_outputs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerOutput {
    /// The observing player.
    pub entity: EntityId,
    /// [`encode_outputs`] of the player's ordered output items.
    pub bytes: Bytes,
}

/// The replay log's unit (plan §8 #2, #3): everything needed to re-run one tick and check it.
/// The zone actor is the only writer; one record per tick, idle ticks included, so an epoch's
/// records have contiguous tick numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedTickRecord {
    /// The zone.
    pub zone: ZoneId,
    /// The zone's epoch.
    pub epoch: u64,
    /// The tick.
    pub tick: Tick,
    /// `time_origin_ms + tick * 100`, as sent on the wire.
    pub server_time_ms: i64,
    /// Every command in application order, with ordinal and source. Replay feeds these.
    pub commands: Vec<AppliedCommand>,
    /// The refused subset, in ordinal order.
    pub dispositions: Vec<Disposition>,
    /// Each player's encoded output, in entity-id order. Replay compares these bytes.
    pub outputs: Vec<PlayerOutput>,
}

impl AppliedTickRecord {
    /// The record of one tick the zone ran.
    #[must_use]
    pub fn from_applied(zone: ZoneId, tick: &AppliedTick) -> Self {
        Self {
            zone,
            epoch: tick.epoch,
            tick: tick.tick,
            server_time_ms: tick.server_time_ms,
            commands: tick.commands.clone(),
            dispositions: tick.dispositions.clone(),
            outputs: tick
                .outputs
                .iter()
                .map(|(entity, items)| PlayerOutput {
                    entity: *entity,
                    bytes: encode_outputs(items),
                })
                .collect(),
        }
    }

    /// Protobuf encoding. Deterministic: equal records give equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        codec::encode_record(self)
    }

    /// Inverse of [`Self::encode`]. `decode(r.encode()) == r` and re-encoding a decoded record
    /// gives the input bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        codec::decode_record(bytes)
    }
}

/// Canonical bytes of one player's ordered output for a tick. Replay re-runs the tick, encodes
/// its output with this function and compares bytes.
#[must_use]
pub fn encode_outputs(items: &[ObserverOutput]) -> Bytes {
    Bytes::from(codec::encode_outputs(items))
}

/// Inverse of [`encode_outputs`], for printing a divergence.
pub fn decode_outputs(bytes: &[u8]) -> Result<Vec<ObserverOutput>, CodecError> {
    codec::decode_outputs(bytes)
}

/// Canonical JSON of a snapshot: the bytes written to `JetStream` and to `zone_snapshots`.
pub fn encode_snapshot(snapshot: &ZoneSnapshot) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec(snapshot).map_err(|e| CodecError(e.to_string()))
}

/// Inverse of [`encode_snapshot`].
pub fn decode_snapshot(bytes: &[u8]) -> Result<ZoneSnapshot, CodecError> {
    serde_json::from_slice(bytes).map_err(|e| CodecError(e.to_string()))
}

/// Bytes that are not a valid record of the expected kind.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("cannot decode replay-log record: {0}")]
pub struct CodecError(pub String);

/// Why an epoch's log was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkReason {
    /// The process shut down gracefully.
    Shutdown,
    /// The epoch ended while the process kept running.
    EpochEnd,
}

/// The completion marker of an epoch (plan §8 #2). Written after the actor has stopped, so
/// every record up to `last_tick` is in the log. Replay refuses an epoch without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watermark {
    /// The zone.
    pub zone: ZoneId,
    /// The epoch.
    pub epoch: u64,
    /// The last tick whose record was acknowledged; `None` if the epoch never ran a tick.
    pub last_tick: Option<Tick>,
    /// Applied-tick records written in the epoch.
    pub records: u64,
    /// Why the epoch was closed.
    pub reason: WatermarkReason,
}

impl Watermark {
    /// Canonical JSON.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        serde_json::to_vec(self).map_err(|e| CodecError(e.to_string()))
    }

    /// Inverse of [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        serde_json::from_slice(bytes).map_err(|e| CodecError(e.to_string()))
    }
}

/// Whether an epoch can be replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochStatus {
    /// No snapshot: the epoch never started (or has expired from the log).
    Missing,
    /// A snapshot but no watermark: the process died, or is still running. Replay refuses it.
    /// `last_tick` is the last record in the log, if any.
    Incomplete {
        /// Last tick with a record.
        last_tick: Option<Tick>,
    },
    /// Closed with a watermark.
    Complete(Watermark),
}

/// The epoch-start snapshot as stored in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSnapshot {
    /// Where it is in the log. Every record of the epoch comes after it.
    pub seq: Seq,
    /// The snapshot.
    pub snapshot: ZoneSnapshot,
}

/// One inbound frame of a session, for audit (plan D5, §8 #3: the zone log is the replay
/// input; these select and explain).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInRecord {
    /// The session.
    pub session: Uuid,
    /// The frame's `ClientMessage.seq`.
    pub seq: u64,
    /// The zone the session is in.
    pub zone: ZoneId,
    /// That zone's epoch.
    pub epoch: u64,
    /// The zone tick that was next when the frame arrived. The applied log says on which tick
    /// (and with which ordinal) it was applied, or its `Disposition` says why not.
    pub tick_seen: Tick,
    /// Wall-clock receive time, Unix ms. Audit only; never reaches the zone.
    pub recv_unix_ms: i64,
    /// The frame exactly as received.
    pub frame: Bytes,
}

/// One outbound frame of a session, for audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOutRecord {
    /// The session.
    pub session: Uuid,
    /// The zone the session is in.
    pub zone: ZoneId,
    /// That zone's epoch.
    pub epoch: u64,
    /// The tick whose output the frame carries.
    pub tick: Tick,
    /// The frame exactly as sent.
    pub frame: Bytes,
}

impl SessionInRecord {
    /// Protobuf encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        codec::encode_session_in(self)
    }

    /// Inverse of [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        codec::decode_session_in(bytes)
    }
}

impl SessionOutRecord {
    /// Protobuf encoding.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        codec::encode_session_out(self)
    }

    /// Inverse of [`Self::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        codec::decode_session_out(bytes)
    }
}
