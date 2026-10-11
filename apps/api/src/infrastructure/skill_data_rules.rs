//! Typed assembly of the full skill registry and generated PvP/regeneration policy tables.
//! Class/NPC magic and base-regeneration profiles are a separate required effective-stat input.

use super::{load_skills, SkillDataError, SkillSource};
use crate::domain::zone::{skill::SkillId, Phase3Rules};
use std::path::Path;
#[path = "skill_data_profiles.rs"]
mod profiles;
pub use profiles::ProfileDataSource;

/// The immutable Phase 3 source bundle. Replay restores its validated domain form instead
/// of re-reading these paths or consulting the process's current data installation.
#[derive(Debug, Clone)]
pub struct Phase3DataSource {
    /// Complete generated skill directory.
    pub skills: SkillSource,
    /// Generated tables/pvp.toml source text.
    pub pvp: String,
    /// Generated tables/regen.toml source text.
    pub regen: String,
    /// Formula provenance including additive C5 constants.
    pub formulas: String,
    /// Separate class/NPC magic and base regeneration profiles.
    pub profiles: ProfileDataSource,
    /// Additive `zones/test_zone_phase3.toml` (never part of the legacy zone hash).
    pub geometry: String,
}

impl Phase3DataSource {
    /// Pinned repository data embedded in the executable, matching the class-data loader.
    pub fn embedded() -> Self {
        Self {
            profiles: ProfileDataSource::embedded(),
            skills: SkillSource::embedded(),
            pvp: include_str!("../../../../packages/data/tables/pvp.toml").into(),
            regen: include_str!("../../../../packages/data/tables/regen.toml").into(),
            formulas: include_str!("../../../../packages/data/tables/formulas.toml").into(),
            geometry: include_str!("../../../../packages/data/zones/test_zone_phase3.toml").into(),
        }
    }
    /// Load one explicit data root. No default filesystem path is consulted.
    pub fn from_dir(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            profiles: ProfileDataSource::from_dir(root)?,
            skills: SkillSource::from_dir(root)?,
            pvp: std::fs::read_to_string(root.join("tables/pvp.toml"))?,
            regen: std::fs::read_to_string(root.join("tables/regen.toml"))?,
            formulas: std::fs::read_to_string(root.join("tables/formulas.toml"))?,
            geometry: std::fs::read_to_string(root.join("zones/test_zone_phase3.toml"))?,
        })
    }
}

