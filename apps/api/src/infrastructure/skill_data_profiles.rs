//! Strict source profile loading; every source value is retained in the V5 rules hash.
use super::super::SkillDataError;
use crate::domain::zone::skill_profiles::{
    ClassSkillProfile, NpcSkillProfile, NpcSkillTemplate, SkillProfiles,
};
use std::collections::BTreeMap;
use std::path::Path;
include!("skill_data_profiles_embedded.rs");

/// Complete explicit file set; never consult a filesystem while restoring a snapshot.
#[derive(Debug, Clone)]
pub struct ProfileDataSource {
    /// classes-phase3/<profession-key>.toml.
    pub classes: BTreeMap<String, String>,
    /// npcs/<template-key>.toml, containing the unchanged legacy prefix and typed suffix.
    pub npcs: BTreeMap<String, String>,
}
impl ProfileDataSource {
    /// Immutable repository input compiled into the executable.
    pub fn embedded() -> Self {
        Self {
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
        if crate::infrastructure::npc_data::parse_template(&npc.id, text, &mut errors).is_none() {
            return Err(SkillDataError(errors));
        }
        if path != &format!("npcs/{}.toml", npc.id)
            || profiles
                .npcs
                .insert(
                    npc.id,
                    NpcSkillProfile {
                        sp_reward: npc.sp_reward,
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
