//! Pinned high-level NPC multipliers and hit conditions (Formulas.java:713-731,
//! 776-809,887-898,1286-1301; HitConditionBonusData.java:92-118). No RNG is consumed here.
use super::skill_math::ExactRatio;
use super::{StatError, Q};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Full table provenance, retained in the immutable epoch hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Source {
    pub repository: String,
    pub revision: String,
    pub file: String,
    pub lines: String,
    pub symbol: String,
}
impl Source {
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
            && !self.lines.is_empty()
            && !self.symbol.is_empty()
    }
}
/// Exact source list and its independently checked Q projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Coefficient {
    pub values: Vec<String>,
    pub values_q: Vec<i64>,
    pub source: Source,
}
/// Generated evidence rows; recomputed on load, not substituted for the source formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PenaltyRow {
    pub gap_min: u32,
    pub gap_max: i32,
    pub damage_index: i32,
    pub chance_index: i32,
    pub physical_q: i64,
    pub physical_critical_q: i64,
    pub magic_q: i64,
    pub physical_skill_q: i64,
    pub physical_skill_critical_q: i64,
    pub magic_failure_q: i64,
}
/// NPC level gates and lists, including the source's unused magic minimum configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct NpcPenaltyRules {
    pub schema: u32,
    pub q: i64,
    pub source: Source,
    pub formula_source: Source,
    pub config: BTreeMap<String, Coefficient>,
    pub rows: Vec<PenaltyRow>,
}
/// Source eligibility flags are independent; physical skills use instanceof `L2Attackable`.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs, clippy::struct_excessive_bools)]
pub struct NpcPenaltyContext {
    pub target_level: u32,
    pub acting_player_level: Option<u32>,
    pub attackable: bool,
    pub l2attackable: bool,
    pub raid: bool,
    pub raid_minion: bool,
}
/// Source callsite determines the multiplier list and level-gap offset.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub enum NpcPenaltyPath {
    Physical { critical: bool },
    PhysicalSkill { critical: bool },
    Magical,
    MagicFailure,
}
impl NpcPenaltyRules {
    fn coefficient(&self, name: &str, index: usize) -> Result<ExactRatio, StatError> {
        let list = self.config.get(name).ok_or(StatError::Overflow)?;
        ExactRatio::parse_literal(
            list.values
                .get(index)
                .or_else(|| list.values.last())
                .ok_or(StatError::Overflow)?,
        )
    }
    /// First damage index is gap−1 (gap2 selects index1); failure index is gap−2.
    /// All lists saturate independently. Chance uses the DAMAGE minimum, per pinned source.
    pub fn multiplier(
        &self,
        context: NpcPenaltyContext,
        path: NpcPenaltyPath,
    ) -> Result<ExactRatio, StatError> {
        let minimum = self.coefficient("MinNPCLevelForDmgPenalty", 0)?.trunc()?;
        let Some(player) = context.acting_player_level else {
            return Ok(ExactRatio::whole(1));
        };
        let gap = context.target_level.saturating_sub(player);
        let (attackable, threshold) = match path {
            NpcPenaltyPath::PhysicalSkill { .. } => (context.l2attackable, 2),
            NpcPenaltyPath::MagicFailure => (context.attackable, 3),
            NpcPenaltyPath::Physical { .. } | NpcPenaltyPath::Magical => (context.attackable, 2),
        };
        if !attackable
            || context.raid
            || context.raid_minion
            || i128::from(context.target_level) < minimum
            || gap < threshold
        {
            return Ok(ExactRatio::whole(1));
        }
        let index = usize::try_from(
            gap.checked_sub(if matches!(path, NpcPenaltyPath::MagicFailure) {
                2
            } else {
                1
            })
            .ok_or(StatError::Overflow)?,
        )
        .map_err(|_| StatError::Overflow)?;
        match path {
            NpcPenaltyPath::Physical { critical } => self.coefficient(
                if critical {
                    "CritDmgPenaltyForLvLDifferences"
                } else {
                    "DmgPenaltyForLvLDifferences"
                },
                index,
            ),
            NpcPenaltyPath::PhysicalSkill { critical } => self
                .coefficient("SkillDmgPenaltyForLvLDifferences", index)?
                .checked_mul(self.coefficient(
                    if critical {
                        "CritDmgPenaltyForLvLDifferences"
                    } else {
                        "DmgPenaltyForLvLDifferences"
                    },
                    index,
                )?),
            NpcPenaltyPath::Magical => self.coefficient("SkillDmgPenaltyForLvLDifferences", index),
            NpcPenaltyPath::MagicFailure => {
                self.coefficient("SkillChancePenaltyForLvLDifferences", index)
            },
        }
    }
    /// Reject malformed provenance, absent/extra lists and inconsistent source projections.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_inner()
            .map_err(|_| "invalid NPC level penalty rules".into())
    }
    fn validate_inner(&self) -> Result<(), StatError> {
        let keys = [
            "MinNPCLevelForDmgPenalty",
            "MinNPCLevelForMagicPenalty",
            "DmgPenaltyForLvLDifferences",
            "CritDmgPenaltyForLvLDifferences",
            "SkillDmgPenaltyForLvLDifferences",
            "SkillChancePenaltyForLvLDifferences",
        ];
        if self.schema != 1
            || self.q != Q
            || !self.source.valid(false)
            || !self.formula_source.valid(false)
            || self.config.len() != keys.len()
        {
            return Err(StatError::Overflow);
        }
        for name in keys {
            let row = self.config.get(name).ok_or(StatError::Overflow)?;
            if row.values.is_empty()
                || row.values.len() != row.values_q.len()
                || !row.source.valid(false)
            {
                return Err(StatError::Overflow);
            }
            for (literal, q) in row.values.iter().zip(&row.values_q) {
                let exact = ExactRatio::parse_literal(literal)?;
                if exact.parts().0 <= 0 || exact != ExactRatio::new(i128::from(*q), i128::from(Q))?
                {
                    return Err(StatError::Overflow);
                }
            }
            if name.starts_with("MinNPC")
                && (row.values.len() != 1 || self.coefficient(name, 0)?.parts().1 != 1)
            {
                return Err(StatError::Overflow);
            }
        }
        if self
            .rows
            .iter()
            .map(|r| (r.gap_min, r.gap_max))
            .collect::<Vec<_>>()
            != [(2, 2), (3, 3), (4, 4), (5, 5), (6, -1)]
        {
            return Err(StatError::Overflow);
        }
        // Rows are documentary projections of the lists, not tunable independent behavior.
        for row in &self.rows {
            let damage_index =
                usize::try_from(row.gap_min.checked_sub(1).ok_or(StatError::Overflow)?)
                    .map_err(|_| StatError::Overflow)?
                    .min(
                        self.config
                            .get("SkillDmgPenaltyForLvLDifferences")
                            .ok_or(StatError::Overflow)?
                            .values
                            .len()
                            .checked_sub(1)
                            .ok_or(StatError::Overflow)?,
                    );
            let chance_index = if row.gap_min < 3 {
                -1
            } else {
                i32::try_from(
                    usize::try_from(row.gap_min.checked_sub(2).ok_or(StatError::Overflow)?)
                        .map_err(|_| StatError::Overflow)?
                        .min(
                            self.config
                                .get("SkillChancePenaltyForLvLDifferences")
                                .ok_or(StatError::Overflow)?
                                .values
                                .len()
                                .checked_sub(1)
                                .ok_or(StatError::Overflow)?,
                        ),
                )
                .map_err(|_| StatError::Overflow)?
            };
            if row.damage_index != i32::try_from(damage_index).map_err(|_| StatError::Overflow)?
                || row.chance_index != chance_index
            {
                return Err(StatError::Overflow);
            }
            let minimum = u32::try_from(self.coefficient("MinNPCLevelForDmgPenalty", 0)?.trunc()?)
                .map_err(|_| StatError::Overflow)?;
            let target = minimum
                .checked_add(row.gap_min)
                .ok_or(StatError::Overflow)?;
            let c = NpcPenaltyContext {
                target_level: target,
                acting_player_level: Some(minimum),
                attackable: true,
                l2attackable: true,
                raid: false,
                raid_minion: false,
            };
            for (path, expected) in [
                (NpcPenaltyPath::Physical { critical: false }, row.physical_q),
                (NpcPenaltyPath::Physical { critical: true }, row.physical_critical_q),
                (NpcPenaltyPath::PhysicalSkill { critical: false }, row.physical_skill_q),
                (NpcPenaltyPath::PhysicalSkill { critical: true }, row.physical_skill_critical_q),
                (NpcPenaltyPath::Magical, row.magic_q),
                (NpcPenaltyPath::MagicFailure, row.magic_failure_q),
            ] {
                if self.multiplier(c, path)?.to_q()? != expected {
                    return Err(StatError::Overflow);
                }
            }
        }
        Ok(())
    }
}

