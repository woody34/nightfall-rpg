//! Validated, immutable race and profession definitions, independent of storage and transport.
//! Source growth is exact at the Phase 1 decimal scale; classes may inherit a parent's table.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::zone::{Archetype, Scaled};
use super::{BaseStats, Race};

/// Retail-compatible profession identifier. Kamael's 123..=136 remain reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClassId(pub u32);

/// One exact source resource row, before CON/MEN multipliers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct GrowthRow {
    pub hp: Scaled,
    pub mp: Scaled,
    pub cp: Scaled,
}

/// Item required for a transfer; count is consumed atomically by the application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TransferRequirement {
    pub item: String,
    pub count: u32,
}

/// Transfer inputs; quest hooks are stable identifiers reserved for Phase 6 evaluation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct TransferRules {
    pub requires: Vec<TransferRequirement>,
    #[serde(default)]
    pub quest_hooks: Vec<String>,
}

/// Deferred class skill-learning hook; Phase 3 owns the actual skill registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SkillLearnDef {
    pub key: String,
    pub skill_level: u32,
    pub required_level: u32,
    pub sp_cost: u64,
    pub auto_get: bool,
    #[serde(default)]
    pub required_items: Vec<TransferRequirement>,
}

/// Deferred equipment mastery unlock; Phase 4 owns the item and mastery effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ProficiencyDef {
    pub key: String,
    pub min_level: u32,
}

/// One node in the class tree. Base stats stay fixed throughout the lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ClassDef {
    pub id: ClassId,
    pub key: String,
    pub display_name: String,
    pub l2_ref: String,
    pub race: Race,
    pub tier: u8,
    pub parent: Option<ClassId>,
    pub min_level: u32,
    #[serde(with = "archetype_serde")]
    pub archetype: Archetype,
    pub base_class_id: ClassId,
    pub base_stats: BaseStats,
    pub movement: RaceMovement,
    pub collision: RaceCollision,
    /// Named growth table, or inherit from the parent when absent.
    pub growth: Option<String>,
    pub transfer: TransferRules,
    pub subclass_allowed: bool,
    pub subclass_equivalents: Vec<ClassId>,
    #[serde(default)]
    pub skill_tree: Vec<SkillLearnDef>,
    #[serde(default)]
    pub proficiencies: Vec<ProficiencyDef>,
}

/// Movement speeds in reference world units per second, converted once at the actor edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RaceMovement {
    pub walk: u32,
    pub run: u32,
    pub swim: u32,
}

/// Source collision dimensions in world units, kept at the exact decimal scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
#[serde(deny_unknown_fields)]
pub struct RaceCollision {
    pub radius_male: Scaled,
    pub radius_female: Scaled,
    pub height_male: Scaled,
    pub height_female: Scaled,
}

/// Racial environmental properties; gameplay consumers are deferred to their owning phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RaceTraits {
    pub breath: u32,
    pub safe_fall: u32,
}

/// A selectable classic race. Passive keys describe Phase 3 hooks, rather than learned skills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct RaceDef {
    pub id: Race,
    pub display_name: String,
    pub l2_ref: String,
    pub starting_zone: String,
    pub reference_starting_zone: String,
    pub start_points: Vec<[i32; 2]>,
    pub mystic_path: bool,
    pub base_class_ids: Vec<ClassId>,
    pub passive_skill_keys: Vec<String>,
    pub movement: RaceMovement,
    pub collision: RaceCollision,
    pub traits: RaceTraits,
}

/// Invalid catalog data or a missing query result. Invalid startup data is never accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// All semantic violations found while constructing a registry.
    #[error("invalid class registry: {0:?}")]
    Invalid(Vec<String>),
    /// The id is absent, including reserved Kamael and NPC ids.
    #[error("unknown class {0:?}")]
    UnknownClass(ClassId),
    /// Resource data does not cover the requested level.
    #[error("growth does not cover level {0}")]
    LevelOutOfRange(u32),
}

/// Serialized registry state. Deserialization re-runs the same validation as startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryParts {
    classes: Vec<ClassDef>,
    races: Vec<RaceDef>,
    growth: BTreeMap<String, Vec<GrowthRow>>,
}

