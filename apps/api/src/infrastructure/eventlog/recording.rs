//! A complete epoch in one file (`.nfr`), so replay runs without `JetStream` (CI, bug reports).
//!
//! Layout: magic `NFREPLAY`, format version (`u32` LE), then a zlib stream of the protobuf
//!
//! ```text
//! message Recording {
//!   bytes snapshot = 1;            // encode_snapshot (JSON), as in the log
//!   bytes watermark = 2;           // Watermark::encode (JSON)
//!   repeated bytes records = 3;    // AppliedTickRecord::encode, in tick order
//! }
//! ```
//!
//! The version sits outside the compressed body so a reader refuses a newer file before
//! inflating it. Bump [`RECORDING_FORMAT_VERSION`] on any layout change.

use std::io::{Read as _, Write as _};
use std::path::Path;

use anyhow::{bail, Context as _};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use prost::Message;
use tokio_stream::StreamExt as _;

use super::InMemoryEventLog;
use crate::application::replay_log::{
    decode_snapshot, encode_snapshot, open_epoch, AppliedTickRecord, EventLog, ReplayError,
    Watermark, WatermarkReason,
};
use crate::domain::zone::{ZoneId, ZoneSnapshot};

/// The `.nfr` layout this build reads and writes.
pub const RECORDING_FORMAT_VERSION: u32 = 1;

const MAGIC: &[u8; 8] = b"NFREPLAY";

/// Default offline recording budget; live snapshot decoding has its own tighter bound.
const MAX_BODY: usize = 256 * 1024 * 1024;
/// Explicit legacy opt-in retains the previous maximum without making it the default.
const MAX_LEGACY_BODY: u64 = 1 << 32;

#[derive(Clone, PartialEq, Message)]
struct PbRecording {
    #[prost(bytes = "vec", tag = "1")]
    snapshot: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    watermark: Vec<u8>,
    #[prost(bytes = "vec", repeated, tag = "3")]
    records: Vec<Vec<u8>>,
}

/// One complete epoch: start snapshot, every applied record, completion watermark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// The epoch-start snapshot.
    pub snapshot: ZoneSnapshot,
    /// The completion watermark.
    pub watermark: Watermark,
    /// Every applied record, in tick order.
    pub records: Vec<AppliedTickRecord>,
}

impl Recording {
    /// Reads a complete epoch from `log`. Refuses an incomplete one, like replay.
    pub async fn export(log: &dyn EventLog, zone: ZoneId, epoch: u64) -> Result<Self, ReplayError> {
        let mut opened = open_epoch(log, zone, epoch).await?;
        let mut records = Vec::new();
        while let Some(r) = opened.records.next().await {
            records.push(r?);
        }
        Ok(Self {
            snapshot: opened.snapshot.snapshot,
            watermark: opened.watermark,
            records,
        })
    }

    /// Captures the finite durable prefix currently in the log, without closing the server
    /// epoch. The local watermark marks the capture boundary; nothing is written to NATS.
    pub async fn export_live(log: &dyn EventLog, zone: ZoneId, epoch: u64) -> anyhow::Result<Self> {
        let snapshot = log
            .read_snapshot(zone, epoch)
            .await?
            .with_context(|| format!("zone {} epoch {epoch} has no snapshot", zone.0))?
            .snapshot;
        let mut stream = log.read_epoch(zone, epoch).await?;
        let mut records = Vec::new();
        let mut next = snapshot.tick;
        while let Some(record) = stream.next().await {
            let record = record?;
            if record.zone != zone || record.epoch != epoch || record.tick != next {
                bail!("invalid live prefix: expected zone {} epoch {epoch} tick {}, found zone {} epoch {} tick {}",
                    zone.0, next.0, record.zone.0, record.epoch, record.tick.0);
            }
            next = next.next();
            records.push(record);
        }
        let watermark = Watermark {
            zone,
            epoch,
            last_tick: records.last().map(|r| r.tick),
            records: u64::try_from(records.len())?,
            reason: WatermarkReason::Capture,
        };
        Ok(Self {
            snapshot,
            watermark,
            records,
        })
    }