/// One source hit-condition bonus. Rain is retained but never applied by the pinned getter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct HitCondition {
    pub condition: String,
    pub percent: i32,
    pub used: bool,
    pub source: Source,
}
/// Source hit-condition table; Phase3's flat, daytime, no-facing profile selects front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct HitConditionRules {
    pub schema: u32,
    pub source: Source,
    pub formula_source: Source,
    pub base_percent: i32,
    pub height_threshold: i32,
    pub minimum_q: i64,
    pub rows: Vec<HitCondition>,
}
/// Exclusive positional condition (back before front before side in the source).
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub enum HitPosition {
    Back,
    Front,
    Side,
}
impl HitConditionRules {
    /// Pure source getter. Z difference uses L2 units, not tiles; rain is intentionally absent.
    pub fn multiplier(
        &self,
        height: i64,
        night: bool,
        position: HitPosition,
    ) -> Result<ExactRatio, StatError> {
        let mut percent = i64::from(self.base_percent);
        let position = match position {
            HitPosition::Back => "back",
            HitPosition::Front => "front",
            HitPosition::Side => "side",
        };
        for row in &self.rows {
            if row.condition == position
                || (row.condition == "dark" && night)
                || (row.condition == "high" && height > i64::from(self.height_threshold))
                || (row.condition == "low"
                    && height
                        < i64::from(self.height_threshold)
                            .checked_neg()
                            .ok_or(StatError::Overflow)?)
            {
                percent = percent
                    .checked_add(i64::from(row.percent))
                    .ok_or(StatError::Overflow)?;
            }
        }
        ExactRatio::new(i128::from(percent.max(0)), 100)
    }
    /// Preserve the pinned getter's supported condition set and strict source provenance.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || !self.source.valid(true)
            || !self.formula_source.valid(false)
            || self.base_percent != 100
            || self.height_threshold != 50
            || self.minimum_q != 0
            || self
                .rows
                .iter()
                .map(|r| r.condition.as_str())
                .collect::<Vec<_>>()
                != ["back", "dark", "front", "high", "low", "rain", "side"]
            || self
                .rows
                .iter()
                .any(|r| !r.source.valid(true) || r.used != (r.condition != "rain"))
        {
            return Err("invalid hit condition rules".into());
        }
        Ok(())
    }
}
/// Required separately hashed source bundle for every new V5 epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct NpcSkillRules {
    pub penalty: NpcPenaltyRules,
    pub hit_condition: HitConditionRules,
}
impl NpcSkillRules {
    /// Both tables are mandatory, even when the current zone only contains low-level NPCs.
    pub fn validate(&self) -> Result<(), String> {
        self.penalty.validate()?;
        self.hit_condition.validate()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    #[test]
    fn every_independent_npc_oracle_vector_matches() {
        let penalty: NpcPenaltyRules = toml::from_str(include_str!(
            "../../../../../packages/data/tables-phase3/npc_level_penalty.toml"
        ))
        .unwrap();
        let conditions: HitConditionRules = toml::from_str(include_str!(
            "../../../../../packages/data/tables-phase3/hit_condition_bonus.toml"
        ))
        .unwrap();
        penalty.validate().unwrap();
        conditions.validate().unwrap();
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle-npc-phase3.json"
        ))
        .unwrap();
        let mut count = 0;
        for (name, cases) in oracle["formulas"].as_object().unwrap() {
            for case in cases.as_array().unwrap() {
                let p = &case["inputs"];
                let u = |key: &str| u32::try_from(p[key].as_u64().unwrap()).unwrap();
                let flag = |key: &str| p[key].as_bool().unwrap();
                let expected = ExactRatio::parse_literal(
                    case[if name == "getConditionBonus" {
                        "exact_rational"
                    } else {
                        "exact_multiplier"
                    }]
                    .as_str()
                    .unwrap(),
                )
                .unwrap();
                if name == "getConditionBonus" {
                    let position = match p["position"].as_str().unwrap() {
                        "back" => HitPosition::Back,
                        "front" => HitPosition::Front,
                        "side" => HitPosition::Side,
                        _ => panic!("unknown position"),
                    };
                    assert_eq!(
                        conditions
                            .multiplier(
                                p["z_difference"].as_i64().unwrap(),
                                flag("night"),
                                position
                            )
                            .unwrap(),
                        expected
                    );
                } else {
                    let context = NpcPenaltyContext {
                        target_level: u("target_level"),
                        acting_player_level: flag("has_acting_player")
                            .then(|| u("acting_player_level")),
                        attackable: flag("is_attackable"),
                        l2attackable: flag("is_l2attackable"),
                        raid: flag("is_raid"),
                        raid_minion: flag("is_raid_minion"),
                    };
                    let path = match name.as_str() {
                        "calcPhysDam_npcLevelPenalty" => NpcPenaltyPath::Physical {
                            critical: flag("critical"),
                        },
                        "calcSkillPhysDam_npcLevelPenalty" => NpcPenaltyPath::PhysicalSkill {
                            critical: flag("critical"),
                        },
                        "calcMagicDam_npcLevelPenalty" => NpcPenaltyPath::Magical,
                        "calcMagicSuccess_npcChancePenalty" => NpcPenaltyPath::MagicFailure,
                        _ => panic!("unknown oracle {name}"),
                    };
                    let multiplier = penalty.multiplier(context, path).unwrap();
                    assert_eq!(multiplier, expected, "{name} {p}");
                    assert_eq!(multiplier.to_q().unwrap(), case["multiplier_q"].as_i64().unwrap());
                    if matches!(path, NpcPenaltyPath::MagicFailure) {
                        let difference = i32::try_from(u("target_level"))
                            .unwrap()
                            .checked_sub(i32::try_from(u("skill_magic_level")).unwrap())
                            .unwrap();
                        let rate = super::super::skill_math::magic_success_rate(
                            difference,
                            multiplier,
                            ExactRatio::parse_literal(p["resist_modifier"].as_str().unwrap())
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(i64::from(rate), case["success_rate"].as_i64().unwrap());
                        assert_eq!(
                            i64::from(u("roll")) < i64::from(rate),
                            case["lands_below_rate"].as_bool().unwrap()
                        );
                        assert!(!case["lands_at_rate"].as_bool().unwrap());
                    } else {
                        let damage = ExactRatio::whole(i64::from(u("incoming_damage")))
                            .checked_mul(multiplier)
                            .unwrap();
                        assert_eq!(
                            damage,
                            ExactRatio::parse_literal(case["exact_rational"].as_str().unwrap())
                                .unwrap()
                        );
                        assert_eq!(
                            damage.trunc().unwrap(),
                            i128::from(case["whole_damage"].as_i64().unwrap())
                        );
                    }
                }
                count += 1;
            }
        }
        assert_eq!(count, 207);
    }
}
