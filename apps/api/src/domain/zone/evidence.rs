//! Opt-in replay diagnostics. These values are never part of canonical state or wire output.
use serde::Serialize;
use serde_json::Value;

use super::{EntityId, Tick};

/// One observation at its actual simulation phase boundary, before tick verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceFact {
    /// Stable category, documented by the replay evidence schema.
    pub kind: String,
    /// Subject (caster for resolutions, recipient for resource/life facts).
    pub entity: EntityId,
    /// Simulation tick, never wall time.
    pub tick: Tick,
    /// Exact inputs and reproduced outputs; contains no acceptance verdict.
    pub data: Value,
}

/// One bounded RNG call in execution order. Rejection sampling may advance several words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceDraw {
    /// Source caller, including branch-specific calls that cannot affect the outcome.
    pub callsite: String,
    /// Bound exclusive; zero is never recorded.
    pub bound: u32,
    /// Returned value in [0, bound).
    pub value: u32,
    /// Recipient life (where applicable).
    pub target: Option<super::effects::TargetBinding>,
    /// Generator word cursor before this call, decimal string to retain full width.
    pub word_before: String,
    /// Generator word cursor after this call.
    pub word_after: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EvidenceBuffer {
    pub facts: Vec<EvidenceFact>,
    pub draws: Vec<EvidenceDraw>,
}