/// Ordered catalog copied into replay snapshots, so replay never consults current files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "RegistryParts", try_from = "RegistryParts")]
pub struct ClassRegistry {
    parts: RegistryParts,
}

impl From<ClassRegistry> for RegistryParts {
    fn from(value: ClassRegistry) -> Self {
        value.parts
    }
}

impl TryFrom<RegistryParts> for ClassRegistry {
    type Error = RegistryError;
    fn try_from(value: RegistryParts) -> Result<Self, Self::Error> {
        Self::new(value.classes, value.races, value.growth)
    }
}

impl ClassRegistry {
    /// Constructs the complete classic catalog, rejecting broken trees, stats or growth.
    pub fn new(
        mut classes: Vec<ClassDef>,
        mut races: Vec<RaceDef>,
        growth: BTreeMap<String, Vec<GrowthRow>>,
    ) -> Result<Self, RegistryError> {
        classes.sort_by_key(|c| c.id);
        races.sort_by_key(|r| r.id.as_str());
        let registry = Self {
            parts: RegistryParts {
                classes,
                races,
                growth,
            },
        };
        let mut errors = Vec::new();
        registry.validate(&mut errors);
        if errors.is_empty() {
            Ok(registry)
        } else {
            Err(RegistryError::Invalid(errors))
        }
    }

    /// Every profession in retail-id order.
    #[must_use]
    pub fn classes(&self) -> &[ClassDef] {
        &self.parts.classes
    }

    /// Every selectable race in stable key order.
    #[must_use]
    pub fn races(&self) -> &[RaceDef] {
        &self.parts.races
    }

    /// One profession, including third-tier metadata.
    #[must_use]
    pub fn get(&self, id: ClassId) -> Option<&ClassDef> {
        self.parts.classes.iter().find(|c| c.id == id)
    }

    /// Resolves a race's static creation data.
    #[must_use]
    pub fn race(&self, id: Race) -> Option<&RaceDef> {
        self.parts.races.iter().find(|r| r.id == id)
    }

    /// Direct children, ordered by retail id.
    #[must_use]
    pub fn children(&self, id: ClassId) -> Vec<ClassId> {
        self.classes()
            .iter()
            .filter(|c| c.parent == Some(id))
            .map(|c| c.id)
            .collect()
    }

    /// Ancestors from the immediate parent up to the lineage root; no current-class entry.
    #[must_use]
    pub fn ancestors(&self, id: ClassId) -> Vec<ClassId> {
        let mut current = self.get(id).and_then(|c| c.parent);
        let mut out = Vec::new();
        while let Some(parent) = current {
            if out.contains(&parent) {
                break;
            }
            out.push(parent);
            current = self.get(parent).and_then(|c| c.parent);
        }
        out
    }

    /// Structurally valid level-gated options. The application also applies the transfer-tier cap.
    #[must_use]
    pub fn transfer_options(&self, from: ClassId, level: u32) -> Vec<ClassId> {
        self.classes()
            .iter()
            .filter(|c| c.parent == Some(from) && c.min_level <= level)
            .map(|c| c.id)
            .collect()
    }

    /// Full source growth table, resolving an absent table through ancestors.
    pub fn growth_table(&self, id: ClassId) -> Result<&[GrowthRow], RegistryError> {
        let mut current = Some(id);
        for _ in 0..=3 {
            let class = self
                .get(current.ok_or(RegistryError::UnknownClass(id))?)
                .ok_or(RegistryError::UnknownClass(id))?;
            if let Some(key) = &class.growth {
                return self
                    .parts
                    .growth
                    .get(key)
                    .map(Vec::as_slice)
                    .ok_or(RegistryError::UnknownClass(id));
            }
            current = class.parent;
        }
        Err(RegistryError::UnknownClass(id))
    }

    /// Exact pre-multiplier resources at a source level in 1..=85.
    pub fn growth(&self, id: ClassId, level: u32) -> Result<GrowthRow, RegistryError> {
        self.growth_table(id)?
            .get(
                usize::try_from(
                    level
                        .checked_sub(1)
                        .ok_or(RegistryError::LevelOutOfRange(level))?,
                )
                .map_err(|_| RegistryError::LevelOutOfRange(level))?,
            )
            .copied()
            .ok_or(RegistryError::LevelOutOfRange(level))
    }