/// Reject any missing root, malformed table, unsupported capability claim or coefficient.
/// The resulting rules are immutable and self-contained for Snapshot9/digestV5.
pub fn load_phase3_rules(
    source: &Phase3DataSource,
    required: &[SkillId],
) -> Result<Phase3Rules, SkillDataError> {
    // The existing physical loader validates the rest of the formulas file. Only the new
    // constants are projected here; old tables remain outside the Phase 3 registry.
    #[derive(serde::Deserialize)]
    struct FormulaExtension {
        constants: crate::domain::zone::skill_rules::Phase3Constants,
    }
    let skills = load_skills(&source.skills, required)?;
    let pvp = toml::from_str(&source.pvp)
        .map_err(|error| SkillDataError(vec![format!("tables/pvp.toml: {error}")]))?;
    let regen = toml::from_str(&source.regen)
        .map_err(|error| SkillDataError(vec![format!("tables/regen.toml: {error}")]))?;
    let formulas: FormulaExtension = toml::from_str(&source.formulas)
        .map_err(|error| SkillDataError(vec![format!("tables/formulas.toml: {error}")]))?;
    Phase3Rules::try_new(
        skills,
        pvp,
        regen,
        formulas.constants,
        profiles::load_profiles(&source.profiles)?,
        toml::from_str(&source.geometry).map_err(|error| {
            SkillDataError(vec![format!("zones/test_zone_phase3.toml: {error}")])
        })?,
    )
    .map_err(SkillDataError)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn geometry_is_required_strict_separately_hashed_and_bounded() {
        use crate::domain::zone::{Vec2Fixed, ZoneBounds};
        let source = Phase3DataSource::embedded();
        let original = load_phase3_rules(&source, &[]).unwrap();
        let bounds =
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
        let square = original.geometry().peace_square(bounds).unwrap();
        for corner in [(116, 116), (116, 136), (136, 116), (136, 136)] {
            assert!(square.contains(Vec2Fixed::from_tiles(corner.0, corner.1)));
        }
        assert!(!square.contains(Vec2Fixed::from_tiles(115, 126)));
        assert!(original
            .geometry()
            .peace_square(
                ZoneBounds::new(Vec2Fixed::from_tiles(120, 120), Vec2Fixed::from_tiles(130, 130))
                    .unwrap()
            )
            .is_err());
        let mut changed = source.clone();
        changed.geometry.push_str("\n# no policy change\n");
        assert_eq!(original.hash(), load_phase3_rules(&changed, &[]).unwrap().hash());
        changed.geometry = source.geometry.replace("10000", "11000");
        assert_ne!(original.hash(), load_phase3_rules(&changed, &[]).unwrap().hash());
        for replacement in [
            String::new(),
            source.geometry.replace("square", "circle"),
            source
                .geometry
                .replace("inclusive = true", "inclusive = false"),
            source.geometry.replace("10000", "0"),
            source.geometry.replace("10000", "-1"),
            source.geometry.replace("10000", "2147483647"),
            source.geometry.replace("[126, 126]", "[2147483647, 126]"),
            source
                .geometry
                .replace("schema_version = 1", "schema_version = 2"),
            source.geometry.replace("shape =", "unknown ="),
        ] {
            changed.geometry = replacement;
            assert!(load_phase3_rules(&changed, &[]).is_err());
        }
        let mut snapshot = serde_json::to_value(&original).unwrap();
        snapshot["geometry"]["peace_zone"]["half_width_millitiles"] = 11000.into();
        assert!(serde_json::from_value::<Phase3Rules>(snapshot).is_err());
    }

    #[test]
    fn complete_bundle_roundtrips_and_hashes_every_policy_table() {
        let source = Phase3DataSource::embedded();
        let rules = load_phase3_rules(&source, &[SkillId(3), SkillId(5076)]).unwrap();
        let bytes = serde_json::to_vec(&rules).unwrap();
        assert_eq!(serde_json::from_slice::<Phase3Rules>(&bytes).unwrap(), rules);
        assert_eq!(rules.skills().skills().len(), 388);
        let mut changed = source.clone();
        changed.pvp = changed
            .pvp
            .replace("pvp_flag_ticks = 1200", "pvp_flag_ticks = 1201");
        assert_ne!(load_phase3_rules(&changed, &[]).unwrap().hash(), rules.hash());
        changed = source.clone();
        changed.regen = changed
            .regen
            .replace("standing = \"1.1\"", "standing = \"1.2\"");
        assert_ne!(load_phase3_rules(&changed, &[]).unwrap().hash(), rules.hash());
        changed = source;
        changed.regen.push_str("\n# no semantics changed\n");
        assert_eq!(load_phase3_rules(&changed, &[]).unwrap().hash(), rules.hash());
    }

    #[test]
    fn restore_rejects_hash_capability_and_policy_tampering() {
        let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let original = serde_json::to_value(&rules).unwrap();
        for path in [
            vec!["timer_policy_version"],
            vec!["pvp", "flags", "pvp_flag_ticks"],
            vec!["regen", "period", "hp_regenerate_period_ticks"],
            vec!["skills", "skills", "3", "runtime_status"],
        ] {
            let mut altered = original.clone();
            let mut value = &mut altered;
            for key in path {
                value = &mut value[key];
            }
            *value = if value.is_string() {
                serde_json::json!("deferred")
            } else {
                serde_json::json!(0)
            };
            assert!(serde_json::from_value::<Phase3Rules>(altered).is_err());
        }
    }

    #[test]
    fn class_catalogue_cannot_name_a_missing_registry_skill() {
        use crate::infrastructure::class_data::{load_classes, ClassSource};
        let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
        let mut source = Phase3DataSource::embedded();
        source.skills.0.remove("skills/3.toml");
        let rules = load_phase3_rules(&source, &[]).unwrap();
        assert!(rules
            .validate_catalogue(&classes)
            .unwrap_err()
            .contains("skill 3"));
    }

    #[test]
    fn invalid_period_divisor_provenance_and_unknown_fields_fail_at_load() {
        for (table, before, after) in [
            ("regen", "hp_regenerate_period_ticks = 30", "hp_regenerate_period_ticks = 29"),
            ("pvp", "divisor = 30", "divisor = 0"),
            ("pvp", "rate_karma_lost = 1", "rate_karma_lost = 0"),
            ("regen", "abfde0490ac52a2106e884b7242c0276c44cdf76", "wrong"),
            ("regen", "min_initial_regen = 1", "unknown_initial_regen = 1"),
        ] {
            let mut source = Phase3DataSource::embedded();
            let text = if table == "regen" {
                &mut source.regen
            } else {
                &mut source.pvp
            };
            assert!(text.contains(before));
            *text = text.replace(before, after);
            assert!(load_phase3_rules(&source, &[]).is_err(), "{table}: {after}");
        }
    }
}
