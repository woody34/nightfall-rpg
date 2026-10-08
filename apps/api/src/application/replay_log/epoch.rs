//! The epoch lifecycle on the log side: starting one (snapshot first) and opening a finished
//! one for replay (watermark required, ticks contiguous).

use std::pin::Pin;
use std::task::{Context, Poll};

use thiserror::Error;
use tokio_stream::Stream;

use super::port::{EventLog, RecordStream};
use super::record::{AppliedTickRecord, EpochStatus, Seq, StoredSnapshot, Watermark};
use crate::domain::zone::{Tick, ZoneId, ZoneSnapshot};

/// Proof that an epoch's start snapshot is durably in the log. The only way to get one is
/// [`start_epoch`], and `DurableTickGate::new` requires one, so no applied record of an epoch
/// can be written before its snapshot (plan §8 #5).
#[derive(Debug, Clone)]
pub struct EpochStarted {
    zone: ZoneId,
    epoch: u64,
    first_tick: Tick,
    snapshot_seq: Seq,
}

impl EpochStarted {
    /// The zone.
    #[must_use]
    pub const fn zone(&self) -> ZoneId {
        self.zone
    }

    /// The epoch.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The tick the snapshot was taken before: the epoch's first record must be for it.
    #[must_use]
    pub const fn first_tick(&self) -> Tick {
        self.first_tick
    }

    /// Where the snapshot is in the log.
    #[must_use]
    pub const fn snapshot_seq(&self) -> Seq {
        self.snapshot_seq
    }
}

/// Writes the epoch-start snapshot and returns the proof the gate needs. `snapshot` must be
/// the zone's state at a tick boundary with nothing pending (a fresh `ZoneState` or
/// `ZoneHandle::snapshot`).
pub async fn start_epoch(
    log: &dyn EventLog,
    snapshot: &ZoneSnapshot,
) -> anyhow::Result<EpochStarted> {
    let snapshot_seq = log.write_snapshot(snapshot).await?;
    Ok(EpochStarted {
        zone: snapshot.seed.zone,
        epoch: snapshot.seed.epoch,
        first_tick: snapshot.tick,
        snapshot_seq,
    })
}

/// Why an epoch cannot be replayed.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// No snapshot in the log.
    #[error("zone {zone} epoch {epoch} has no snapshot in the log")]
    Missing {
        /// The zone.
        zone: u32,
        /// The epoch.
        epoch: u64,
    },
    /// No completion watermark: the recording may be cut short (plan §8 #2).
    #[error("zone {zone} epoch {epoch} has no completion watermark (last tick {last_tick:?})")]
    Incomplete {
        /// The zone.
        zone: u32,
        /// The epoch.
        epoch: u64,
        /// The last tick in the log, if any.
        last_tick: Option<Tick>,
    },
    /// A record is missing or out of order.
    #[error("expected the record of tick {expected:?}, found tick {found:?}")]
    Gap {
        /// The tick that should have come next.
        expected: Tick,
        /// The tick that did.
        found: Tick,
    },
    /// A record belongs to another zone or epoch.
    #[error("record for zone {zone} epoch {epoch} in the wrong epoch's log")]
    Foreign {
        /// Its zone.
        zone: u32,
        /// Its epoch.
        epoch: u64,
    },
    /// The log ends before the watermark's last tick.
    #[error("log ends before tick {expected_last:?} named by the watermark")]
    Truncated {
        /// The watermark's last tick.
        expected_last: Option<Tick>,
    },
    /// The log could not be read.
    #[error(transparent)]
    Log(#[from] anyhow::Error),
}

/// A complete epoch, ready for replay.
pub struct RecordedEpoch {
    /// The start snapshot.
    pub snapshot: StoredSnapshot,
    /// The completion watermark.
    pub watermark: Watermark,
    /// The records from the snapshot's tick to the watermark's, checked as they stream:
    /// a gap, a foreign record or an early end yields one `Err` and then ends.
    pub records: CheckedRecords,
}

/// Opens an epoch for replay. Refuses one without a watermark ([`EpochStatus::Incomplete`]).
pub async fn open_epoch(
    log: &dyn EventLog,
    zone: ZoneId,
    epoch: u64,
) -> Result<RecordedEpoch, ReplayError> {
    let watermark = match log.epoch_status(zone, epoch).await? {
        EpochStatus::Missing => {
            return Err(ReplayError::Missing {
                zone: zone.0,
                epoch,
            })
        },
        EpochStatus::Incomplete { last_tick } => {
            return Err(ReplayError::Incomplete {
                zone: zone.0,
                epoch,
                last_tick,
            })
        },
        EpochStatus::Complete(w) => w,
    };
    let snapshot = log
        .read_snapshot(zone, epoch)
        .await?
        .ok_or(ReplayError::Missing {
            zone: zone.0,
            epoch,
        })?;
    let records = CheckedRecords {
        inner: log.read_epoch(zone, epoch).await?,
        zone,
        epoch,
        next: snapshot.snapshot.tick,
        last: watermark.last_tick,
        finished: false,
    };
    Ok(RecordedEpoch {
        snapshot,
        watermark,
        records,
    })
}

/// An epoch's records, checked for contiguity against the snapshot and the watermark.
pub struct CheckedRecords {
    inner: RecordStream,
    zone: ZoneId,
    epoch: u64,
    next: Tick,
    last: Option<Tick>,
    finished: bool,
}

impl CheckedRecords {
    fn check(&mut self, r: AppliedTickRecord) -> Result<AppliedTickRecord, ReplayError> {
        if r.zone != self.zone || r.epoch != self.epoch {
            return Err(ReplayError::Foreign {
                zone: r.zone.0,
                epoch: r.epoch,
            });
        }
        if r.tick != self.next {
            return Err(ReplayError::Gap {
                expected: self.next,
                found: r.tick,
            });
        }
        self.next = r.tick.next();
        Ok(r)
    }

    /// Whether every record up to the watermark's last tick has been yielded.
    fn reached_watermark(&self) -> bool {
        match self.last {
            None => true,
            Some(last) => self.next > last,
        }
    }
}

impl Stream for CheckedRecords {
    type Item = Result<AppliedTickRecord, ReplayError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        // Records after the watermark (none are expected) are ignored: the watermark defines
        // the epoch.
        if self.reached_watermark() {
            self.finished = true;
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(r))) => {
                let checked = self.check(r);
                if checked.is_err() {
                    self.finished = true;
                }
                Poll::Ready(Some(checked))
            },
            Poll::Ready(Some(Err(e))) => {
                self.finished = true;
                Poll::Ready(Some(Err(ReplayError::Log(e))))
            },
            Poll::Ready(None) => {
                self.finished = true;
                if self.reached_watermark() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Err(ReplayError::Truncated {
                        expected_last: self.last,
                    })))
                }
            },
        }
    }
}
