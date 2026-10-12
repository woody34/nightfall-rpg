//! Immutable skill definitions. Ordered XML handlers remain ordered; unknown data is never
//! silently omitted. Phase 3 executability is independently checked by the registry loader.

use super::skill_math::{periodic_interval, ExactRatio};
use crate::domain::class::RuntimeSkillStatus;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

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
    pub raw_attributes: BTreeMap<String, String>,
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

/// Rank-expanded inclusive source level interval (ConditionPlayerLevelRange.java:41–43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PlayerLevelRange {
    pub minimum: u32,
    pub maximum: u32,
}

/// Recursive source conditions. Unsupported leaves remain inspectable deferred metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum SkillCondition {
    PlayerLevelRange {
        ranges: Vec<PlayerLevelRange>,
    },
    PlayerInsideZoneIds {
        zone_ids: Vec<u32>,
    },
    PlayerResting {
        value: bool,
    },
    PlayerStanding {
        value: bool,
    },
    PlayerMoving {
        value: bool,
    },
    PlayerRunning {
        value: bool,
    },
    #[serde(rename = "player_canEscape")]
    PlayerCanEscape {
        value: bool,
    },
    #[serde(rename = "player_flyMounted")]
    PlayerFlyMounted {
        value: bool,
    },
    UsingSlot {
        slot: String,
    },
    PlayerWeight {
        value: i32,
    },
    #[serde(rename = "player_invSize")]
    PlayerInventorySize {
        value: i32,
    },
    UsingKind {
        weapon_kinds: Vec<String>,
    },
    PlayerCanResurrect {
        value: bool,
    },
    And {
        conditions: Vec<SkillCondition>,
        #[serde(default)]
        attrs: BTreeMap<String, String>,
    },
    Or {
        conditions: Vec<SkillCondition>,
        #[serde(default)]
        attrs: BTreeMap<String, String>,
    },
    Not {
        conditions: Vec<SkillCondition>,
        #[serde(default)]
        attrs: BTreeMap<String, String>,
    },
    Unsupported {
        tag: String,
        attrs: BTreeMap<String, String>,
    },
}

impl SkillCondition {
    fn valid_level_ranges(&self, ranks: u32) -> bool {
        match self {
            Self::PlayerLevelRange { ranges } => {
                usize::try_from(ranks).ok() == Some(ranges.len())
                    && ranges.iter().all(|range| range.minimum <= range.maximum)
            },
            Self::And { conditions, .. }
            | Self::Or { conditions, .. }
            | Self::Not { conditions, .. } => conditions
                .iter()
                .all(|child| child.valid_level_ranges(ranks)),
            Self::PlayerInsideZoneIds { .. }
            | Self::PlayerResting { .. }
            | Self::PlayerStanding { .. }
            | Self::PlayerMoving { .. }
            | Self::PlayerRunning { .. }
            | Self::PlayerCanEscape { .. }
            | Self::PlayerFlyMounted { .. }
            | Self::UsingSlot { .. }
            | Self::PlayerWeight { .. }
            | Self::PlayerInventorySize { .. }
            | Self::UsingKind { .. }
            | Self::PlayerCanResurrect { .. }
            | Self::Unsupported { .. } => true,
        }
    }

    fn has_phase4(&self) -> bool {
        match self {
            Self::And { conditions, .. }
            | Self::Or { conditions, .. }
            | Self::Not { conditions, .. } => conditions.iter().any(Self::has_phase4),
            Self::UsingKind { .. } | Self::PlayerCanResurrect { .. } | Self::Unsupported { .. } => {
                false
            },
            Self::PlayerLevelRange { .. }
            | Self::PlayerInsideZoneIds { .. }
            | Self::PlayerResting { .. }
            | Self::PlayerStanding { .. }
            | Self::PlayerMoving { .. }
            | Self::PlayerRunning { .. }
            | Self::PlayerCanEscape { .. }
            | Self::PlayerFlyMounted { .. }
            | Self::UsingSlot { .. }
            | Self::PlayerWeight { .. }
            | Self::PlayerInventorySize { .. } => true,
        }
    }
}

