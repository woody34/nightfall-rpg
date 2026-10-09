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
use flate2::read::ZlibDecoder;
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

/// Largest inflated body accepted (a week of a busy zone is far below this).
const MAX_BODY: u64 = 1 << 32;

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
        let body = PbRecording {
            snapshot: encode_snapshot(&self.snapshot)?,
            watermark: self.watermark.encode()?,
            records: self.records.iter().map(AppliedTickRecord::encode).collect(),
        }
        .encode_to_vec();
        let mut out = Vec::with_capacity(body.len() / 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&RECORDING_FORMAT_VERSION.to_le_bytes());
        let mut z = ZlibEncoder::new(out, Compression::best());
        z.write_all(&body)?;
        Ok(z.finish()?)
    }

    /// Inverse of [`Self::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
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
        let mut body = Vec::new();
        ZlibDecoder::new(compressed)
            .take(MAX_BODY)
            .read_to_end(&mut body)
            .context("inflate recording")?;
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
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).with_context(|| format!("load {}", path.display()))
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
