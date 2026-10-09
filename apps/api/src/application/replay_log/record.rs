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
//! * A record whose encoding would exceed what the log accepts per message (`JetStream`'s
//!   `max_payload`) stores each player's output as its SHA-256 instead of the bytes
//!   ([`OutputForm::Sha256`]); replay digests its re-run output and compares digests.
//! * The snapshot and the watermark are canonical **JSON** (`serde_json` over types whose maps
//!   are all ordered), written once per epoch and readable by a human during an incident. The
//!   same snapshot bytes go to `JetStream` and to Postgres.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use super::codec;
use crate::domain::zone::{
    AppliedCommand, AppliedTick, Disposition, EntityId, ObserverOutput, Tick, ZoneEvent, ZoneId,
    ZoneSnapshot,
};

/// A position in the log: the `JetStream` stream sequence (or the in-memory adapter's
/// counter). Ordered within one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(pub u64);

/// One player's ordered output for one tick (see [`encode_outputs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerOutput {
    /// The observing player.
    pub entity: EntityId,
    /// [`encode_outputs`] of the player's ordered output items, or, in a record whose
    /// [`AppliedTickRecord::output_form`] is [`OutputForm::Sha256`], the 32-byte SHA-256 of
    /// those bytes.
    pub bytes: Bytes,
}

/// How an [`AppliedTickRecord`] stores its per-player outputs. Every output of one record has
/// the same form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputForm {
    /// The [`encode_outputs`] bytes themselves.
    #[default]
    Encoded,
    /// SHA-256 of the [`encode_outputs`] bytes: written when the full record would not fit in
    /// one log message. Replay compares digests; a divergence is still detected, but the
    /// stored record cannot show what the player was sent.
    Sha256,
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
    /// Each player's output, in entity-id order, in [`Self::output_form`]. Replay compares
    /// these bytes (see [`Self::reproduced_by`]).
    pub outputs: Vec<PlayerOutput>,
    /// Whether `outputs` (and `events`) hold the encoded bytes or their digests.
    pub output_form: OutputForm,
    /// [`encode_events`] of every zone event of the tick, including facts no observer was
    /// sent (off-AOI combat, hate), or its SHA-256 in [`OutputForm::Sha256`].
    pub events: Bytes,
    /// SHA-256 of the canonical end-of-tick state ([`AppliedTick::state_digest`]).
    pub state_digest: Bytes,
    /// State encoding, selected by the record schema (3: JSON v1, 4: binary v2).
    pub digest_version: crate::domain::zone::StateDigestVersion,
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
            output_form: OutputForm::Encoded,
            events: encode_events(&tick.events),
            state_digest: Bytes::copy_from_slice(&tick.state_digest),
            digest_version: tick.digest_version,
        }
    }

    /// The same record with every player's output replaced by its SHA-256. A record already
    /// in that form is returned unchanged.
    #[must_use]
    pub fn with_output_digests(&self) -> Self {
        let mut digested = self.clone();
        if self.output_form == OutputForm::Encoded {
            for o in &mut digested.outputs {
                o.bytes = Bytes::copy_from_slice(&Sha256::digest(&o.bytes));
            }
            digested.events = Bytes::copy_from_slice(&Sha256::digest(&digested.events));
            digested.output_form = OutputForm::Sha256;
        }
        digested
    }

    /// The record to append to a log that accepts at most `max_bytes` per record: `self` if
    /// its encoding fits, otherwise [`Self::with_output_digests`] (which may still not fit if
    /// the commands alone are too large; the append then fails like any other).
    #[must_use]
    pub fn bounded(self, max_bytes: usize) -> Self {
        if self.encoded_len() <= max_bytes {
            self
        } else {
            self.with_output_digests()
        }
    }

    /// Whether `rerun`, the record of re-running this tick from the log (outputs encoded),
    /// reproduces this one byte for byte. A record stored with [`OutputForm::Sha256`] is
    /// compared with the digests of the re-run's outputs, so a single changed output byte is
    /// still a divergence.
    #[must_use]
    pub fn reproduced_by(&self, rerun: &Self) -> bool {
        let rerun = match self.output_form {
            OutputForm::Encoded => rerun.encode(),
            OutputForm::Sha256 => rerun.with_output_digests().encode(),
        };
        rerun == self.encode()
    }

    /// Length of [`Self::encode`], without encoding.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        codec::record_encoded_len(self)
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

/// Canonical bytes of a tick's zone events, in order.
#[must_use]
pub fn encode_events(events: &[ZoneEvent]) -> Bytes {
    Bytes::from(codec::encode_events(events))
}

/// Inverse of [`encode_events`], for printing a divergence.
pub fn decode_events(bytes: &[u8]) -> Result<Vec<ZoneEvent>, CodecError> {
    codec::decode_events(bytes)
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
    /// Local file capture boundary; does not close the live epoch in the log.
    Capture,
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
    /// The frame's `ClientMessage.seq`, or zero when it could not be decoded. The original
    /// raw frame remains authoritative; the existing uint64 wire field has no presence bit.
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