fn all_conditions(skill: &SkillDef) -> impl Iterator<Item = &SkillCondition> {
    skill
        .conditions
        .iter()
        .chain(skill.effects.iter().flat_map(|handler| {
            handler
                .conditions
                .iter()
                .chain(&handler.section_conditions)
                .chain(
                    handler
                        .stat_children
                        .iter()
                        .flat_map(|modifier| &modifier.conditions),
                )
        }))
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
    pub activate_rate: RankValue<i64>,
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
    #[serde(default)]
    pub condition_tables: BTreeMap<String, Vec<String>>,
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

impl SkillDef {
    /// Pinned Skill.java:1225-1226. Hero/GM rank membership is captured from datapack
    /// heroSkillTree.xml:5-9, gameMasterSkillTree.xml:4-49 and
    /// gameMasterAuraSkillTree.xml:4-49 (all rank one), revision 3ca488dd.
    /// These trees are disjoint from today's executable closure; retaining the explicit
    /// check also handles a future overlapping definition without making it stealable.
    pub fn can_be_stolen(&self, rank: u32) -> bool {
        let privileged = rank == 1
            && PRIVILEGED_SKILL_RANGES
                .iter()
                .any(|(start, end)| (*start..=*end).contains(&self.id.0));
        !matches!(self.operate_type.as_str(), "P" | "T")
            && !self.abnormal.as_ref().is_some_and(|a| a.is_debuff)
            && !privileged
            && self.irreplaceable_buff.at(rank, self.max_level) == Some(&false)
            && (self.is_magic != 2
                || self.id.0 == 2341 // CommonSkill.java:52, caravan medicine exception.
                || self.magic_level.get(rank.saturating_sub(1) as usize).is_some_and(|v| *v < 0))
    }
}

/// Frozen source membership used by `can_be_stolen`; every captured tree row is rank one.
/// Ranges compress consecutive IDs only, with no inferred holes. See method provenance.
pub(crate) const PRIVILEGED_SKILL_RANGES: &[(u32, u32)] = &[
    (395, 396),
    (1374, 1376),
    (7029, 7029),
    (7041, 7064),
    (7088, 7096),
    (23238, 23249),
    (23253, 23296),
];

/// An immutable, capability-validated registry; source-only samples are excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRegistry {
    #[serde(skip_serializing_if = "is_legacy_capability")]
    capability_revision: u32,
    skills: BTreeMap<SkillId, SkillDef>,
    hash: [u8; 32],
}

impl SkillRegistry {
    /// Validate immutable definitions for both file admission and snapshot restoration.
    /// Runtime labels never bypass independent per-capability checks.
    pub fn try_new(skills: BTreeMap<SkillId, SkillDef>) -> Result<Self, Vec<String>> {
        Self::try_new_revision(skills, 1)
    }

    /// V6 skill closure. Its distinct capability policy cannot enter a V5 rules bundle.
    pub fn try_new_phase4(skills: BTreeMap<SkillId, SkillDef>) -> Result<Self, Vec<String>> {
        Self::try_new_revision(skills, 2)
    }

