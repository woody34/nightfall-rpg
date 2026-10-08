//! The first point where a re-run differs from the recording, and its human-readable report.

use std::fmt::{self, Write as _};
use std::ops::RangeInclusive;

use crate::application::replay_log::{decode_outputs, AppliedTickRecord, OutputForm, PlayerOutput};
use crate::domain::zone::{EntityId, ObserverOutput, Ordinal, Tick, ZoneSnapshot};

/// What differed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    /// `server_time_ms` differs (time must derive from the tick, never a clock).
    ServerTime {
        /// In the log.
        recorded: i64,
        /// From the re-run.
        produced: i64,
    },
    /// A different set of commands was refused.
    Dispositions,
    /// The log has output for a player the re-run sent nothing.
    MissingOutput {
        /// The player.
        entity: EntityId,
    },
    /// The re-run sent output to a player the log has none for.
    UnexpectedOutput {
        /// The player.
        entity: EntityId,
    },
    /// A player's output differs (digest, or bytes).
    Output {
        /// The player.
        entity: EntityId,
    },
}

/// The first divergence of a replay.
#[derive(Debug, Clone)]
pub struct Divergence {
    /// The record as logged.
    pub recorded: AppliedTickRecord,
    /// The record of the re-run tick, outputs encoded.
    pub produced: AppliedTickRecord,
    /// What differed.
    pub mismatch: Mismatch,
    /// The zone before the tick, when `VerifyOptions::keep_state_before` was set.
    pub state_before: Option<ZoneSnapshot>,
}

impl Divergence {
    pub(super) const fn new(
        recorded: AppliedTickRecord,
        produced: AppliedTickRecord,
        mismatch: Mismatch,
        state_before: Option<ZoneSnapshot>,
    ) -> Self {
        Self {
            recorded,
            produced,
            mismatch,
            state_before,
        }
    }

    /// The tick.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.recorded.tick
    }

    /// The player whose output differs, if the mismatch is about one player.
    #[must_use]
    pub const fn session(&self) -> Option<EntityId> {
        match self.mismatch {
            Mismatch::MissingOutput { entity }
            | Mismatch::UnexpectedOutput { entity }
            | Mismatch::Output { entity } => Some(entity),
            Mismatch::ServerTime { .. } | Mismatch::Dispositions => None,
        }
    }

    /// The ordinals the tick applied.
    #[must_use]
    pub fn ordinals(&self) -> Option<RangeInclusive<Ordinal>> {
        let first = self.recorded.commands.first()?.ordinal;
        let last = self.recorded.commands.last()?.ordinal;
        Some(first..=last)
    }

    /// The session's logged output bytes (or digest).
    #[must_use]
    pub fn recorded_output(&self) -> Option<&PlayerOutput> {
        let e = self.session()?;
        self.recorded.outputs.iter().find(|o| o.entity == e)
    }

    /// The session's re-run output bytes.
    #[must_use]
    pub fn produced_output(&self) -> Option<&PlayerOutput> {
        let e = self.session()?;
        self.produced.outputs.iter().find(|o| o.entity == e)
    }

    /// Index of the first output item that differs, and the command ordinal it answers when
    /// it is an ack or a rejection. `None` when the recorded side cannot be decoded.
    #[must_use]
    pub fn first_difference(&self) -> Option<(usize, Option<Ordinal>)> {
        let recorded = self.recorded_items().ok()?;
        let produced = self.produced_items().ok()?;
        let i = (0..recorded.len().max(produced.len()))
            .find(|&i| recorded.get(i) != produced.get(i))?;
        let ordinal = recorded
            .get(i)
            .and_then(response_ordinal)
            .or_else(|| produced.get(i).and_then(response_ordinal));
        Some((i, ordinal))
    }

    fn recorded_items(&self) -> Result<Vec<ObserverOutput>, String> {
        match self.recorded_output() {
            None => Ok(Vec::new()),
            Some(_) if self.recorded.output_form == OutputForm::Sha256 => {
                Err("the record stores output digests only".to_owned())
            },
            Some(o) => decode_outputs(&o.bytes).map_err(|e| e.to_string()),
        }
    }

    fn produced_items(&self) -> Result<Vec<ObserverOutput>, String> {
        self.produced_output()
            .map_or_else(|| Ok(Vec::new()), |o| decode_outputs(&o.bytes).map_err(|e| e.to_string()))
    }
}

const fn response_ordinal(o: &ObserverOutput) -> Option<Ordinal> {
    match o {
        ObserverOutput::Accepted { ordinal, .. } => Some(*ordinal),
        ObserverOutput::Rejected(d) => Some(d.ordinal),
        ObserverOutput::Event(_) => None,
    }
}

/// Lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn write_items(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    output: Option<&PlayerOutput>,
    form: OutputForm,
    items: Result<Vec<ObserverOutput>, String>,
) -> fmt::Result {
    let Some(output) = output else {
        return writeln!(f, "  {label}: (no output)");
    };
    if form == OutputForm::Sha256 {
        return writeln!(f, "  {label}: sha256 {} (digest only)", hex(&output.bytes));
    }
    writeln!(f, "  {label} ({} bytes):", output.bytes.len())?;
    match items {
        Ok(items) => {
            for (i, item) in items.iter().enumerate() {
                writeln!(f, "    [{i}] {item:?}")?;
            }
            Ok(())
        },
        Err(e) => writeln!(f, "    undecodable ({e}): {}", hex(&output.bytes)),
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "divergence at tick {}", self.tick().0)?;
        match self.ordinals() {
            Some(r) => writeln!(f, "  ordinals applied: {}..={}", r.start().0, r.end().0)?,
            None => writeln!(f, "  ordinals applied: none (idle tick)")?,
        }
        if let Some(e) = self.session() {
            writeln!(f, "  session (player entity): {e}")?;
        }
        match self.mismatch {
            Mismatch::ServerTime { recorded, produced } => {
                writeln!(f, "  server_time_ms: recorded {recorded}, produced {produced}")
            },
            Mismatch::Dispositions => {
                writeln!(f, "  dispositions differ")?;
                writeln!(f, "  recorded: {:?}", self.recorded.dispositions)?;
                writeln!(f, "  produced: {:?}", self.produced.dispositions)
            },
            Mismatch::MissingOutput { .. }
            | Mismatch::UnexpectedOutput { .. }
            | Mismatch::Output { .. } => {
                match self.first_difference() {
                    Some((i, Some(o))) => {
                        writeln!(f, "  first difference: item {i}, ordinal {}", o.0)?;
                    },
                    Some((i, None)) => writeln!(f, "  first difference: item {i}")?,
                    None => writeln!(f, "  output digest differs")?,
                }
                write_items(
                    f,
                    "recorded",
                    self.recorded_output(),
                    self.recorded.output_form,
                    self.recorded_items(),
                )?;
                write_items(
                    f,
                    "produced",
                    self.produced_output(),
                    OutputForm::Encoded,
                    self.produced_items(),
                )
            },
        }
    }
}
