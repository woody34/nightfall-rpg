//! Replay verification (Story 3.3; architecture.md §2.5, "Replay tool").
//!
//! Rebuilds a zone from its epoch-start snapshot, feeds every applied record's commands in
//! ordinal order through [`TickRunner::replay_tick`] (no actor, no clock: the records are the tick
//! source), re-encodes each player's output with the log's codec and compares it with the
//! recorded output: SHA-256 first, then the bytes when the record holds them. Zone-wide events
//! (including off-AOI facts) and the end-of-tick state digest are compared too. The first
//! mismatch stops the run and is returned as a [`Divergence`].

mod divergence;

use std::collections::BTreeSet;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_stream::{Stream, StreamExt};

pub use divergence::{Divergence, Mismatch};

use super::replay_log::{AppliedTickRecord, OutputForm, PlayerOutput, RecordedEpoch, ReplayError};
use crate::domain::zone::{
    AppliedTick, AppliedTickDraft, EntityId, SnapshotError, TickError, ZoneSnapshot, ZoneState,
};

/// The transition replay drives. [`ZoneState`] in production; tests substitute doubles to
/// prove a nondeterministic implementation is caught.
pub trait TickRunner {
    /// Applies one tick (see [`ZoneState::run_tick`]).
    fn replay_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError>;

    /// The state at the current tick boundary.
    fn boundary_snapshot(&self) -> ZoneSnapshot;
}

impl TickRunner for ZoneState {
    fn replay_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        self.run_tick(draft)
    }

    fn boundary_snapshot(&self) -> ZoneSnapshot {
        self.snapshot()
    }
}

/// What to verify.
#[derive(Debug, Clone, Copy, Default)]
pub struct VerifyOptions {
    /// Compare only this player's outputs. Every command is still applied: selecting a
    /// session never filters inputs (plan §8 #3).
    pub session: Option<EntityId>,
    /// Keep the state before each tick so a [`Divergence`] can carry it (costs one snapshot
    /// per tick).
    pub keep_state_before: bool,
}

/// A replay that matched the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplayReport {
    /// Ticks re-run.
    pub ticks: u64,
    /// Distinct players whose outputs were compared.
    pub players: usize,
    /// Per-player outputs compared.
    pub outputs: u64,
    /// Output bytes compared byte for byte (records with full outputs).
    pub bytes_compared: u64,
    /// Outputs compared by digest only (records stored with [`OutputForm::Sha256`]).
    pub digest_only: u64,
}

/// Why a replay did not finish with a match.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// The re-run produced something the recording does not have.
    #[error("{0}")]
    Diverged(Box<Divergence>),
    /// The log is incomplete, has a gap, or cannot be read.
    #[error(transparent)]
    Log(#[from] ReplayError),
    /// The snapshot does not describe a valid zone.
    #[error("invalid snapshot: {0}")]
    Snapshot(#[from] SnapshotError),
    /// A record does not continue the zone (wrong epoch, tick or ordinals).
    #[error("tick {tick}: record does not continue the zone: {source}")]
    Tick {
        /// The record's tick.
        tick: u64,
        /// What `run_tick` refused.
        source: TickError,
    },
}

/// Replays a complete epoch against [`ZoneState`].
pub async fn verify_epoch(
    epoch: RecordedEpoch,
    opts: VerifyOptions,
) -> Result<ReplayReport, VerifyError> {
    let state = ZoneState::from_snapshot(epoch.snapshot.snapshot)?;
    verify_with(state, epoch.records, opts).await
}

/// Replays `records` (in tick order, as `open_epoch` yields them) against `runner`.
pub async fn verify_with<R, S>(
    mut runner: R,
    mut records: S,
    opts: VerifyOptions,
) -> Result<ReplayReport, VerifyError>
where
    R: TickRunner,
    S: Stream<Item = Result<AppliedTickRecord, ReplayError>> + Unpin,
{
    let mut report = ReplayReport::default();
    let mut players = BTreeSet::new();
    while let Some(record) = records.next().await {
        let record = record?;
        let before = opts.keep_state_before.then(|| runner.boundary_snapshot());
        let draft = AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        };
        let applied = runner
            .replay_tick(draft)
            .map_err(|source| VerifyError::Tick {
                tick: record.tick.0,
                source,
            })?;
        let rerun = AppliedTickRecord::from_applied(record.zone, &applied);
        if let Err(mismatch) = compare(&record, &rerun, opts.session, &mut report, &mut players) {
            return Err(VerifyError::Diverged(Box::new(Divergence::new(
                record, rerun, mismatch, before,
            ))));
        }
        report.ticks = report.ticks.saturating_add(1);
    }
    report.players = players.len();
    Ok(report)
}

