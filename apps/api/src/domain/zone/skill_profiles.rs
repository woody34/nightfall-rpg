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
profile!(SkillProfiles { classes: BTreeMap<ClassId, ClassSkillProfile>, npcs: BTreeMap<String, NpcSkillProfile> });

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
