//! `nightfall-replay frames`: replays a recording byte-exact and writes one session's replayed
//! WebSocket frames (D26 live-versus-replayed evidence, plan §5.4).
//!
//! The replay is the same check as `verify`: every tick is re-run from the epoch-start snapshot
//! and every selected output must equal the recorded bytes (including V6 `PbOutput41` enriched
//! carriers). Only after a full match are the replayed outputs of `--session` turned into the
//! exact frames a live session sends (`encode_observer_outputs` with the tick's recorded server
//! time) and written as JSON lines:
//! `{"tick":N,"server_time_ms":T,"index":I,"enriched":bool,"kind":"spawn|stats|other","frame_hex":"..."}`.
//! `enriched` marks frames whose output travels in the D26 tag-41 carrier (an `EntitySpawn` or
//! `StatsChanged` with a Phase 4 payload), so a client can compare those bytes with the frames it
//! received live.

use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;
use bytes::Bytes;
use nightfall_api::application::replay::{verify_with, TickRunner, VerifyError, VerifyOptions};
use nightfall_api::application::replay_log::{
    decode_outputs, open_epoch, restore_snapshot, AppliedTickRecord,
};
use nightfall_api::domain::zone::{
    AppliedTick, AppliedTickDraft, EntityId, ObserverOutput, TickError, ZoneEvent, ZoneId,
    ZoneSnapshot, ZoneState,
};
use nightfall_api::infrastructure::eventlog::Recording;
use nightfall_api::interface::zone_mapping::{encode_observer_outputs, observer_output_to_pb};

/// One replayed tick's output for the selected session.
struct TickOutput {
    tick: u64,
    server_time_ms: i64,
    bytes: Bytes,
}

/// Runs the real zone transition and keeps the selected session's replayed output bytes.
struct Capture {
    state: ZoneState,
    zone: ZoneId,
    session: EntityId,
    ticks: Vec<TickOutput>,
}

impl TickRunner for &mut Capture {
    fn replay_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        let applied = self.state.run_tick(draft)?;
        let record = AppliedTickRecord::from_applied(self.zone, &applied);
        if let Some(output) = record.outputs.iter().find(|o| o.entity == self.session) {
            self.ticks.push(TickOutput {
                tick: record.tick.0,
                server_time_ms: record.server_time_ms,
                bytes: output.bytes.clone(),
            });
        }
        Ok(applied)
    }

    fn boundary_snapshot(&self) -> ZoneSnapshot {
        self.state.snapshot()
    }
}

/// The D26 carrier selection: exactly the events the codec writes as `PbOutput41`.
fn enriched_kind(output: &ObserverOutput) -> (bool, &'static str) {
    match output {
        ObserverOutput::Event(ZoneEvent::EntitySpawn { phase4, .. }) => (phase4.is_some(), "spawn"),
        ObserverOutput::Event(ZoneEvent::StatsChanged { phase4, .. }) => {
            (phase4.is_some(), "stats")
        },
        ObserverOutput::Event(_)
        | ObserverOutput::Accepted { .. }
        | ObserverOutput::Rejected(_) => (false, "other"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len().saturating_mul(2)), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Returns the process exit code (0 match and written, 1 divergence, 2 error, 3 incomplete).
pub(super) async fn write(file: &Path, session: EntityId, out: &Path) -> anyhow::Result<u8> {
    if out.exists() {
        anyhow::bail!("frames destination must be fresh: {}", out.display());
    }
    let rec = Recording::read(file)?;
    let (zone, epoch) = (rec.snapshot.seed.zone, rec.snapshot.seed.epoch);
    let log = rec.into_log().await?;
    let opened = match open_epoch(&log, zone, epoch).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(super::INCOMPLETE);
        },
    };
    let state = restore_snapshot(opened.snapshot.snapshot)?;
    let mut capture = Capture {
        state,
        zone,
        session,
        ticks: Vec::new(),
    };
    let opts = VerifyOptions {
        session: Some(session),
        keep_state_before: false,
    };
    match verify_with(&mut capture, opened.records, opts).await {
        Ok(report) if report.players == 0 => {
            eprintln!("session {session} has no output in zone {} epoch {epoch}", zone.0);
            return Ok(super::ERROR);
        },
        Ok(_) => {},
        Err(VerifyError::Diverged(d)) => {
            println!("zone {} epoch {epoch}: {d}", zone.0);
            return Ok(super::DIVERGED);
        },
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(super::ERROR);
        },
    }
    let mut lines = Vec::new();
    let (mut frames, mut enriched) = (0_u64, 0_u64);
    for tick in &capture.ticks {
        let outputs = decode_outputs(&tick.bytes)
            .with_context(|| format!("decode replayed output of tick {}", tick.tick))?;
        let encoded = encode_observer_outputs(&outputs, tick.server_time_ms);
        // encode_observer_outputs is the in-order flat map of observer_output_to_pb.
        let mut flags = Vec::with_capacity(encoded.len());
        for output in &outputs {
            let n = observer_output_to_pb(output, tick.server_time_ms).len();
            flags.extend(std::iter::repeat_n(enriched_kind(output), n));
        }
        anyhow::ensure!(
            flags.len() == encoded.len(),
            "tick {}: {} frames for {} classified messages",
            tick.tick,
            encoded.len(),
            flags.len()
        );
        for (index, (frame, (is_enriched, kind))) in encoded.iter().zip(flags).enumerate() {
            frames = frames.saturating_add(1);
            enriched = enriched.saturating_add(u64::from(is_enriched));
            let line = serde_json::json!({
                "tick": tick.tick,
                "server_time_ms": tick.server_time_ms,
                "index": index,
                "enriched": is_enriched,
                "kind": kind,
                "frame_hex": hex(frame),
            });
            serde_json::to_writer(&mut lines, &line)?;
            lines.push(b'\n');
        }
    }
    let mut f =
        std::fs::File::create_new(out).with_context(|| format!("create {}", out.display()))?;
    f.write_all(&lines)?;
    println!(
        "zone {} epoch {epoch}: match. {frames} replayed frames for session {session} ({enriched} \
         enriched) written to {}",
        zone.0,
        out.display()
    );
    Ok(super::MATCH)
}
