//! Strict, independent skill registry admission. A generated executable label is a claim
//! checked against the engine's capabilities, not permission to partially execute a skill.

use crate::domain::zone::skill::{validate_skill_shape, SkillDef, SkillId, SkillRegistry};
use std::collections::BTreeMap;
use std::path::Path;

include!("skill_data_embedded.rs");
#[path = "skill_data_rules.rs"]
mod rules;
pub use rules::{load_phase3_rules, Phase3DataSource, ProfileDataSource};

/// Source text in canonical relative-file order.
#[derive(Debug, Clone, Default)]
pub struct SkillSource(pub BTreeMap<String, String>);

impl SkillSource {
    /// All definitions, including deferred skills; malformed files cannot be skipped.
    pub fn embedded() -> Self {
        Self(
            EMBEDDED_SKILLS
                .iter()
                .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
                .collect(),
        )
    }
    /// Read a complete skills directory beneath a data root.
    pub fn from_dir(data_root: &Path) -> std::io::Result<Self> {
        let mut files = BTreeMap::new();
        for entry in std::fs::read_dir(data_root.join("skills"))? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                let name = path
                    .file_name()
                    .ok_or_else(|| std::io::Error::other("skill file has no name"))?;
                files.insert(
                    format!("skills/{}", name.to_string_lossy()),
                    std::fs::read_to_string(&path)?,
                );
            }
        }
        Ok(Self(files))
    }
}

/// All syntax/provenance/capability problems, with skill/file context.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid skill data: {0:?}")]
pub struct SkillDataError(pub Vec<String>);

