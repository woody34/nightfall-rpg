//! Strict source profile loading; every source value is retained in the V5 rules hash.
use super::super::SkillDataError;
use crate::domain::zone::skill_profiles::{
    ClassSkillProfile, NpcPhysicalProfile, NpcSkillProfile, NpcSkillTemplate, SkillProfiles,
};
use std::collections::BTreeMap;
use std::path::Path;
include!("skill_data_profiles_embedded.rs");

/// Complete explicit file set; never consult a filesystem while restoring a snapshot.
#[derive(Debug, Clone)]
pub struct ProfileDataSource {
    /// Separately hashed tables-phase3/starter_weapon.toml.
    pub starter_weapon: String,
    /// Separately hashed tables-phase3/constants.toml.
    pub limits: String,
    /// classes-phase3/<profession-key>.toml.
    pub classes: BTreeMap<String, String>,
    /// npcs/<template-key>.toml, containing the unchanged legacy prefix and typed suffix.
    pub npcs: BTreeMap<String, String>,
    /// Explicit simulation fixture movement overrides, applied after template validation.
    /// The resulting physical profile is hashed and checked against the selected zone.
    pub npc_speed_overrides: BTreeMap<String, crate::domain::zone::Speed>,
}
impl ProfileDataSource {
    /// Immutable repository input compiled into the executable.
    pub fn embedded() -> Self {
        Self {
            npc_speed_overrides: BTreeMap::new(),
            starter_weapon: include_str!(
                "../../../../packages/data/tables-phase3/starter_weapon.toml"
            )
            .into(),
            limits: include_str!("../../../../packages/data/tables-phase3/constants.toml").into(),
            classes: EMBEDDED_PROFILES
                .iter()
                .map(|(p, s)| ((*p).into(), (*s).into()))
                .collect(),
            npcs: crate::infrastructure::npc_data::EMBEDDED_TEMPLATES
                .iter()
                .map(|(p, s)| (format!("npcs/{p}.toml"), (*s).into()))
                .collect(),
        }
    }
    /// Read exactly one supplied data root with no default path fallback.
    pub fn from_dir(root: &Path) -> std::io::Result<Self> {
        fn directory(root: &Path, name: &str) -> std::io::Result<BTreeMap<String, String>> {
            let mut entries = BTreeMap::new();
            for entry in std::fs::read_dir(root.join(name))? {
                let path = entry?.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    let file = path
                        .file_name()
                        .ok_or_else(|| std::io::Error::other("profile without filename"))?;
                    entries.insert(
                        format!("{name}/{}", file.to_string_lossy()),
                        std::fs::read_to_string(path)?,
                    );
                }
            }
            Ok(entries)
        }
        Ok(Self {
            npc_speed_overrides: BTreeMap::new(),
            starter_weapon: std::fs::read_to_string(
                root.join("tables-phase3/starter_weapon.toml"),
            )?,
            limits: std::fs::read_to_string(root.join("tables-phase3/constants.toml"))?,
            classes: directory(root, "classes-phase3")?,
            npcs: directory(root, "npcs")?,
        })
    }
}
/// Parse every source file and reject mismatched filenames, duplicate ids, missing typed
/// NPC extension, malformed legacy prefix and getter/source disagreement before hashing.
pub(super) fn load_profiles(source: &ProfileDataSource) -> Result<SkillProfiles, SkillDataError> {
    #[derive(serde::Deserialize)]
    struct NpcFile {
        id: String,
        sp_reward: u64,
        phase3: NpcSkillTemplate,
    }
    let mut profiles = SkillProfiles {
        starter_weapon: toml::from_str(&source.starter_weapon)
            .map_err(|e| SkillDataError(vec![format!("tables-phase3/starter_weapon.toml: {e}")]))?,
        limits: toml::from_str(&source.limits)
            .map_err(|e| SkillDataError(vec![format!("tables-phase3/constants.toml: {e}")]))?,
        classes: BTreeMap::new(),
        npcs: BTreeMap::new(),
    };
    for (path, text) in &source.classes {
        let p: ClassSkillProfile =
            toml::from_str(text).map_err(|e| SkillDataError(vec![format!("{path}: {e}")]))?;
        p.validate()
            .map_err(|e| SkillDataError(vec![format!("{path}: {e}")]))?;
        if path != &format!("classes-phase3/{}.toml", p.key)
            || profiles.classes.insert(p.id, p).is_some()
        {
            return Err(SkillDataError(vec![format!("{path}: profile filename/id mismatch")]));
        }
    }
    for (path, text) in &source.npcs {
        let npc: NpcFile =
            toml::from_str(text).map_err(|e| SkillDataError(vec![format!("{path}: {e}")]))?;
        let mut errors = Vec::new();
        let mut physical =
            crate::infrastructure::npc_data::parse_template(&npc.id, text, &mut errors)
                .ok_or(SkillDataError(errors))?;
        if let Some(speed) = source.npc_speed_overrides.get(&npc.id) {
            physical.move_speed = *speed;
        }
        if path != &format!("npcs/{}.toml", npc.id)
            || profiles
                .npcs
                .insert(
                    npc.id,
                    NpcSkillProfile {
                        sp_reward: npc.sp_reward,
                        physical: NpcPhysicalProfile::from_template(&physical),
                        template: npc.phase3,
                    },
                )
                .is_some()
        {
            return Err(SkillDataError(vec![format!("{path}: NPC filename/id mismatch")]));
        }
    }
    profiles.validate().map_err(|e| SkillDataError(vec![e]))?;
    Ok(profiles)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
    #[test]
    fn npc_physical_inputs_are_canonical_and_independently_validated() {
        let source = Phase3DataSource::embedded();
        let original = load_phase3_rules(&source, &[]).unwrap();
        let mut changed = source.clone();
        let text = changed.profiles.npcs.get_mut("npcs/keltir.toml").unwrap();
        *text = text.replace("p_atk = \"8.8\"", "p_atk = \"9.1\"");
        let changed = load_phase3_rules(&changed, &[]).unwrap();
        assert_ne!(changed.hash(), original.hash());
        assert_eq!(changed.profiles().npcs["keltir"].physical.p_atk_q, 9_100_000);
        let encoded = serde_json::to_vec(&original).unwrap();
        let restored: crate::domain::zone::Phase3Rules = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            restored.profiles().npcs["keltir"].physical,
            original.profiles().npcs["keltir"].physical
        );
        for field in ["level", "max_hp", "p_def_q", "attack_speed"] {
            let mut value = serde_json::to_value(&original).unwrap();
            value["profiles"]["npcs"]["keltir"]["physical"][field] = 0.into();
            assert!(serde_json::from_value::<crate::domain::zone::Phase3Rules>(value).is_err());
        }
    }
    #[test]
    fn profiles_are_complete_linked_hashed_and_reject_invalid_getter_casts() {
        let source = Phase3DataSource::embedded();
        let rules = load_phase3_rules(&source, &[]).unwrap();
        let classes = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap();
        rules.validate_catalogue(&classes.registry).unwrap();
        assert_eq!(rules.profiles().classes.len(), 89);
        assert_eq!(
            rules
                .profiles()
                .classes
                .values()
                .map(|p| p.regen.levels.len())
                .sum::<usize>(),
            7565
        );
        assert_eq!(rules.profiles().npcs["keltir"].template.template.mp_regen, "7549747/8388608");
        let mut changed = source.clone();
        changed
            .profiles
            .classes
            .get_mut("classes-phase3/human_fighter.toml")
            .unwrap()
            .push_str("\n# comment only\n");
        assert_eq!(rules.hash(), load_phase3_rules(&changed, &[]).unwrap().hash());
        let original = source.profiles.classes["classes-phase3/human_fighter.toml"].clone();
        changed.profiles.classes.insert(
            "classes-phase3/human_fighter.toml".into(),
            original.replacen("hp_regen = \"2.0\"", "hp_regen = \"2.1\"", 1),
        );
        assert_ne!(rules.hash(), load_phase3_rules(&changed, &[]).unwrap().hash());
        for replacement in ["m_atk = 6", "mp_regen = \"0.9\"", "m_atk_speed = 0"] {
            let mut bad = source.clone();
            let text = bad.profiles.npcs.get_mut("npcs/keltir.toml").unwrap();
            let old = if replacement.starts_with("m_atk =") {
                "m_atk = 5"
            } else if replacement.starts_with("mp_regen") {
                "mp_regen = \"7549747/8388608\""
            } else {
                "m_atk_speed = 333"
            };
            *text = text.replace(old, replacement);
            assert!(load_phase3_rules(&bad, &[]).is_err(), "{replacement}");
        }
        changed = source;
        changed
            .profiles
            .classes
            .remove("classes-phase3/human_fighter.toml");
        assert!(load_phase3_rules(&changed, &[]).is_err());
    }
    #[test]
    fn generated_suffix_never_changes_legacy_npc_projection_or_hash() {
        use crate::infrastructure::npc_data::{legacy_hash_source, parse_template, KELTIR_TOML};
        let legacy = legacy_hash_source(KELTIR_TOML);
        assert!(legacy.len() < KELTIR_TOML.len());
        let mut errors = vec![];
        let old = parse_template("keltir", legacy, &mut errors).unwrap();
        let new = parse_template("keltir", KELTIR_TOML, &mut errors).unwrap();
        assert!(errors.is_empty());
        assert_eq!(old, new);
        let zone = include_str!("../../../../packages/data/zones/test_zone.toml");
        let old =
            crate::infrastructure::zone_data::parse_zone_with(zone, &[("keltir", legacy)]).unwrap();
        let new =
            crate::infrastructure::zone_data::parse_zone_with(zone, &[("keltir", KELTIR_TOML)])
                .unwrap();
        assert_eq!(old.config_hash, new.config_hash);
        let hidden = KELTIR_TOML.replace("[phase3]\n", "max_hp = 123\n[phase3]\n");
        assert_eq!(legacy_hash_source(&hidden), hidden);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod equipment_tests {
    use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};

    #[test]
    fn equipped_sources_are_strict_required_and_hash_all_provenance() {
        let source = Phase3DataSource::embedded();
        let rules = load_phase3_rules(&source, &[]).unwrap();
        let weapon = &rules.profiles().starter_weapon;
        assert_eq!(weapon.item_id, 2369);
        assert_eq!(
            weapon
                .functions
                .iter()
                .map(|f| (f.stat.as_str(), f.order, f.value_q))
                .collect::<Vec<_>>(),
            [
                ("pAtk", 0, 6_000_000),
                ("mAtk", 0, 5_000_000),
                ("critRate", 0, 8_000_000),
                ("pAtkSpd", 0, 379_000_000)
            ]
        );
        assert_eq!(rules.profiles().limits.constants.max_magic_crit.value, 200);
        assert_eq!(rules.profiles().limits.constants.max_run_speed.value, 250);
        assert_eq!(rules.profiles().limits.constants.run_speed_boost.value, 0);
        for (old, new) in [
            ("stat = \"mAtk\"", "stat = \"unknown\""),
            ("operation = \"SET\"", "operation = \"ADD\""),
            ("order = 0", "order = 1"),
            ("value_q = 5000000", "value_q = 6000000"),
            ("xml_ordinal = 1", "xml_ordinal = 0"),
            ("item_id = 2369", "item_id = 1"),
        ] {
            let mut changed = source.clone();
            changed.profiles.starter_weapon = changed.profiles.starter_weapon.replace(old, new);
            assert!(load_phase3_rules(&changed, &[]).is_err(), "{old}");
        }
        let mut changed = source.clone();
        changed.profiles.limits = changed
            .profiles
            .limits
            .replace("value = 200", "value = 201");
        assert!(load_phase3_rules(&changed, &[]).is_err());
        changed = source.clone();
        changed.profiles.limits.push_str("\nUnexpected = 1\n");
        assert!(load_phase3_rules(&changed, &[]).is_err());
        changed = source.clone();
        changed.profiles.starter_weapon.clear();
        assert!(load_phase3_rules(&changed, &[]).is_err());
        changed = source.clone();
        changed.profiles.starter_weapon = changed
            .profiles
            .starter_weapon
            .replace("symbol = \"item 2369\"", "symbol = \"item 2369 provenance edit\"");
        assert_ne!(load_phase3_rules(&changed, &[]).unwrap().hash(), rules.hash());
        changed = source.clone();
        changed.profiles.limits = changed.profiles.limits.replace(
            "symbol = \"remaining effective-stat limits\"",
            "symbol = \"remaining effective-stat limits provenance edit\"",
        );
        assert_ne!(load_phase3_rules(&changed, &[]).unwrap().hash(), rules.hash());
        let json = serde_json::to_vec(&rules).unwrap();
        assert_eq!(
            serde_json::from_slice::<crate::domain::zone::Phase3Rules>(&json).unwrap(),
            rules
        );
    }
}