    /// Deferred skill hooks, oldest ancestor first, then the current class.
    pub fn skill_tree(&self, id: ClassId) -> Result<Vec<&SkillLearnDef>, RegistryError> {
        let mut chain = self.ancestors(id);
        chain.reverse();
        chain.push(id);
        let mut out = Vec::new();
        for id in chain {
            out.extend(
                &self
                    .get(id)
                    .ok_or(RegistryError::UnknownClass(id))?
                    .skill_tree,
            );
        }
        Ok(out)
    }

    /// Deferred equipment hooks, oldest ancestor first, then the current class.
    pub fn proficiencies(&self, id: ClassId) -> Result<Vec<&ProficiencyDef>, RegistryError> {
        let mut chain = self.ancestors(id);
        chain.reverse();
        chain.push(id);
        let mut out = Vec::new();
        for id in chain {
            out.extend(
                &self
                    .get(id)
                    .ok_or(RegistryError::UnknownClass(id))?
                    .proficiencies,
            );
        }
        Ok(out)
    }

    fn validate(&self, errors: &mut Vec<String>) {
        let ids: BTreeSet<_> = self.classes().iter().map(|c| c.id.0).collect();
        let expected: BTreeSet<_> = (0..=57).chain(88..=118).collect();
        if ids != expected || self.classes().len() != 89 {
            errors.push("catalog must contain exactly classic ids 0..=57 and 88..=118".into());
        }
        let mut keys = BTreeSet::new();
        let mut names = BTreeSet::new();
        for class in self.classes() {
            if !keys.insert(&class.key)
                || class.key.is_empty()
                || !names.insert(&class.display_name)
                || class.display_name.is_empty()
                || class.l2_ref.is_empty()
            {
                errors.push(format!("class {:?}: empty or duplicate identity", class.id));
            }
            self.validate_class(class, errors);
        }
        self.validate_races(errors);
        for (key, table) in &self.parts.growth {
            if table.len() != 85
                || table
                    .iter()
                    .any(|r| r.hp.raw() <= 0 || r.mp.raw() <= 0 || r.cp.raw() <= 0)
            {
                errors.push(format!("growth {key}: must contain 85 positive HP/MP/CP rows"));
            }
            if table.windows(2).any(|r| match r {
                [a, b] => a.hp > b.hp || a.mp > b.mp || a.cp > b.cp,
                _ => false,
            }) {
                errors.push(format!("growth {key}: resources must not decrease"));
            }
        }
    }

