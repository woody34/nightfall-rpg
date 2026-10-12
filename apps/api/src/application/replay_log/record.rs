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
//! * Legacy snapshots and watermarks are canonical **JSON** (`serde_json` over ordered maps).
//!   Phase 2 snapshots retain that complete JSON inside a bounded, versioned zlib envelope:
//!   8-byte `NFSNAP7\0`, little-endian u32 expanded length, one complete zlib stream.
//!   The same bytes go to `JetStream`, Postgres and recordings; decode never loads current data.
//! * NFSNAP9 retains the envelope and limits, with a versioned protobuf body: base JSON,
//!   one content-deduplicated immutable registry, typed sidecars, shared effect headers,
//!   delta-coded allocation IDs and packed source UUIDs. Only V9 uses zlib level nine.

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
    /// State encoding, selected by the record schema (3: JSON v1, 4: binary v2, 5: binary v3, 6: token-policy v4).
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
        // Inspect full reproduced outputs before digest-only comparison hides their fields.
        if self.validate().is_err() || rerun.validate().is_err() {
            return false;
        }
        let rerun = match self.output_form {
            OutputForm::Encoded => rerun.encode(),
            OutputForm::Sha256 => rerun.with_output_digests().encode(),
        };
        rerun == self.encode()
    }

    /// Validates raw nested payloads against this record's immutable policy.
    pub fn validate(&self) -> Result<(), CodecError> {
        Self::decode(&self.encode()).map(|_| ())
    }

    /// Checked encoding for callers accepting externally assembled records.
    pub fn try_encode(&self) -> Result<Vec<u8>, CodecError> {
        let bytes = self.encode();
        Self::decode(&bytes)?;
        Ok(bytes)
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

/// Maximum expanded snapshot body size (JSON for legacy, protobuf for V9).
pub(crate) const MAX_SNAPSHOT_JSON: usize = 32 * 1024 * 1024;
/// Conservative default broker budget, leaving 1 KiB for subject/headers.
pub(crate) const MAX_SNAPSHOT_ENCODED: usize = 1024 * 1024 - 1024;
const PHASE3_SNAPSHOT_MAGIC: &[u8; 8] = b"NFSNAP9\0";
const PHASE4_SNAPSHOT_MAGIC: &[u8; 8] = b"NFSNAP10";
const TOKEN_SNAPSHOT_MAGIC: &[u8; 8] = b"NFSNAP8\0";
const SNAPSHOT_MAGIC: &[u8; 8] = b"NFSNAP7\0";

struct BoundedBytes {
    bytes: Vec<u8>,
    maximum: usize,
}
impl std::io::Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("snapshot exceeds bounded transport allocation"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn bounded_snapshot_json<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    let mut out = BoundedBytes {
        bytes: Vec::new(),
        maximum: MAX_SNAPSHOT_JSON,
    };
    serde_json::to_writer(&mut out, value).map_err(|e| CodecError(e.to_string()))?;
    Ok(out.bytes)
}

/// Canonical snapshot transport. Legacy epochs retain JSON exactly; `BinaryV3` snapshots
/// and `BinaryV4` use JSON envelopes; `BinaryV5` uses a compact protobuf body.
/// All compressed policies retain length-prefixed zlib with the same bounds.
pub fn encode_snapshot(snapshot: &ZoneSnapshot) -> Result<Vec<u8>, CodecError> {
    use std::io::Write as _;
    snapshot
        .validate_version()
        .map_err(|e| CodecError(e.to_string()))?;
    let json = if matches!(
        snapshot.meta.digest_version,
        crate::domain::zone::StateDigestVersion::BinaryV5
            | crate::domain::zone::StateDigestVersion::BinaryV6
    ) {
        super::snapshot_phase3::encode(snapshot)?
    } else {
        let mut json = BoundedBytes {
            bytes: Vec::new(),
            maximum: MAX_SNAPSHOT_JSON,
        };
        serde_json::to_writer(&mut json, snapshot).map_err(|e| CodecError(e.to_string()))?;
        validate_snapshot_json(&json.bytes)?;
        json.bytes
    };
    if !matches!(
        snapshot.meta.digest_version,
        crate::domain::zone::StateDigestVersion::BinaryV3
            | crate::domain::zone::StateDigestVersion::BinaryV4
            | crate::domain::zone::StateDigestVersion::BinaryV5
            | crate::domain::zone::StateDigestVersion::BinaryV6
    ) {
        return Ok(json);
    }
    let mut compressed = flate2::write::ZlibEncoder::new(
        BoundedBytes {
            bytes: Vec::new(),
            maximum: MAX_SNAPSHOT_ENCODED - 12,
        },
        if matches!(
            snapshot.meta.digest_version,
            crate::domain::zone::StateDigestVersion::BinaryV5
                | crate::domain::zone::StateDigestVersion::BinaryV6
        ) {
            flate2::Compression::best()
        } else {
            flate2::Compression::default()
        },
    );
    compressed
        .write_all(&json)
        .map_err(|e| CodecError(e.to_string()))?;
    let body = compressed
        .finish()
        .map_err(|e| CodecError(e.to_string()))?
        .bytes;
    let mut out = Vec::with_capacity(body.len().saturating_add(12));
    out.extend_from_slice(
        if matches!(
            snapshot.meta.digest_version,
            crate::domain::zone::StateDigestVersion::BinaryV5
                | crate::domain::zone::StateDigestVersion::BinaryV6
        ) {
            if snapshot.meta.digest_version == crate::domain::zone::StateDigestVersion::BinaryV6 {
                PHASE4_SNAPSHOT_MAGIC
            } else {
                PHASE3_SNAPSHOT_MAGIC
            }
        } else if snapshot.meta.digest_version == crate::domain::zone::StateDigestVersion::BinaryV4
        {
            TOKEN_SNAPSHOT_MAGIC
        } else {
            SNAPSHOT_MAGIC
        },
    );
    let length = u32::try_from(json.len()).map_err(|e| CodecError(e.to_string()))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&body);
    if out.len() > MAX_SNAPSHOT_ENCODED {
        return Err(CodecError("compressed snapshot exceeds broker budget".into()));
    }
    Ok(out)
}

