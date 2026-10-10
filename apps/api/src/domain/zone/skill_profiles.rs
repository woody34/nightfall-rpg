//! Source templates used only by V5. Player regeneration is an 85-row per-class curve;
//! NPC regeneration uses the inherited scalar getter (including its binary32 boundary).

use super::skill_math::ExactRatio;
use crate::domain::class::{ClassId, ClassRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! profile {
    ($name:ident { $($field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(missing_docs)]
        pub struct $name { $(pub $field: $ty),* }
    };
}
profile!(ProfileSource {
    repository: String,
    revision: String,
    file: String,
    lines: String,
    symbol: String
});
profile!(ProfileBaseStats {
    str: u32,
    dex: u32,
    con: u32,
    int: u32,
    wit: u32,
    men: u32
});
profile!(MagicSlots {
    rear: u32,
    lear: u32,
    rfinger: u32,
    lfinger: u32,
    neck: u32
});
profile!(ClassMagicProfile {
    base_m_atk: u32,
    base_m_def: u32,
    base_m_atk_speed: u32,
    base_m_atk_speed_source: ProfileSource,
    base_m_critical: u32,
    base_m_critical_source: ProfileSource,
    m_def_slots_xml: MagicSlots,
    m_def_slots: MagicSlots,
});
profile!(ClassGetterBases {
    accuracy: u32,
    evasion: u32,
    magic_critical: u32,
    source: ProfileSource
});
profile!(ClassRegenLevel {
    level: u32,
    hp_regen: String,
    mp_regen: String,
    cp_regen: String
});
profile!(ClassRegenProfile { kind: String, source: ProfileSource, getter_source: ProfileSource, levels: Vec<ClassRegenLevel> });
profile!(ClassSkillProfile {
    schema: u32, id: ClassId, key: String, l2_ref: String, base_class_id: ClassId, tier: u8,
    min_level: u32, parent: Option<ClassId>, source: ProfileSource, base_stats: ProfileBaseStats,
    magical: ClassMagicProfile, combat_getter_bases: ClassGetterBases, regen: ClassRegenProfile,
});
profile!(NpcMagicXml {
    m_atk: String,
    m_def: String,
    hp_regen: String,
    mp_regen: String,
    accuracy: String
});
profile!(NpcMagicTemplate {
    m_atk: u32,
    m_def: u32,
    m_atk_speed: u32,
    m_atk_speed_source: ProfileSource,
    hp_regen: String,
    mp_regen: String
});
profile!(NpcGetterBases {
    accuracy: u32,
    evasion: u32
});
profile!(NpcSkillTemplate {
    schema: u32,
    source_npc_id: u32,
    source: ProfileSource,
    regen_kind: String,
    cp_regen_kind: String,
    xml: NpcMagicXml,
    template: NpcMagicTemplate,
    combat_getter_bases: NpcGetterBases,
    base_stats: ProfileBaseStats,
});
profile!(NpcSkillProfile {
    sp_reward: u64,
    template: NpcSkillTemplate
});
profile!(SkillProfiles { classes: BTreeMap<ClassId, ClassSkillProfile>, npcs: BTreeMap<String, NpcSkillProfile>, starter_weapon: StarterWeaponProfile, limits: EffectiveLimitProfile });
/// Pinned source provenance for the weapon calculator's insertion/replacement semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs, clippy::struct_field_names)] // Field names follow the generated input contract.
pub struct WeaponOrdering {
    pub default_source: ProfileSource,
    pub override_source: ProfileSource,
    pub set_source: ProfileSource,
    pub fold_source: ProfileSource,
}
profile!(WeaponFunction {
    stat: String,
    operation: String,
    order: i32,
    xml_ordinal: u32,
    value: String,
    value_q: i64,
    source: ProfileSource
});
profile!(StarterWeaponProfile { schema: u32, id: String, item_id: u32, name: String, source: ProfileSource, weapon_type: String, bodypart: String, random_damage: u32, attack_range: u32, ordering: WeaponOrdering, functions: Vec<WeaponFunction> });
profile!(EffectiveLimitProfile {
    schema: u32,
    source: ProfileSource,
    constants: EffectiveLimitConstants
});
/// Remaining separately sourced getter caps. All provenance participates in the rules hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct EffectiveLimitConstants {
    #[serde(rename = "MaxMCritRate")]
    pub max_magic_crit: super::skill_rules::SourceConstant,
    #[serde(rename = "MaxRunSpeed")]
    pub max_run_speed: super::skill_rules::SourceConstant,
    #[serde(rename = "RunSpeedBoost")]
    pub run_speed_boost: super::skill_rules::SourceConstant,
}
impl StarterWeaponProfile {
    /// Admit the supported equipment's complete source Func block; partial weapon profiles
    /// cannot silently fall back to class magic or the legacy physical projection.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.id != "squires_sword"
            || self.item_id != 2369
            || self.name != "Squire's Sword"
            || self.weapon_type != "SWORD"
            || self.bodypart != "rhand"
            || self.random_damage != 10
            || self.attack_range != 40
            || !self.source.valid(true)
            || self.source.file != "src/main/resources/data/stats/items/02300-02399.xml"
            || self.functions.len() != 4
            || [
                &self.ordering.default_source,
                &self.ordering.override_source,
                &self.ordering.set_source,
                &self.ordering.fold_source,
            ]
            .iter()
            .any(|source| !source.valid(false))
        {
            return Err("invalid Phase3 starter weapon source/profile".into());
        }
        for (function, (stat, value, line)) in self.functions.iter().zip([
            ("pAtk", 6, 862),
            ("mAtk", 5, 863),
            ("critRate", 8, 864),
            ("pAtkSpd", 379, 865),
        ]) {
            if function.stat != stat
                || function.operation != "SET"
                || function.order != 0
                || function.xml_ordinal.checked_add(862) != Some(line)
                || function.value_q != i64::from(value).saturating_mul(super::Q)
                || ExactRatio::parse_literal(&function.value).ok()
                    != Some(ExactRatio::whole(i64::from(value)))
                || !function.source.valid(true)
                || function.source.file != self.source.file
                || function.source.lines != line.to_string()
            {
                return Err(format!("invalid Phase3 weapon Func {stat}"));
            }
        }
        Ok(())
    }
    /// Order0 SETs install before builtins without modifying their insertion identity.
    pub fn functions(&self) -> Vec<super::stat_functions::StatFunction> {
        self.functions
            .iter()
            .map(|f| super::stat_functions::StatFunction {
                key: super::stat_functions::FunctionKey {
                    stat: f.stat.clone(),
                    priority: f.order,
                    install_sequence: 0,
                    handler_ordinal: 0,
                    function_ordinal: f.xml_ordinal,
                },
                operation: super::stat_functions::FunctionOp::Set(f.value_q),
                conditions: Vec::new(),
            })
            .collect()
    }
    /// Prove that shared physical equipment identity agrees with the legacy input bundle;
    /// this does not put new magic data into the legacy hash or calculator.
    pub fn validate_physical(&self, weapon: &super::WeaponBlock) -> Result<(), super::StatError> {
        let matches = self.id == weapon.id
            && self.random_damage == weapon.random_damage
            && self.attack_range == weapon.attack_range_l2
            && self.functions.iter().all(|f| match f.stat.as_str() {
                "pAtk" => f.value_q == weapon.p_atk.raw(),
                "critRate" => f.value_q == i64::from(weapon.crit_rate).saturating_mul(super::Q),
                "pAtkSpd" => f.value_q == i64::from(weapon.attack_speed).saturating_mul(super::Q),
                "mAtk" => true,
                _ => false,
            });
        if matches {
            Ok(())
        } else {
            Err(super::StatError::Overflow)
        }
    }
}
impl EffectiveLimitProfile {
    /// Require the pinned remaining caps, including exact source assignments/locations.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !self.source.valid(false)
            || self.source.file != "src/main/resources/config/character.properties"
        {
            return Err("invalid Phase3 limit provenance".into());
        }
        for (constant, name, value, line) in [
            (&self.constants.max_magic_crit, "MaxMCritRate", 200, 260),
            (&self.constants.max_run_speed, "MaxRunSpeed", 250, 252),
            (&self.constants.run_speed_boost, "RunSpeedBoost", 0, 31),
        ] {
            if constant.value != value
                || constant.line != line
                || constant.symbol != name
                || constant.literal != format!("{name} = {value}")
                || constant.file != self.source.file
            {
                return Err(format!("invalid Phase3 source limit {name}"));
            }
        }
        Ok(())
    }
}