/// Parse, validate and hash normalized definitions. `required` is the class-tree/reference
/// root set; source-only samples never enter the runtime registry. Rules/template assembly
/// is separate, so missing magic/regen input cannot be replaced by guessed defaults.
pub fn load_skills(
    source: &SkillSource,
    required: &[SkillId],
) -> Result<SkillRegistry, SkillDataError> {
    let mut errors = Vec::new();
    let mut skills = BTreeMap::new();
    for (path, text) in &source.0 {
        let skill: SkillDef = match toml::from_str(text) {
            Ok(skill) => skill,
            Err(error) => {
                errors.push(format!("{path}: {error}"));
                continue;
            },
        };
        let id = skill.id;
        if path != &format!("skills/{}.toml", id.0)
            || skill.provenance.skill_id != id.0
            || skill.key != format!("l2.skill.{}", id.0)
            || skill.provenance.revision != "3ca488dd2bd0bfaca43e378886a3c2e37968153a"
            || skill.provenance.repository != "https://bitbucket.org/l2jserver/l2j-server-datapack"
        {
            errors.push(format!("{path}: skill {} identity/provenance mismatch", id.0));
        }
        validate_skill_shape(&skill, &mut errors);
        if skill.sample_only {
            continue;
        }
        if skills.insert(id, skill).is_some() {
            errors.push(format!("{path}: duplicate skill {}", id.0));
        }
    }
    for id in required {
        if !skills.contains_key(id) {
            errors.push(format!("missing required skill {}", id.0));
        }
    }
    if !errors.is_empty() {
        return Err(SkillDataError(errors));
    }
    SkillRegistry::try_new(skills).map_err(SkillDataError)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::class::RuntimeSkillStatus;
    use crate::domain::zone::skill::numeric;
    use crate::domain::zone::skill::SkillCondition;
    use std::fmt::Write as _;

    #[test]
    fn category_dispel_privileged_membership_and_static_exceptions() {
        let registry = load_skills(&SkillSource::embedded(), &[]).unwrap();
        let mut shield = registry.get(SkillId(1040)).unwrap().clone();
        assert!(shield.can_be_stolen(1));
        for (start, end) in crate::domain::zone::skill::PRIVILEGED_SKILL_RANGES {
            for id in *start..=*end {
                assert!(!registry
                    .skills()
                    .values()
                    .any(|s| s.id.0 == id && s.runtime_status == RuntimeSkillStatus::Executable));
                shield.id = SkillId(id);
                assert!(!shield.can_be_stolen(1));
                assert!(shield.can_be_stolen(2), "membership is rank-aware");
            }
        }
        shield.id = SkillId(1040);
        shield.is_magic = 2;
        assert!(!shield.can_be_stolen(1));
        shield.id = SkillId(2341);
        assert!(shield.can_be_stolen(1));
        shield.id = SkillId(1040);
        shield.magic_level[0] = -1;
        assert!(shield.can_be_stolen(1));
        shield.irreplaceable_buff = crate::domain::zone::skill::RankValue::Scalar(true);
        assert!(!shield.can_be_stolen(1));
        assert!(!registry.get(SkillId(1168)).unwrap().can_be_stolen(1));
    }

    fn source() -> SkillSource {
        SkillSource(BTreeMap::from([(
            "skills/3.toml".into(),
            include_str!("../../../../packages/data/skills/3.toml").into(),
        )]))
    }
    fn edited(edit: impl FnOnce(&mut SkillDef)) -> SkillSource {
        let mut source = source();
        let text = source.0.get_mut("skills/3.toml").unwrap();
        let mut skill: SkillDef = toml::from_str(text).unwrap();
        edit(&mut skill);
        let value = serde_json::to_value(&skill).unwrap();
        text.clear();
        for (key, value) in value
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, value)| !value.is_null())
        {
            writeln!(text, "{key} = {}", toml_value(value)).unwrap();
        }
        source
    }
    fn toml_value(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => format!(
                "{{ {} }}",
                map.iter()
                    .filter(|(_, value)| !value.is_null())
                    .map(|(key, value)| format!("{key} = {}", toml_value(value)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            serde_json::Value::Array(values) => {
                format!("[{}]", values.iter().map(toml_value).collect::<Vec<_>>().join(", "))
            },
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => value.to_string(),
        }
    }
    fn refused(source: &SkillSource, reason: &str) {
        let errors = load_skills(source, &[SkillId(3)]).unwrap_err().to_string();
        assert!(errors.contains(reason), "{errors}");
        assert!(errors.contains('3'), "must name the skill: {errors}");
    }

    #[test]
    fn embedded_registry_agrees_with_generated_capability_status() {
        let registry = load_skills(&SkillSource::embedded(), &[]).unwrap();
        assert_eq!(registry.skills().len(), 388);
        assert_eq!(
            registry
                .skills()
                .values()
                .filter(|s| s.runtime_status == RuntimeSkillStatus::Executable)
                .count(),
            188
        );
    }

    #[test]
    fn valid_skill_keeps_handler_rank_order_and_has_canonical_hash() {
        let source = source();
        let registry = load_skills(&source, &[SkillId(3)]).unwrap();
        let skill = registry.get(SkillId(3)).unwrap();
        assert_eq!(skill.effects[0].params["power"].at(9, 9).unwrap(), "84");
        let mut reformatted = source.clone();
        reformatted
            .0
            .get_mut("skills/3.toml")
            .unwrap()
            .push_str("\n# formatting is not state\n");
        assert_eq!(registry.hash(), load_skills(&reformatted, &[]).unwrap().hash());
        let changed = edited(|s| s.reuse_ms = crate::domain::zone::skill::RankValue::Scalar(4000));
        assert_ne!(registry.hash(), load_skills(&changed, &[]).unwrap().hash());
    }

    #[test]
    fn executable_label_cannot_hide_unknown_parameter_operation_filter_or_condition() {
        for source in [
            &edited(|s| {
                s.effects[0].params.insert(
                    "unknown".into(),
                    crate::domain::zone::skill::RankValue::Scalar("1".into()),
                );
            }),
            &edited(|s| s.affect_object = "INVISIBLE".into()),
            &edited(|s| {
                s.conditions = vec![SkillCondition::And {
                    conditions: vec![SkillCondition::Not {
                        conditions: vec![],
                        attrs: BTreeMap::new(),
                    }],
                    attrs: BTreeMap::new(),
                }];
            }),
            &edited(|s| {
                s.effects[0]
                    .stat_children
                    .push(crate::domain::zone::skill::SkillModifier {
                        op: "pow".into(),
                        stat: "pAtk".into(),
                        value: vec!["2".into(); 9],
                        order: None,
                        raw_attributes: BTreeMap::new(),
                        conditions: vec![],
                        raw_conditions: vec![],
                    });
            }),
        ] {
            refused(source, "runtime_status mismatch");
        }
        refused(
            &edited(|s| s.runtime_status = RuntimeSkillStatus::Deferred),
            "runtime_status mismatch",
        );
    }

    #[test]
    fn deferred_unknown_handler_is_metadata_but_invalid_syntax_is_never_skipped() {
        let source = edited(|s| {
            s.effects[0].name = "UnsupportedHandler".into();
            s.runtime_status = RuntimeSkillStatus::Deferred;
            s.blockers = vec!["effect:UnsupportedHandler".into()];
        });
        assert_eq!(
            load_skills(&source, &[])
                .unwrap()
                .get(SkillId(3))
                .unwrap()
                .runtime_status,
            RuntimeSkillStatus::Deferred
        );
        let mut broken = source;
        broken
            .0
            .get_mut("skills/3.toml")
            .unwrap()
            .push_str("\ninvalid = { key: 1 }\n");
        refused(&broken, "skills/3.toml");
    }

    #[test]
    fn refuses_unresolved_ranges_bad_tables_provenance_and_missing_required_skill() {
        refused(
            &edited(|s| s.cast_range = crate::domain::zone::skill::RankValue::Scalar(0)),
            "zero non-self range",
        );
        refused(
            &edited(|s| {
                s.mp_consume_end.pop();
            }),
            "per-rank array length mismatch",
        );
        refused(&edited(|s| s.provenance.revision = "wrong".into()), "provenance mismatch");
        assert!(load_skills(&source(), &[SkillId(999)])
            .unwrap_err()
            .to_string()
            .contains("missing required skill 999"));
        refused(&edited(|s| s.sample_only = true), "missing required skill 3");
    }

    #[test]
    fn decimal_bounds_and_source_tick_multiplication_are_checked() {
        assert!(numeric("-0.125"));
        assert!(!numeric("999999999999999999999999999999999999999999999"));
        for literal in ["", " ", "NaN", "1.2.3", "1e100", "1."] {
            assert!(!numeric(literal), "{literal}");
        }
        refused(
            &edited(|s| {
                s.effects[0].name = "TickMp".into();
                s.effects[0].params.remove("ticks");
            }),
            "runtime_status mismatch",
        );
        refused(
            &edited(|s| {
                s.effects[0].name = "TickHp".into();
                s.effects[0].params.insert(
                    "ticks".into(),
                    crate::domain::zone::skill::RankValue::Scalar(u32::MAX.to_string()),
                );
            }),
            "runtime_status mismatch",
        );
    }
}