/// Diagnostic expanded/encoded Snapshot9 byte counts without retaining an over-budget
/// compressed buffer. This does not relax `encode_snapshot` or any transport limit.
pub fn phase3_snapshot_size(snapshot: &ZoneSnapshot) -> Result<(usize, usize), CodecError> {
    use std::io::Write as _;
    #[derive(Default)]
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    snapshot
        .validate_version()
        .map_err(|e| CodecError(e.to_string()))?;
    let body = super::snapshot_phase3::encode(snapshot)?;
    let mut compressed =
        flate2::write::ZlibEncoder::new(Count::default(), flate2::Compression::best());
    compressed
        .write_all(&body)
        .map_err(|e| CodecError(e.to_string()))?;
    let encoded = compressed
        .finish()
        .map_err(|e| CodecError(e.to_string()))?
        .0;
    Ok((body.len(), encoded.saturating_add(12)))
}

/// Reads legacy JSON or a bounded Phase2 envelope. Rejects truncated streams, dishonest
/// length headers, trailing compressed bytes and decompression beyond the declared bound.
pub fn decode_snapshot(bytes: &[u8]) -> Result<ZoneSnapshot, CodecError> {
    if !bytes.starts_with(SNAPSHOT_MAGIC)
        && !bytes.starts_with(TOKEN_SNAPSHOT_MAGIC)
        && !bytes.starts_with(PHASE3_SNAPSHOT_MAGIC)
        && !bytes.starts_with(PHASE4_SNAPSHOT_MAGIC)
    {
        if bytes.len() > MAX_SNAPSHOT_JSON {
            return Err(CodecError("legacy snapshot exceeds 32 MiB".into()));
        }
        validate_snapshot_json(bytes)?;
        let snapshot: ZoneSnapshot =
            serde_json::from_slice(bytes).map_err(|e| CodecError(e.to_string()))?;
        if snapshot.meta.schema_version >= 10 {
            return Err(CodecError("V6 requires hash-resolved snapshot envelope".into()));
        }
        snapshot
            .validate_version()
            .map_err(|e| CodecError(e.to_string()))?;
        return Ok(snapshot);
    }
    if bytes.len() > MAX_SNAPSHOT_ENCODED {
        return Err(CodecError("compressed snapshot exceeds broker budget".into()));
    }
    let size: [u8; 4] = bytes
        .get(8..12)
        .ok_or_else(|| CodecError("truncated snapshot header".into()))?
        .try_into()
        .map_err(|_| CodecError("invalid snapshot header".into()))?;
    let size = u32::from_le_bytes(size) as usize;
    if size > MAX_SNAPSHOT_JSON {
        return Err(CodecError("snapshot expansion exceeds 32 MiB".into()));
    }
    let body = bytes
        .get(12..)
        .ok_or_else(|| CodecError("truncated snapshot".into()))?;
    let mut decoder = flate2::Decompress::new(true);
    let mut json = Vec::with_capacity(size.saturating_add(1));
    let status = decoder
        .decompress_vec(body, &mut json, flate2::FlushDecompress::Finish)
        .map_err(|e| CodecError(e.to_string()))?;
    if status != flate2::Status::StreamEnd
        || json.len() != size
        || decoder.total_in() != body.len() as u64
    {
        return Err(CodecError("snapshot length or compressed stream mismatch".into()));
    }
    let snapshot: ZoneSnapshot =
        if bytes.starts_with(PHASE3_SNAPSHOT_MAGIC) || bytes.starts_with(PHASE4_SNAPSHOT_MAGIC) {
            super::snapshot_phase3::decode(&json)?
        } else {
            validate_snapshot_json(&json)?;
            serde_json::from_slice(&json).map_err(|e| CodecError(e.to_string()))?
        };
    snapshot
        .validate_version()
        .map_err(|e| CodecError(e.to_string()))?;
    let expected = if bytes.starts_with(PHASE4_SNAPSHOT_MAGIC) {
        (10, crate::domain::zone::StateDigestVersion::BinaryV6)
    } else if bytes.starts_with(PHASE3_SNAPSHOT_MAGIC) {
        (9, crate::domain::zone::StateDigestVersion::BinaryV5)
    } else if bytes.starts_with(TOKEN_SNAPSHOT_MAGIC) {
        (8, crate::domain::zone::StateDigestVersion::BinaryV4)
    } else {
        (7, crate::domain::zone::StateDigestVersion::BinaryV3)
    };
    if (snapshot.meta.schema_version, snapshot.meta.digest_version) != expected {
        return Err(CodecError("snapshot envelope/schema mismatch".into()));
    }
    Ok(snapshot)
}

