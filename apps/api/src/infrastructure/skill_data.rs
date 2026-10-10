//! Strict, independent skill registry admission. A generated executable label is a claim
//! checked against the engine's capabilities, not permission to partially execute a skill.

use crate::domain::class::RuntimeSkillStatus;
use crate::domain::zone::skill::{SkillCondition, SkillDef, SkillHandler, SkillId, SkillRegistry};
use crate::domain::zone::skill_math::{periodic_interval, ExactRatio};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

include!("skill_data_embedded.rs");

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
    let mut blockers = BTreeMap::new();
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
        validate_shape(&skill, &mut errors);
        if skill.sample_only {
            continue;
        }
        blockers.insert(id, capabilities(&skill));
        if skills.insert(id, skill).is_some() {
            errors.push(format!("{path}: duplicate skill {}", id.0));
        }
    }
    for id in required {
        if !skills.contains_key(id) {
            errors.push(format!("missing required skill {}", id.0));
        }
    }
    // Fixed-point closure in id order; references cannot execute through a deferred child.
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
        return Err(SkillDataError(errors));
    }
    let bytes = serde_json::to_vec(&skills).map_err(|e| SkillDataError(vec![e.to_string()]))?;
    let mut hash = Sha256::new();
    hash.update(b"nightfall.skills.1\0");
    hash.update(bytes);
    Ok(SkillRegistry::validated(skills, hash.finalize().into()))
}

fn validate_shape(skill: &SkillDef, errors: &mut Vec<String>) {
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
            || handler
                .stat_children
                .iter()
                .any(|p| Some(p.value.len()) != count)
        {
            error("handler rank table length mismatch");
        }
    }
}

fn capabilities(skill: &SkillDef) -> BTreeSet<String> {
    let mut missing = BTreeSet::new();
    if !["A1", "A2", "A3", "P"].contains(&skill.operate_type.as_str()) {
        missing.insert(format!("operate:{}", skill.operate_type));
    }
    for target in skill.target_type.values() {
        if !["SELF", "TARGET", "ENEMY", "ENEMY_ONLY", "PC_BODY"].contains(&target.as_str()) {
            missing.insert(format!("target:{target}"));
        }
    }
    for scope in skill.affect_scope.values() {
        if !["SINGLE", "POINT_BLANK", "RANGE", "unset"].contains(&scope.as_str()) {
            missing.insert(format!("scope:{scope}"));
        }
    }
    if !["", "ALL", "NOT_FRIEND"].contains(&skill.affect_object.as_str()) {
        missing.insert(format!("scope_filter:{}", skill.affect_object));
    }
    if !skill.item_consume.is_empty() {
        missing.insert("item_consume".into());
    }
    for condition in &skill.conditions {
        condition_capabilities(condition, &mut missing);
    }
    for handler in &skill.effects {
        if handler.section != "effects" {
            missing.insert(format!("section:{}", handler.section));
        }
        for condition in handler.conditions.iter().chain(&handler.section_conditions) {
            condition_capabilities(condition, &mut missing);
        }
        handler_capabilities(handler, &mut missing);
        for modifier in &handler.stat_children {
            for condition in &modifier.conditions {
                condition_capabilities(condition, &mut missing);
            }
            if !["mul", "add", "sub"].contains(&modifier.op.as_str()) {
                missing.insert(format!("stat_op:{}", modifier.op));
            }
            if !STATS.contains(&modifier.stat.as_str()) {
                missing.insert(format!("stat:{}", modifier.stat));
            }
            if modifier.value.iter().any(|v| !numeric(v)) {
                missing.insert(format!("stat_value:{}", modifier.stat));
            }
        }
    }
    missing
}

fn condition_capabilities(condition: &SkillCondition, missing: &mut BTreeSet<String>) {
    match condition {
        SkillCondition::UsingKind { weapon_kinds } => {
            if weapon_kinds.is_empty()
                || weapon_kinds
                    .iter()
                    .any(|kind| !WEAPONS.contains(&kind.as_str()))
            {
                missing.insert("condition:weapon_kind".into());
            }
        },
        SkillCondition::PlayerCanResurrect => {},
        SkillCondition::And { conditions }
        | SkillCondition::Or { conditions }
        | SkillCondition::Not { conditions } => {
            if conditions.is_empty()
                || matches!(condition, SkillCondition::Not { .. }) && conditions.len() != 1
            {
                missing.insert("condition:arity".into());
            }
            for child in conditions {
                condition_capabilities(child, missing);
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

fn handler_capabilities(handler: &SkillHandler, missing: &mut BTreeSet<String>) {
    let allowed: &[&str] = match handler.name.as_str() {
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
    for (key, values) in &handler.params {
        if !allowed.contains(&key.as_str()) {
            missing.insert(format!("param:{}:{key}", handler.name));
            continue;
        }
        for value in values.values() {
            let valid = match key.as_str() {
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

fn numeric(value: &str) -> bool {
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
    "SHIELD",
    "SIGIL",
    "LIGHT",
    "HEAVY",
    "MAGIC",
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

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
            207
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
                    conditions: vec![SkillCondition::Not { conditions: vec![] }],
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