    fn validate_class(&self, c: &ClassDef, errors: &mut Vec<String>) {
        let sum = [
            c.base_stats.str,
            c.base_stats.dex,
            c.base_stats.con,
            c.base_stats.int,
            c.base_stats.wit,
            c.base_stats.men,
        ]
        .iter()
        .copied()
        .fold(0_u32, u32::saturating_add);
        if !c.base_stats.is_valid() || sum != 170 {
            errors.push(format!("class {:?}: stats must be legal and total 170", c.id));
        }
        let level = match c.tier {
            0 => 1,
            1 => 20,
            2 => 40,
            3 => 76,
            _ => 0,
        };
        if level == 0 || c.min_level != level {
            errors.push(format!("class {:?}: invalid tier/level gate", c.id));
        }
        if c.tier == 0 {
            if c.parent.is_some() || c.base_class_id != c.id || !c.transfer.requires.is_empty() {
                errors.push(format!("class {:?}: invalid lineage root", c.id));
            }
        } else if let Some(parent) = c.parent.and_then(|id| self.get(id)) {
            if parent.tier.checked_add(1) != Some(c.tier)
                || parent.race != c.race
                || parent.base_class_id != c.base_class_id
                || parent.base_stats != c.base_stats
                || parent.archetype != c.archetype
            {
                errors.push(format!(
                    "class {:?}: inconsistent parent tier/race/stats/root/archetype",
                    c.id
                ));
            }
        } else {
            errors.push(format!("class {:?}: missing parent", c.id));
        }
        if self.ancestors(c.id).contains(&c.id) {
            errors.push(format!("class {:?}: cyclic lineage", c.id));
        }
        if c.movement.walk == 0 || c.movement.run == 0 || c.movement.swim == 0 {
            errors.push(format!("class {:?}: invalid movement", c.id));
        }
        if self.growth_table(c.id).is_err() {
            errors.push(format!("class {:?}: unresolved growth", c.id));
        }
        if c.transfer
            .requires
            .iter()
            .any(|r| r.item.is_empty() || r.count == 0)
            || c.transfer.quest_hooks.iter().any(String::is_empty)
        {
            errors.push(format!("class {:?}: invalid transfer requirement", c.id));
        }
        if c.skill_tree.iter().any(|skill| {
            skill.key.is_empty()
                || skill.skill_level == 0
                || !(1..=85).contains(&skill.required_level)
                || skill
                    .required_items
                    .iter()
                    .any(|item| item.item.is_empty() || item.count == 0)
        }) || c
            .proficiencies
            .iter()
            .any(|p| p.key.is_empty() || !(1..=85).contains(&p.min_level))
        {
            errors.push(format!("class {:?}: invalid deferred learning/proficiency hook", c.id));
        }
        if c.subclass_allowed && (c.tier != 2 || [51, 57].contains(&c.id.0)) {
            errors.push(format!("class {:?}: forbidden subclass", c.id));
        }
        let equiv: BTreeSet<_> = c.subclass_equivalents.iter().copied().collect();
        if equiv.len() != c.subclass_equivalents.len() || equiv.contains(&c.id) {
            errors.push(format!("class {:?}: invalid equivalence set", c.id));
        }
        for id in &c.subclass_equivalents {
            if !self
                .get(*id)
                .is_some_and(|other| other.tier == 2 && other.subclass_equivalents.contains(&c.id))
            {
                errors.push(format!("class {:?}: asymmetric or missing equivalent {id:?}", c.id));
            }
        }
    }

    fn validate_races(&self, errors: &mut Vec<String>) {
        let ids: BTreeSet<_> = self.races().iter().map(|r| r.id.as_str()).collect();
        if ids.len() != 5 || self.races().len() != 5 {
            errors.push("catalog must contain each of five selectable races once".into());
        }
        for race in self.races() {
            let expected: Vec<_> = self
                .classes()
                .iter()
                .filter(|c| c.race == race.id && c.tier == 0)
                .map(|c| c.id)
                .collect();
            if race.base_class_ids != expected
                || race.mystic_path != (race.id != Race::Dwarf)
                || race.display_name.is_empty()
                || race.starting_zone.is_empty()
                || race.start_points.is_empty()
                || race.movement.walk == 0
                || race.movement.run == 0
                || race.movement.swim == 0
                || [
                    race.collision.radius_male,
                    race.collision.radius_female,
                    race.collision.height_male,
                    race.collision.height_female,
                ]
                .iter()
                .any(|v| v.raw() <= 0)
            {
                errors.push(format!(
                    "race {}: invalid creation/movement/collision metadata",
                    race.id.as_str()
                ));
            }
            if race
                .passive_skill_keys
                .iter()
                .any(|k| !k.starts_with("racial."))
            {
                errors.push(format!("race {}: invalid racial passive hook", race.id.as_str()));
            }
        }
    }
}

// The existing stat-rule enum keeps its snapshot encoding; catalog files use stable snake case.
mod archetype_serde {
    use super::Archetype;
    use serde::{Deserialize, Deserializer, Serializer};
    // Serde field adapters require a borrowed value even for a small Copy enum.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(super) fn serialize<S: Serializer>(
        value: &Archetype,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match value {
            Archetype::Fighter => "fighter",
            Archetype::Mystic => "mystic",
        })
    }
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Archetype, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "fighter" => Ok(Archetype::Fighter),
            "mystic" => Ok(Archetype::Mystic),
            value => Err(serde::de::Error::custom(format!("unknown archetype {value}"))),
        }
    }
}