/// Compares one tick. Record-level fields first (time, dispositions, who got output), then
/// each selected player's output.
fn compare(
    recorded: &AppliedTickRecord,
    rerun: &AppliedTickRecord,
    session: Option<EntityId>,
    report: &mut ReplayReport,
    players: &mut BTreeSet<EntityId>,
) -> Result<(), Mismatch> {
    if recorded.server_time_ms != rerun.server_time_ms {
        return Err(Mismatch::ServerTime {
            recorded: recorded.server_time_ms,
            produced: rerun.server_time_ms,
        });
    }
    if recorded.dispositions != rerun.dispositions {
        return Err(Mismatch::Dispositions);
    }
    let events_match = match recorded.output_form {
        OutputForm::Encoded => recorded.events == rerun.events,
        OutputForm::Sha256 => recorded.events.as_ref() == Sha256::digest(&rerun.events).as_slice(),
    };
    if !events_match {
        return Err(Mismatch::Events);
    }
    if recorded.state_digest != rerun.state_digest {
        return Err(Mismatch::StateDigest);
    }
    let selected = |e: &EntityId| session.is_none_or(|s| s == *e);
    let recorded_ids: Vec<EntityId> = recorded.outputs.iter().map(|o| o.entity).collect();
    let rerun_ids: Vec<EntityId> = rerun.outputs.iter().map(|o| o.entity).collect();
    if let Some(&entity) = recorded_ids
        .iter()
        .filter(|e| selected(e))
        .find(|e| !rerun_ids.contains(e))
    {
        return Err(Mismatch::MissingOutput { entity });
    }
    if let Some(&entity) = rerun_ids
        .iter()
        .filter(|e| selected(e))
        .find(|e| !recorded_ids.contains(e))
    {
        return Err(Mismatch::UnexpectedOutput { entity });
    }
    for want in recorded.outputs.iter().filter(|o| selected(&o.entity)) {
        let Some(got) = rerun.outputs.iter().find(|o| o.entity == want.entity) else {
            continue; // unreachable: the id sets were checked above
        };
        compare_output(want, recorded.output_form, &got.bytes, report)?;
        players.insert(want.entity);
    }
    Ok(())
}

fn compare_output(
    want: &PlayerOutput,
    form: OutputForm,
    produced: &Bytes,
    report: &mut ReplayReport,
) -> Result<(), Mismatch> {
    let produced_digest = Sha256::digest(produced);
    let same = match form {
        OutputForm::Encoded => {
            Sha256::digest(&want.bytes) == produced_digest && want.bytes == *produced
        },
        OutputForm::Sha256 => want.bytes.as_ref() == produced_digest.as_slice(),
    };
    if !same {
        return Err(Mismatch::Output {
            entity: want.entity,
        });
    }
    report.outputs = report.outputs.saturating_add(1);
    match form {
        OutputForm::Encoded => {
            let n = u64::try_from(produced.len()).unwrap_or(u64::MAX);
            report.bytes_compared = report.bytes_compared.saturating_add(n);
        },
        OutputForm::Sha256 => report.digest_only = report.digest_only.saturating_add(1),
    }
    Ok(())
}