    /// The `.nfr` bytes.
    pub fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        self.to_bytes_with_limit(MAX_BODY)
    }

    /// Encodes with an explicit inflated-byte budget (up to the historic 4 GiB maximum).
    pub fn to_bytes_with_limit(&self, max_body: usize) -> anyhow::Result<Vec<u8>> {
        compressed_budget(max_body)?;
        let body = PbRecording {
            snapshot: encode_snapshot(&self.snapshot)?,
            watermark: self.watermark.encode()?,
            records: self.records.iter().map(AppliedTickRecord::encode).collect(),
        };
        if body.encoded_len() > max_body {
            bail!("recording exceeds inflated body budget of {max_body} bytes");
        }
        let body = body.encode_to_vec();
        let mut out = Vec::with_capacity(body.len() / 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&RECORDING_FORMAT_VERSION.to_le_bytes());
        let mut z = ZlibEncoder::new(out, Compression::best());
        z.write_all(&body)?;
        Ok(z.finish()?)
    }

    /// Inverse of [`Self::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        Self::from_bytes_with_limit(bytes, MAX_BODY)
    }

    /// Decodes with an explicit inflated-byte budget; no allocations scale beyond the budget.
    pub fn from_bytes_with_limit(bytes: &[u8], max_body: usize) -> anyhow::Result<Self> {
        let max_compressed = compressed_budget(max_body)?;
        let (magic, rest) = bytes
            .split_at_checked(MAGIC.len())
            .context("not a recording")?;
        if magic != MAGIC {
            bail!("not a recording (bad magic)");
        }
        let (version, compressed) = rest.split_at_checked(4).context("truncated header")?;
        let version = u32::from_le_bytes(version.try_into()?);
        if version != RECORDING_FORMAT_VERSION {
            bail!("recording format {version} is not supported (this build reads {RECORDING_FORMAT_VERSION})");
        }
        if compressed.len() > max_compressed {
            bail!("recording exceeds compressed body budget");
        }
        let body = inflate_body(compressed, max_body)?;
        let pb = PbRecording::decode(body.as_slice()).context("decode recording")?;
        Ok(Self {
            snapshot: decode_snapshot(&pb.snapshot)?,
            watermark: Watermark::decode(&pb.watermark)?,
            records: pb
                .records
                .iter()
                .map(|r| AppliedTickRecord::decode(r))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Writes the file.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, self.to_bytes()?).with_context(|| format!("write {}", path.display()))
    }

    /// Reads a file.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let limit =
            std::env::var("NIGHTFALL_RECORDING_MAX_BODY_BYTES").map_or(Ok(MAX_BODY), |value| {
                value
                    .parse::<usize>()
                    .context("invalid recording body budget")
            })?;
        Self::read_with_limit(path, limit)
    }

    /// Bounds both file input and expansion, retaining opt-in support for large historic epochs.
    pub fn read_with_limit(path: &Path, max_body: usize) -> anyhow::Result<Self> {
        let max_file = compressed_budget(max_body)?
            .checked_add(13)
            .context("recording budget overflow")?;
        let file = std::fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
        let mut bytes = Vec::new();
        file.take(u64::try_from(max_file)?)
            .read_to_end(&mut bytes)?;
        Self::from_bytes_with_limit(&bytes, max_body)
            .with_context(|| format!("load {}", path.display()))
    }

    /// The recording as a log, so replay opens it through `open_epoch` like any other source
    /// (same watermark and contiguity checks).
    pub async fn into_log(self) -> anyhow::Result<InMemoryEventLog> {
        let log = InMemoryEventLog::with_max_record_bytes(usize::MAX);
        log.write_snapshot(&self.snapshot).await?;
        for r in &self.records {
            log.append_applied(r).await?;
        }
        log.write_watermark(&self.watermark).await?;
        Ok(log)
    }
}

fn compressed_budget(max_body: usize) -> anyhow::Result<usize> {
    if max_body == 0 || u64::try_from(max_body)? > MAX_LEGACY_BODY {
        bail!("recording body budget must be within 1 byte..=4 GiB");
    }
    max_body
        .checked_add(max_body / 100)
        .and_then(|n| n.checked_add(65536))
        .context("recording budget overflow")
}

fn inflate_body(compressed: &[u8], limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut decoder = flate2::Decompress::new(true);
    let mut body = Vec::new();
    let mut chunk = [0; 16384];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let input = compressed
            .get(usize::try_from(before.0)?..)
            .context("invalid recording stream offset")?;
        let status = decoder
            .decompress(input, &mut chunk, flate2::FlushDecompress::None)
            .context("inflate recording")?;
        if decoder.total_out() > u64::try_from(limit)? {
            bail!("recording exceeds inflated body budget of {limit} bytes");
        }
        let produced = usize::try_from(decoder.total_out().saturating_sub(before.1))?;
        body.extend_from_slice(
            chunk
                .get(..produced)
                .context("invalid recording expansion")?,
        );
        if status == flate2::Status::StreamEnd {
            if decoder.total_in() != u64::try_from(compressed.len())? {
                bail!("trailing recording stream data");
            }
            return Ok(body);
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            bail!("truncated recording stream");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod bounds_tests {
    use super::*;

    #[test]
    fn historical_fixture_round_trips_and_exact_expansion_budget_is_enforced() {
        let recording =
            Recording::from_bytes(include_bytes!("../../../fixtures/sessions/two-players-v4.nfr"))
                .unwrap();
        let body = PbRecording {
            snapshot: encode_snapshot(&recording.snapshot).unwrap(),
            watermark: recording.watermark.encode().unwrap(),
            records: recording
                .records
                .iter()
                .map(AppliedTickRecord::encode)
                .collect(),
        };
        let limit = body.encoded_len();
        let bytes = recording.to_bytes_with_limit(limit).unwrap();
        assert_eq!(Recording::from_bytes_with_limit(&bytes, limit).unwrap(), recording);
        assert!(Recording::from_bytes_with_limit(&bytes, limit - 1)
            .unwrap_err()
            .to_string()
            .contains("inflated body budget"));
        assert!(recording.to_bytes_with_limit(limit - 1).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(Recording::from_bytes(&trailing).is_err());
        assert!(Recording::from_bytes(bytes.get(..bytes.len() - 2).unwrap()).is_err());
    }

    #[test]
    fn bounded_decoder_stops_compression_bombs_and_checks_budget_range() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&vec![0; 1024 * 1024]).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(compressed.len() < 2048);
        assert!(inflate_body(&compressed, 1024)
            .unwrap_err()
            .to_string()
            .contains("inflated body budget"));
        assert!(compressed_budget(0).is_err());
        assert!(compressed_budget(usize::MAX).is_err());
        assert!(compressed_budget(MAX_BODY).is_ok());
    }
}
