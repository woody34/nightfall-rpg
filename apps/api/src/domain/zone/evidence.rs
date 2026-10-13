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
    pub items: Option<Vec<ItemEvidenceFact>>,
    /// D46 `bow_launch` facts opened at an accepted bow start and completed with the hit,
    /// critical and spread residues at impact: (resolution id, inputs, actual). Evidence only.
    pub pending_launch: std::collections::BTreeMap<EntityId, (String, Value, Value)>,
}

/// D46 observations captured at their operation boundary, never persisted as state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemEvidenceFact {
    /// Actual subject and life at resolution time.
    pub entity: EntityId,
    /// Incarnation is captured before later death, departure or respawn.
    pub incarnation: u32,
    /// D56 distinguishes private paused work from a live combatant life.
    pub origin: ItemEvidenceOrigin,
    /// Tick containing this observation.
    pub tick: Tick,
    /// Frozen D46 fact kind and payload.
    #[serde(flatten)]
    pub fact: ItemEvidenceData,
}

/// Owner context at the operation boundary; never inferred from later entity state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemEvidenceOrigin {
    /// An admitted combatant with a positive incarnation.
    Live,
    /// A paused character, identified by its character UUID and incarnation zero.
    Offline,
}

/// Typed RNG cursors serialize as full-width JSON integers, including values above u64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemEvidenceData {
    #[serde(rename = "item_baseline")]
    Baseline(Value),
    #[serde(rename = "item_formula")]
    Formula(Value),
    #[serde(rename = "item_delta")]
    Delta(Value),
    #[serde(rename = "item_rng")]
    Rng {
        callsite: String,
        word_before: u128,
        word_after: u128,
        bound: u32,
        value: u32,
        resolution_id: String,
    },
}