impl ProfileBaseStats {
    /// Strictly decoded six source attributes, in the shared domain representation.
    pub const fn values(&self) -> crate::domain::BaseStats {
        crate::domain::BaseStats {
            str: self.str,
            dex: self.dex,
            con: self.con,
            int: self.int,
            wit: self.wit,
            men: self.men,
        }
    }
}
impl ProfileSource {
    fn valid(&self, datapack: bool) -> bool {
        let (repository, revision) = if datapack {
            (
                "https://bitbucket.org/l2jserver/l2j-server-datapack",
                "3ca488dd2bd0bfaca43e378886a3c2e37968153a",
            )
        } else {
            (
                "https://bitbucket.org/l2jserver/l2j-server-game",
                "abfde0490ac52a2106e884b7242c0276c44cdf76",
            )
        };
        self.repository == repository
            && self.revision == revision
            && !self.file.is_empty()
            && !self.symbol.is_empty()
            && !self.lines.is_empty()
            && self
                .lines
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == ',')
    }
}
impl MagicSlots {
    fn total(&self) -> Option<u32> {
        self.rear
            .checked_add(self.lear)?
            .checked_add(self.rfinger)?
            .checked_add(self.lfinger)?
            .checked_add(self.neck)
    }
}
fn positive(value: &str) -> bool {
    ExactRatio::parse_literal(value).is_ok_and(|r| {
        r.checked_cmp(ExactRatio::whole(0))
            .is_ok_and(std::cmp::Ordering::is_gt)
            && r.to_q().is_ok()
    })
}
impl ClassSkillProfile {
    /// Structural/source admission independent of the class catalogue's linkage validation.
    pub fn validate(&self) -> Result<(), String> {
        let magic = &self.magical;
        let xml = &magic.m_def_slots_xml;
        let slots = &magic.m_def_slots;
        let bases = &self.combat_getter_bases;
        if self.schema != 1
            || self.key.is_empty()
            || self.tier > 3
            || !(1..=85).contains(&self.min_level)
            || !self.base_stats.values().is_valid()
            || !self.source.valid(true)
            || !self
                .source
                .file
                .starts_with("src/main/resources/data/stats/chars/baseStats/")
            || !magic.base_m_atk_speed_source.valid(false)
            || !magic.base_m_critical_source.valid(false)
            || magic.base_m_atk == 0
            || magic.base_m_atk > 1_000_000
            || magic.base_m_def == 0
            || magic.base_m_def > 1_000_000
            || magic.base_m_atk_speed != 333
            || magic.base_m_critical != 0
            || slots.rear != xml.rear
            || slots.lear != xml.lear
            || slots.rfinger != xml.rfinger
            || slots.lfinger != xml.rfinger
            || slots.neck != xml.neck
            || slots.total() != Some(magic.base_m_def)
            || bases.accuracy != 0
            || bases.evasion != 0
            || bases.magic_critical != 1
            || !bases.source.valid(false)
            || self.regen.kind != "per_level"
            || !self.regen.source.valid(true)
            || !self.regen.getter_source.valid(false)
            || self.regen.source.file != self.source.file
            || self.regen.levels.len() != 85
        {
            return Err(format!(
                "class Phase3 profile {}: invalid identity/source/getter shape",
                self.key
            ));
        }
        for (index, row) in self.regen.levels.iter().enumerate() {
            if usize::try_from(row.level)
                .ok()
                .and_then(|l| l.checked_sub(1))
                != Some(index)
                || !positive(&row.hp_regen)
                || !positive(&row.mp_regen)
                || !positive(&row.cp_regen)
            {
                return Err(format!(
                    "class Phase3 profile {}: invalid regen level {}",
                    self.key, row.level
                ));
            }
        }
        Ok(())
    }
    /// The exact current-level source row; no parent/default fallback.
    pub fn regen_at(&self, level: u32) -> Option<&ClassRegenLevel> {
        usize::try_from(level.checked_sub(1)?)
            .ok()
            .and_then(|i| self.regen.levels.get(i))
    }
}
impl NpcSkillTemplate {
    /// Prove source XML -> getter casts, including exact widened scalar regeneration.
    pub fn validate(&self) -> Result<(), String> {
        let bad = || "invalid NPC Phase3 source/getter profile".to_owned();
        let ratio = |s: &str| ExactRatio::parse_literal(s).map_err(|_| bad());
        if self.schema != 1
            || self.source_npc_id == 0
            || !self.source.valid(true)
            || !self
                .source
                .file
                .starts_with("src/main/resources/data/stats/npcs/")
            || !self.template.m_atk_speed_source.valid(false)
            || self.template.m_atk_speed != 333
            || self.regen_kind != "scalar"
            || self.cp_regen_kind != "not_applicable"
            || !self.base_stats.values().is_valid()
            || self.combat_getter_bases.accuracy != 0
            || self.combat_getter_bases.evasion != 0
            || self.template.m_atk == 0
            || self.template.m_def == 0
            || ratio(&self.xml.m_atk)?.trunc().map_err(|_| bad())?
                != i128::from(self.template.m_atk)
            || ratio(&self.xml.m_def)?.trunc().map_err(|_| bad())?
                != i128::from(self.template.m_def)
            || ratio(&self.xml.hp_regen)?.binary32().map_err(|_| bad())?
                != ratio(&self.template.hp_regen)?
            || ratio(&self.xml.mp_regen)?.binary32().map_err(|_| bad())?
                != ratio(&self.template.mp_regen)?
            || !positive(&self.template.hp_regen)
            || !positive(&self.template.mp_regen)
            || !positive(&self.xml.accuracy)
        {
            return Err(bad());
        }
        Ok(())
    }
}
impl SkillProfiles {
    /// Bound and validate the self-contained immutable bundle before hashing/restoring it.
    pub fn validate(&self) -> Result<(), String> {
        self.starter_weapon.validate()?;
        self.limits.validate()?;
        if self.classes.len() != 89 || self.npcs.is_empty() {
            return Err("missing Phase3 class/NPC profiles".into());
        }
        let mut keys = std::collections::BTreeSet::new();
        for (id, profile) in &self.classes {
            profile.validate()?;
            if *id != profile.id || !keys.insert(&profile.key) {
                return Err("duplicate/mismatched Phase3 class profile".into());
            }
        }
        for (id, profile) in &self.npcs {
            profile.template.validate()?;
            if id.is_empty() || i64::try_from(profile.sp_reward).is_err() {
                return Err("invalid Phase3 NPC identity/reward".into());
            }
        }
        Ok(())
    }
    /// Do not combine magic/regen bases from a different profession catalogue.
    pub fn validate_catalogue(&self, registry: &ClassRegistry) -> Result<(), String> {
        if self.classes.len() != registry.classes().len() {
            return Err("Phase3 profile/class count mismatch".into());
        }
        for class in registry.classes() {
            let p = self
                .classes
                .get(&class.id)
                .ok_or_else(|| format!("missing Phase3 class profile {}", class.key))?;
            if p.key != class.key
                || p.l2_ref != class.l2_ref
                || p.base_class_id != class.base_class_id
                || p.tier != class.tier
                || p.parent != class.parent
                || p.min_level != class.min_level
                || p.base_stats.values() != class.base_stats
            {
                return Err(format!("Phase3 profile/catalogue mismatch: {}", class.key));
            }
        }
        Ok(())
    }
}