fn validate_snapshot_json(bytes: &[u8]) -> Result<(), CodecError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| CodecError(e.to_string()))?;
    if value
        .get("meta")
        .and_then(|m| m.get("schema_version"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|v| v < 10)
    {
        super::codec_validation::pre_phase4_json(&value)?;
    }
    if value
        .get("meta")
        .and_then(|m| m.get("schema_version"))
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|v| v <= 8)
    {
        super::codec_validation::legacy_json(&value)?;
    }
    Ok(())
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod legacy_learning_boundary_tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn legacy_snapshots_reject_nested_learning_outbox_before_typed_decode() {
        for schema in 4..=8 {
            // Deliberately minimal event proves raw presence rejection before typed
            // DomainEvent decoding (including before the C-owned variant is integrated).
            let value = serde_json::json!({
                "meta": {"schema_version":schema},
                "checkpoints":[{"events":[{"type":"character_skill_learned"}]}]
            });
            let json = serde_json::to_vec(&value).unwrap();
            let error = decode_snapshot(&json).unwrap_err();
            assert!(error.0.contains("Phase 3 event character_skill_learned"), "{error}");
            let mut compressor =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            compressor.write_all(&json).unwrap();
            let compressed = compressor.finish().unwrap();
            for magic in [SNAPSHOT_MAGIC, TOKEN_SNAPSHOT_MAGIC] {
                let mut envelope = magic.to_vec();
                envelope.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
                envelope.extend_from_slice(&compressed);
                let error = decode_snapshot(&envelope).unwrap_err();
                assert!(error.0.contains("Phase 3 event character_skill_learned"), "{error}");
            }
        }
    }

    #[test]
    fn every_legacy_pair_refuses_typed_learning_facts_in_all_snapshot_boundaries() {
        use crate::domain::zone::{
            CheckpointSnapshot, StateDigestVersion as V, Vec2Fixed, ZoneBounds, ZoneSeed, ZoneState,
        };
        use crate::infrastructure::{
            class_data::{load_classes, ClassSource},
            rules_data::{load_rules, RulesSource},
        };
        let rules = load_rules(&RulesSource::embedded()).unwrap().rules;
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        for (schema, version, profession) in [
            (4, V::JsonV1, false),
            (5, V::JsonV1, false),
            (6, V::JsonV1, false),
            (6, V::BinaryV2, false),
            (7, V::JsonV1, false),
            (7, V::BinaryV2, false),
            (7, V::BinaryV3, true),
            (8, V::JsonV1, false),
            (8, V::BinaryV2, false),
            (8, V::BinaryV4, true),
        ] {
            let mut state = ZoneState::new(
                ZoneSeed {
                    zone: ZoneId(1),
                    epoch: 1,
                },
                ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256))
                    .unwrap(),
                0,
            )
            .with_rules(rules.clone());
            if profession {
                state = state.with_classes(classes.clone()).unwrap();
            }
            let mut snapshot = state.snapshot();
            snapshot.meta.schema_version = schema;
            snapshot.meta.digest_version = version;
            ZoneState::from_snapshot(snapshot.clone()).unwrap();
            snapshot.checkpoints.push(CheckpointSnapshot {
                entity: EntityId::from_uuid(Uuid::nil()),
                revision: 1,
                last_saved: Tick(0),
                dirty: true,
                fenced: false,
                latest: None,
                events: vec![crate::domain::DomainEvent::CharacterSkillLearned {
                    metadata: crate::domain::EventMetadata::default(),
                    character_id: crate::domain::CharacterId::from_uuid(Uuid::nil()),
                    skill_id: 3,
                    skill_level: 1,
                    sp_spent: 60,
                    sp_remaining: 0,
                    tick: 0,
                    request_key: Uuid::nil(),
                }],
            });
            assert!(snapshot.validate_version().is_err());
            assert!(encode_snapshot(&snapshot).is_err());
            assert!(decode_snapshot(&serde_json::to_vec(&snapshot).unwrap()).is_err());
            assert!(ZoneState::from_snapshot(snapshot).is_err());
        }
    }

    #[test]
    fn raw_learning_guard_is_versioned_and_does_not_match_player_text() {
        let allowed = serde_json::json!({"meta":{"schema_version":9},
            "checkpoints":[{"events":[{"type":"character_skill_learned"}]}]});
        validate_snapshot_json(&serde_json::to_vec(&allowed).unwrap()).unwrap();
        let legacy = serde_json::json!({"meta":{"schema_version":8},
            "entities":[{"name":"character_skill_learned"}],
            "checkpoints":[{"events":[{"type":"character_class_changed"}]}]});
        validate_snapshot_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    }
}