    fn try_new_revision(
        skills: BTreeMap<SkillId, SkillDef>,
        capability_revision: u32,
    ) -> Result<Self, Vec<String>> {
        if !matches!(capability_revision, 1 | 2) {
            return Err(vec!["unsupported skill capability revision".into()]);
        }
        let phase4 = capability_revision == 2;
        let mut errors = Vec::new();
        let mut blockers = BTreeMap::new();
        for (id, skill) in &skills {
            validate_skill_shape_revision(skill, &mut errors, phase4);
            for condition in all_conditions(skill) {
                if !condition.valid_level_ranges(skill.max_level) {
                    errors.push(format!("skill {}: invalid rank level ranges", id.0));
                }
            }
            if *id != skill.id
                || id.0 == 0
                || skill.sample_only
                || skill.provenance.skill_id != id.0
                || skill.key != format!("l2.skill.{}", id.0)
                || skill.provenance.revision != "3ca488dd2bd0bfaca43e378886a3c2e37968153a"
                || skill.provenance.repository
                    != "https://bitbucket.org/l2jserver/l2j-server-datapack"
            {
                errors.push(format!("skill {}: identity/provenance/sample mismatch", id.0));
            }
            if !phase4 && all_conditions(skill).any(SkillCondition::has_phase4) {
                errors.push(format!("skill {}: V6 condition in legacy registry", id.0));
            }
            blockers.insert(*id, capabilities(skill, phase4));
        }
        loop {
            let previous = blockers.clone();
            for (id, skill) in &skills {
                let Some(own) = blockers.get_mut(id) else {
                    continue;
                };
                for target in references(skill) {
                    if previous.get(&target).is_none_or(|v| !v.is_empty()) {
                        own.insert(format!("ref:{}", target.0));
                    }
                }
            }
            if blockers == previous {
                break;
            }
        }
        for (id, skill) in &skills {
            let computed = blockers.get(id).is_some_and(BTreeSet::is_empty);
            if computed != (skill.runtime_status == RuntimeSkillStatus::Executable)
                || computed != skill.blockers.is_empty()
            {
                errors.push(format!(
                    "skill {} ({}): runtime_status mismatch; engine blockers {:?}; generated {:?}",
                    id.0,
                    skill.l2_ref,
                    blockers.get(id),
                    skill.blockers
                ));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let bytes = serde_json::to_vec(&skills).map_err(|error| vec![error.to_string()])?;
        let mut hash = Sha256::new();
        hash.update(b"nightfall.skills.1\0");
        hash.update(bytes);
        if phase4 {
            hash.update(b"nightfall.skills.capabilities.2\0");
        }
        Ok(Self {
            capability_revision,
            skills,
            hash: hash.finalize().into(),
        })
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
    /// Separately validated capability grammar; revision one preserves V5 bytes.
    pub const fn capability_revision(&self) -> u32 {
        self.capability_revision
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_legacy_capability(value: &u32) -> bool {
    *value == 1
}

fn present_capability<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    u32::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for SkillRegistry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            #[serde(default, deserialize_with = "present_capability")]
            capability_revision: Option<u32>,
            skills: BTreeMap<SkillId, SkillDef>,
            hash: [u8; 32],
        }
        let snapshot = Snapshot::deserialize(deserializer)?;
        let revision = match snapshot.capability_revision {
            None => 1,
            Some(2) => 2,
            Some(_) => {
                return Err(serde::de::Error::custom("noncanonical skill capability marker"))
            },
        };
        let registry = Self::try_new_revision(snapshot.skills, revision)
            .map_err(|errors| serde::de::Error::custom(errors.join("; ")))?;
        if registry.hash != snapshot.hash {
            return Err(serde::de::Error::custom("skill registry hash mismatch"));
        }
        Ok(registry)
    }
}

pub(crate) fn validate_skill_shape(skill: &SkillDef, errors: &mut Vec<String>) {
    validate_skill_shape_revision(skill, errors, false);
}

pub(crate) fn validate_skill_shape_revision(
    skill: &SkillDef,
    errors: &mut Vec<String>,
    phase4: bool,
) {
    validate_skill_numeric_shape(skill, errors, phase4);
    if !skill.activate_rate.valid_length(skill.max_level)
        || (!phase4 && matches!(skill.activate_rate, RankValue::Levels(_)))
    {
        errors.push(format!("skill {}: invalid/versioned activation rate table", skill.id.0));
    }
    let mut error = |message: &str| errors.push(format!("skill {}: {message}", skill.id.0));
    if skill.max_level == 0 || skill.is_magic > 4 {
        error("invalid rank count or magic classification");
    }
    let count = usize::try_from(skill.max_level).ok();
    for values in [
        &skill.mp_consume_start,
        &skill.mp_consume_end,
        &skill.hp_consume,
        &skill.magic_level,
        &skill.effect_point,
    ] {
        if Some(values.len()) != count {
            error("per-rank array length mismatch");
        }
    }
    if !skill.target_type.valid_length(skill.max_level)
        || !skill.affect_scope.valid_length(skill.max_level)
        || [
            &skill.cast_range,
            &skill.effect_range,
            &skill.hit_time_ms,
            &skill.cool_time_ms,
            &skill.reuse_ms,
            &skill.hit_cancel_ms,
        ]
        .iter()
        .any(|v| !v.valid_length(skill.max_level))
        || skill
            .affect_range
            .as_ref()
            .is_some_and(|v| !v.valid_length(skill.max_level))
    {
        error("scalar/rank table length mismatch");
    }
    if [
        &skill.reuse_delay_lock,
        &skill.stay_after_death,
        &skill.stay_on_subclass_change,
        &skill.irreplaceable_buff,
        &skill.abnormal_instant,
        &skill.removed_on_damage,
        &skill.removed_on_action_except_move,
    ]
    .iter()
    .any(|value| !value.valid_length(skill.max_level))
        || skill
            .source_sets
            .values()
            .any(|value| !value.valid_length(skill.max_level))
        || skill.item_consume.iter().any(|item| {
            !item.item_id.valid_length(skill.max_level) || !item.count.valid_length(skill.max_level)
        })
    {
        error("lifecycle/source/item rank table length mismatch");
    }
    if skill
        .condition_tables
        .values()
        .any(|values| Some(values.len()) != count)
    {
        error("condition rank table length mismatch");
    }
    for rank in 1..=u32::try_from(skill.mp_consume_start.len()).unwrap_or(0) {
        if skill
            .target_type
            .at(rank, skill.max_level)
            .is_some_and(|target| target != "SELF")
            && [&skill.cast_range, &skill.effect_range]
                .iter()
                .any(|range| range.at(rank, skill.max_level) == Some(&0))
        {
            error("zero non-self range: possible unresolved source rank table");
            break;
        }
    }
    if skill.min_chance > skill.max_chance {
        error("invalid chance bounds");
    }
    if skill
        .mp_consume_start
        .iter()
        .chain(&skill.mp_consume_end)
        .chain(&skill.hp_consume)
        .any(|n| *n < 0)
    {
        error("negative resource cost");
    }
    if let Some(abnormal) = &skill.abnormal {
        if Some(abnormal.level.len()) != count || Some(abnormal.time_s.len()) != count {
            error("abnormal rank table length mismatch");
        }
    }
    for handler in &skill.effects {
        if handler
            .params
            .values()
            .any(|p| !p.valid_length(skill.max_level))
            || handler.stat_children.iter().any(|p| {
                Some(p.value.len()) != count
                    || p.order
                        .as_ref()
                        .is_some_and(|order| !order.valid_length(skill.max_level))
            })
        {
            error("handler rank table length mismatch");
        }
    }
}

fn validate_skill_numeric_shape(skill: &SkillDef, errors: &mut Vec<String>, phase4: bool) {
    // V6 also imports blocked source modes (e.g. CallPc's -2 ranges). Retain those
    // signed source integers as metadata; executable and historical bounds stay strict.
    let deferred = phase4 && skill.runtime_status == RuntimeSkillStatus::Deferred;
    let minimum_time = if deferred { i64::from(i32::MIN) } else { 0 };
    let minimum_range = if deferred { i64::from(i32::MIN) } else { -1 };
    let mut error = |message: &str| errors.push(format!("skill {}: {message}", skill.id.0));
    if [
        &skill.hit_time_ms,
        &skill.cool_time_ms,
        &skill.reuse_ms,
        &skill.hit_cancel_ms,
    ]
    .iter()
    .flat_map(|value| value.values())
    .any(|value| !(minimum_time..=i64::from(i32::MAX)).contains(value))
        || [&skill.cast_range, &skill.effect_range]
            .iter()
            .flat_map(|value| value.values())
            .any(|value| !(minimum_range..=i64::from(i32::MAX)).contains(value))
    {
        error("source timing/range outside signed integer bounds");
    }
    if skill.affect_limit.width > 0
        && skill
            .affect_limit
            .offset
            .checked_add(skill.affect_limit.width.saturating_sub(1))
            .is_none_or(|limit| limit > i32::MAX.unsigned_abs())
    {
        error("affectLimit outside positive source integer bounds");
    }
}

fn capabilities(skill: &SkillDef, phase4: bool) -> BTreeSet<String> {
    let mut missing = BTreeSet::new();
    if !["A1", "A2", "A3", "P"].contains(&skill.operate_type.as_str()) {
        missing.insert(format!("operate:{}", skill.operate_type));
    }
    for target in skill.target_type.values() {
        if !(["SELF", "TARGET", "ENEMY", "ENEMY_ONLY", "PC_BODY"].contains(&target.as_str())
            || (phase4 && target == "NONE" && skill.is_magic == 2 && skill.effects.is_empty()))
        {
            missing.insert(format!("target:{target}"));
        }
    }
    for scope in skill.affect_scope.values() {
        if !(["SINGLE", "POINT_BLANK", "RANGE", "unset"].contains(&scope.as_str())
            || (phase4 && scope == "FAN"))
        {
            missing.insert(format!("scope:{scope}"));
        }
    }
    if !["", "ALL", "NOT_FRIEND"].contains(&skill.affect_object.as_str()) {
        missing.insert(format!("scope_filter:{}", skill.affect_object));
    }
    if !phase4 && !skill.item_consume.is_empty() {
        missing.insert("item_consume".into());
    }
    for condition in &skill.conditions {
        condition_capabilities(condition, &mut missing, phase4);
    }
    for handler in &skill.effects {
        if handler.section != "effects" {
            missing.insert(format!("section:{}", handler.section));
        }
        for condition in handler.conditions.iter().chain(&handler.section_conditions) {
            condition_capabilities(condition, &mut missing, phase4);
        }
        handler_capabilities(handler, &mut missing, phase4);
        for modifier in &handler.stat_children {
            for attribute in modifier.raw_attributes.keys() {
                if !["stat", "val", "order"].contains(&attribute.as_str()) {
                    missing.insert(format!("stat_attr:{attribute}"));
                }
            }
            for condition in &modifier.conditions {
                condition_capabilities(condition, &mut missing, phase4);
            }
            if !["mul", "add", "sub"].contains(&modifier.op.as_str()) {
                missing.insert(format!("stat_op:{}", modifier.op));
            }
            if !(STATS.contains(&modifier.stat.as_str())
                || (phase4 && PHASE4_STATS.contains(&modifier.stat.as_str())))
            {
                missing.insert(format!("stat:{}", modifier.stat));
            }
            if modifier.value.iter().any(|v| !numeric(v)) {
                missing.insert(format!("stat_value:{}", modifier.stat));
            }
        }
    }
    missing
}

fn condition_capabilities(
    condition: &SkillCondition,
    missing: &mut BTreeSet<String>,
    phase4: bool,
) {
    match condition {
        SkillCondition::PlayerLevelRange { .. } => {
            // D34: caller support lands before the coordinated generated-table promotion.
            missing.insert("cond:player:levelRange".into());
        },
        SkillCondition::UsingKind { weapon_kinds } => {
            if weapon_kinds.is_empty()
                || weapon_kinds.iter().any(|kind| {
                    !(WEAPONS.contains(&kind.as_str())
                        || (phase4
                            && ["LIGHT", "MAGIC", "HEAVY", "SIGIL", "SHIELD"]
                                .contains(&kind.as_str())))
                })
            {
                missing.insert("condition:weapon_kind".into());
            }
        },
        SkillCondition::PlayerCanResurrect { .. } => {},
        SkillCondition::PlayerInsideZoneIds { zone_ids } => {
            if !phase4 || zone_ids.is_empty() || zone_ids.contains(&0) {
                missing.insert("condition:zone_ids".into());
            }
        },
        SkillCondition::UsingSlot { slot } => {
            if !phase4
                || ![
                    "chest", "lhand", "rhand", "lrhand", "head", "legs", "feet", "gloves",
                ]
                .contains(&slot.as_str())
            {
                missing.insert("condition:using_slot".into());
            }
        },
        SkillCondition::PlayerResting { .. }
        | SkillCondition::PlayerStanding { .. }
        | SkillCondition::PlayerMoving { .. }
        | SkillCondition::PlayerRunning { .. }
        | SkillCondition::PlayerCanEscape { .. }
        | SkillCondition::PlayerFlyMounted { .. }
        | SkillCondition::PlayerWeight { .. }
        | SkillCondition::PlayerInventorySize { .. } => {
            if !phase4 {
                missing.insert("condition:phase4".into());
            }
        },
        SkillCondition::And { conditions, attrs }
        | SkillCondition::Or { conditions, attrs }
        | SkillCondition::Not { conditions, attrs } => {
            if !attrs.is_empty() {
                missing.insert("condition:attributes".into());
            }
            if conditions.is_empty()
                || matches!(condition, SkillCondition::Not { .. }) && conditions.len() != 1
            {
                missing.insert("condition:arity".into());
            }
            for child in conditions {
                condition_capabilities(child, missing, phase4);
            }
        },
        SkillCondition::Unsupported { tag, attrs } => {
            for key in attrs.keys() {
                missing.insert(format!("cond:{tag}:{key}"));
            }
            if attrs.is_empty() {
                missing.insert(format!("cond:{tag}"));
            }
        },
    }
}

fn handler_capabilities(handler: &SkillHandler, missing: &mut BTreeSet<String>, phase4: bool) {
    let allowed: &[&str] = match handler.name.as_str() {
        "FatalBlow" if phase4 => &["power", "criticalChance"],
        "Lethal" if phase4 => &["fullLethal", "halfLethal"],
        "Hp" if phase4 => &["amount", "mode"],
        "Escape" if phase4 => &["escapeType"],
        "PhysicalAttack" => &["power", "criticalChance", "ignoreShieldDefence"],
        "MagicalAttack" | "Heal" | "HpPerMax" | "Resurrection" | "AddHate" => &["power"],
        "MagicalAttackRange" => &["power", "shieldDefensePercent"],
        "MaxHp" | "MaxMp" => &["power", "type", "heal"],
        "TickHp" | "TickMp" => &["power", "mode", "ticks"],
        "Mp" | "Cp" => &["amount", "mode"],
        "DispelBySlot" => &["dispel"],
        "DispelByCategory" => &["max", "slot", "rate"],
        "DefenceTrait" | "AttackTrait" => TRAITS,
        "Stun" | "Sleep" | "Root" | "Buff" | "Debuff" | "Passive" | "GetAgro" => &[],
        _ => {
            missing.insert(format!("effect:{}", handler.name));
            return;
        },
    };
    if matches!(handler.name.as_str(), "TickHp" | "TickMp") && !handler.params.contains_key("ticks")
    {
        missing.insert(format!("param_missing:{}:ticks", handler.name));
    }
    for (key, values) in &handler.params {
        // TraitType.GUST uses the existing name-indexed exact trait fold; V5's
        // admitted trait vocabulary remains unchanged.
        if !(allowed.contains(&key.as_str())
            || (phase4
                && matches!(handler.name.as_str(), "DefenceTrait" | "AttackTrait")
                && key == "GUST"))
        {
            missing.insert(format!("param:{}:{key}", handler.name));
            continue;
        }
        for value in values.values() {
            let valid = match key.as_str() {
                "escapeType" if phase4 => value == "TOWN",
                "mode" | "type" => ["DIFF", "PER"].contains(&value.as_str()),
                "ignoreShieldDefence" | "heal" => ["true", "false"].contains(&value.as_str()),
                "slot" => ["ALL", "BUFF", "DEBUFF"].contains(&value.as_str()),
                "criticalChance" | "max" | "rate" => value.parse::<i32>().is_ok(),
                "ticks" => value
                    .parse::<u32>()
                    .is_ok_and(|v| periodic_interval(v, 666).is_ok()),
                "dispel" => value.split(';').all(|entry| {
                    let (slot, rank) = entry
                        .split_once(',')
                        .map_or((entry, None), |(slot, rank)| (slot, Some(rank)));
                    [
                        "BLEEDING",
                        "POISON",
                        "PARALYZE",
                        "ROOT_PHYSICALLY",
                        "TURN_STONE",
                    ]
                    .contains(&slot)
                        && rank.is_none_or(|rank| rank.parse::<i32>().is_ok())
                }),
                _ => numeric(value),
            };
            if !valid {
                missing.insert(format!("param_value:{}:{key}", handler.name));
            }
        }
    }
}

pub(crate) fn numeric(value: &str) -> bool {
    ExactRatio::parse_literal(value).is_ok()
}

fn references(skill: &SkillDef) -> BTreeSet<SkillId> {
    skill
        .effects
        .iter()
        .flat_map(|handler| handler.params.iter())
        .filter(|(name, _)| name.eq_ignore_ascii_case("skillId"))
        .flat_map(|(_, values)| values.values())
        .filter_map(|v| v.parse::<u32>().ok())
        .map(SkillId)
        .collect()
}

const STATS: &[&str] = &[
    "pAtk",
    "pDef",
    "mAtk",
    "mDef",
    "pAtkSpd",
    "mAtkSpd",
    "runSpd",
    "maxHp",
    "maxMp",
    "maxCp",
    "accCombat",
    "rEvas",
    "critRate",
    "critDmg",
    "critDmgAdd",
    "regHp",
    "regMp",
    "regCp",
    "physicalMpConsumeRate",
    "magicalMpConsumeRate",
    "danceMpConsumeRate",
    "pReuse",
    "mReuse",
    "pSkillEvas",
    "darkRes",
    "darkPower",
    "earthRes",
    "earthPower",
    "fireRes",
    "firePower",
    "holyRes",
    "holyPower",
    "waterRes",
    "waterPower",
    "windRes",
    "windPower",
    "debuffVuln",
    "cancelVuln",
];
const WEAPONS: &[&str] = &[
    "SWORD",
    "BLUNT",
    "DAGGER",
    "DUAL",
    "FIST",
    "DUALFIST",
    "BOW",
    "POLE",
    "RAPIER",
    "ANCIENTSWORD",
    "CROSSBOW",
    "DUALDAGGER",
    "BIGSWORD",
    "BIGBLUNT",
    "NONE",
];
const TRAITS: &[&str] = &[
    "SHOCK",
    "SLEEP",
    "HOLD",
    "PARALYZE",
    "POISON",
    "BLEED",
    "DERANGEMENT",
    "SWORD",
    "BLUNT",
    "DAGGER",
    "POLE",
    "BOW",
    "DUAL",
    "FIST",
    "DUALFIST",
    "RAPIER",
    "ANCIENTSWORD",
    "CROSSBOW",
    "DUALDAGGER",
    "PLANT_WEAKNESS",
    "BEAST_WEAKNESS",
    "BUG_WEAKNESS",
    "DRAGON_WEAKNESS",
    "ANIMAL_WEAKNESS",
];

const PHASE4_STATS: &[&str] = &[
    "sDef",
    "rShld",
    "shieldDefAngle",
    "blowRate",
    "weightLimit",
    "weightPenalty",
    "atkCountMax",
    "pAtkAngle",
    "pAtkRange",
    "STR",
    "DEX",
    "CON",
    "INT",
    "MEN",
    "WIT",
    "mCritRate",
    "physicalSkillPower",
    "mpConsume",
];
