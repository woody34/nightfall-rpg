//! Immutable skill definitions. Ordered XML handlers remain ordered; unknown data is never
//! silently omitted. Phase 3 executability is independently checked by the registry loader.

use crate::domain::class::RuntimeSkillStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Source skill identity, shared across learned ranks and reuse timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SkillId(pub u32);

/// A specific learned/source skill rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRef {
    /// Source identity.
    pub id: SkillId,
    /// One-based rank.
    pub rank: u32,
}

/// XML scalar or rank table, preserving exact decimal strings until capability compilation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RankValue<T> {
    /// Constant across all ranks.
    Scalar(T),
    /// Exactly one value per rank; index = rank - 1.
    Levels(Vec<T>),
}

impl<T> RankValue<T> {
    /// The rank value, without a last-element fallback for malformed tables.
    pub fn at(&self, rank: u32, max_rank: u32) -> Option<&T> {
        if rank == 0 || rank > max_rank {
            return None;
        }
        match self {
            Self::Scalar(value) => Some(value),
            Self::Levels(values) => rank
                .checked_sub(1)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| values.get(i)),
        }
    }
    /// All supplied values, in source order, for capability/range validation.
    pub fn values(&self) -> &[T] {
        match self {
            Self::Scalar(value) => std::slice::from_ref(value),
            Self::Levels(values) => values,
        }
    }
    /// Scalar or an exact rank count; no truncation/padding.
    pub fn valid_length(&self, max_rank: u32) -> bool {
        match self {
            Self::Scalar(_) => true,
            Self::Levels(values) => usize::try_from(max_rank).ok() == Some(values.len()),
        }
    }
}

/// Pinned datapack provenance, included in the immutable registry hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillProvenance {
    pub repository: String,
    pub revision: String,
    pub file: String,
    pub skill_id: u32,
}

/// Target count returned by `Skill.getAffectLimit`; width zero means unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct AffectLimit {
    pub offset: u32,
    pub width: u32,
}

/// Generic XML modifier, retaining its explicit priority and source position in the vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillModifier {
    pub op: String,
    pub stat: String,
    pub value: Vec<String>,
    #[serde(default)]
    pub order: Option<RankValue<i32>>,
    #[serde(default)]
    pub conditions: Vec<SkillCondition>,
    #[serde(default)]
    pub raw_conditions: Vec<String>,
}

/// One XML handler, not an effect-list slot. Its parameter set is validated per handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillHandler {
    pub section: String,
    pub name: String,
    pub params: BTreeMap<String, RankValue<String>>,
    pub stat_children: Vec<SkillModifier>,
    #[serde(default)]
    pub conditions: Vec<SkillCondition>,
    #[serde(default)]
    pub raw_conditions: Vec<String>,
    #[serde(default)]
    pub section_conditions: Vec<SkillCondition>,
    #[serde(default)]
    pub section_raw_conditions: Vec<String>,
}

/// Recursive source conditions. Unsupported leaves remain inspectable deferred metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum SkillCondition {
    UsingKind {
        weapon_kinds: Vec<String>,
    },
    PlayerCanResurrect,
    And {
        conditions: Vec<SkillCondition>,
    },
    Or {
        conditions: Vec<SkillCondition>,
    },
    Not {
        conditions: Vec<SkillCondition>,
    },
    Unsupported {
        tag: String,
        attrs: BTreeMap<String, String>,
    },
}

/// Source elemental attack data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillAttribute {
    pub r#type: String,
    pub power: i64,
}

/// One skill-wide abnormal container, regardless of the number of child handlers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillAbnormal {
    pub r#type: String,
    pub level: Vec<u32>,
    pub time_s: Vec<i64>,
    pub is_debuff: bool,
    #[serde(default)]
    pub visual: Option<String>,
}

/// Item costs remain deferred but must survive parsing and capability checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillItemCost {
    pub item_id: RankValue<u32>,
    pub count: RankValue<u32>,
}

/// Complete currently generated skill schema. Missing lifecycle/template source fields are
/// tracked as data-owner requests; this descriptor alone does not authorize an epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillDef {
    pub provenance: SkillProvenance,
    pub id: SkillId,
    pub key: String,
    pub l2_ref: String,
    pub max_level: u32,
    pub operate_type: String,
    pub is_magic: u32,
    pub target_type: RankValue<String>,
    pub affect_scope: RankValue<String>,
    #[serde(default)]
    pub affect_range: Option<RankValue<i64>>,
    #[serde(default)]
    pub affect_limit: AffectLimit,
    #[serde(default)]
    pub affect_object: String,
    pub cast_range: RankValue<i64>,
    pub effect_range: RankValue<i64>,
    pub hit_time_ms: RankValue<i64>,
    pub cool_time_ms: RankValue<i64>,
    pub reuse_ms: RankValue<i64>,
    pub hit_cancel_ms: RankValue<i64>,
    pub mp_consume_start: Vec<i64>,
    pub mp_consume_end: Vec<i64>,
    pub hp_consume: Vec<i64>,
    pub magic_level: Vec<i64>,
    pub effect_point: Vec<i64>,
    pub activate_rate: i64,
    pub basic_property: String,
    pub level_bonus_rate: i64,
    pub r#trait: String,
    pub min_chance: u32,
    pub max_chance: u32,
    pub reuse_delay_lock: RankValue<bool>,
    pub stay_after_death: RankValue<bool>,
    pub stay_on_subclass_change: RankValue<bool>,
    pub irreplaceable_buff: RankValue<bool>,
    pub abnormal_instant: RankValue<bool>,
    pub removed_on_damage: RankValue<bool>,
    pub removed_on_action_except_move: RankValue<bool>,
    pub source_sets: BTreeMap<String, RankValue<String>>,
    pub next_action_attack: bool,
    pub has_enchant_routes: bool,
    pub runtime_status: RuntimeSkillStatus,
    pub blockers: Vec<String>,
    #[serde(default)]
    pub sample_only: bool,
    #[serde(default)]
    pub conditions: Vec<SkillCondition>,
    #[serde(default)]
    pub raw_conditions: Vec<String>,
    pub attribute: SkillAttribute,
    #[serde(default)]
    pub abnormal: Option<SkillAbnormal>,
    #[serde(default)]
    pub item_consume: Vec<SkillItemCost>,
    #[serde(default)]
    pub effects: Vec<SkillHandler>,
}

/// An immutable, capability-validated registry; source-only samples are excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRegistry {
    skills: BTreeMap<SkillId, SkillDef>,
    hash: [u8; 32],
}

impl SkillRegistry {
    /// Build only after loader provenance, shape, capability and reference checks succeed.
    pub(crate) fn validated(skills: BTreeMap<SkillId, SkillDef>, hash: [u8; 32]) -> Self {
        Self { skills, hash }
    }
    /// Exact definition by id.
    pub fn get(&self, id: SkillId) -> Option<&SkillDef> {
        self.skills.get(&id)
    }
    /// Stable id order for snapshots and catalogue projections.
    pub fn skills(&self) -> &BTreeMap<SkillId, SkillDef> {
        &self.skills
    }
    /// Canonical normalized definitions hash.
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }
}
