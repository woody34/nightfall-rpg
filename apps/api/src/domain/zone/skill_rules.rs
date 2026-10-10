//! Immutable, source-provenanced policy tables carried by the V5 rules snapshot.
//! Decimal literals remain strings until checked exact evaluation, never binary host floats.

use super::skill_math::ExactRatio;
use serde::{Deserialize, Serialize};

/// A pinned source file. Both repository and revision participate in the rules hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RuleSource {
    pub repository: String,
    pub revision: String,
    pub file: String,
}

impl RuleSource {
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
        self.repository == repository && self.revision == revision && !self.file.is_empty()
    }
}

/// `PvP` flag deadlines, in zone ticks, from the generated pinned policy table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PvpFlagRules {
    pub pvp_flag_ticks: u64,
    pub pvp_flag_low_ticks: u64,
}

/// `Formulas.java:1830-1848`. Summon terms are retained as source metadata, not executable
/// permission to introduce summons into Phase 3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct KarmaGainRules {
    pub base_gain: u32,
    pub low_pk_count_threshold: u32,
    pub low_pk_count_multiplier: String,
    pub low_pk_count_scale: u32,
    pub mid_pk_count_threshold: u32,
    pub mid_pk_count_multiplier: String,
    pub mid_pk_count_base: String,
    pub mid_pk_count_scale: u32,
    pub summon_multiplier: String,
    pub summon_scale: u32,
    pub summon_offset: u32,
    pub summon_max: u32,
}

/// `Formulas.java:1814-1821` and `pcKarmaIncrease.xml:3-87`; index zero is level one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct KarmaLostRules {
    pub divisor: u32,
    pub rate_karma_lost: u32,
    pub multipliers: Vec<String>,
}

/// Complete generated `PvP` table, including provenance and exact decimal literals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PvpRules {
    pub source_game: RuleSource,
    pub source_datapack: RuleSource,
    pub flags: PvpFlagRules,
    pub karma_gain: KarmaGainRules,
    pub karma_lost: KarmaLostRules,
}

impl PvpRules {
    /// Validate the supported source formula shape and all level divisors.
    pub fn validate(&self) -> Result<(), String> {
        if !self.source_game.valid(false) || !self.source_datapack.valid(true) {
            return Err("PvP rules source provenance mismatch".into());
        }
        let gain = &self.karma_gain;
        if self.flags.pvp_flag_ticks == 0
            || self.flags.pvp_flag_low_ticks == 0
            || gain.base_gain != 43_200
            || gain.low_pk_count_threshold != 99
            || gain.low_pk_count_scale != 720
            || gain.mid_pk_count_threshold != 180
            || gain.mid_pk_count_scale != 720
            || gain.summon_scale != 240
            || gain.summon_offset != 150
            || gain.summon_max != 10_800
            || !literal_eq(&gain.low_pk_count_multiplier, 1, 2)
            || !literal_eq(&gain.mid_pk_count_multiplier, 1, 8)
            || !literal_eq(&gain.mid_pk_count_base, 151, 4)
            || !literal_eq(&gain.summon_multiplier, 3, 8)
            || self.karma_lost.divisor != 30
            || self.karma_lost.rate_karma_lost == 0
            || self.karma_lost.multipliers.len() != 85
            || self
                .karma_lost
                .multipliers
                .iter()
                .any(|value| !positive_literal(value))
        {
            return Err("PvP rules contain an invalid or unsupported source coefficient".into());
        }
        Ok(())
    }
}

/// `L2Character.java:192`: millisecond period and its exact zone-tick equivalent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenPeriod {
    pub hp_regenerate_period_ms: u32,
    pub hp_regenerate_period_ticks: u32,
}

/// Post-stat movement multipliers (`Formulas.java:335-340,420-426,446-452`).
/// Sitting remains metadata until an actual supported sitting state exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenMovement {
    pub sitting: String,
    pub standing: String,
    pub moving: String,
}

/// Configured regeneration multipliers after generic stat Funcs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenMultipliers {
    #[serde(rename = "hp_regen_multiplier")]
    pub hp: String,
    #[serde(rename = "mp_regen_multiplier")]
    pub mp: String,
    #[serde(rename = "cp_regen_multiplier")]
    pub cp: String,
}

/// Source dependencies; HP/CP use CON and MP uses MEN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenDependencies {
    #[serde(rename = "hp_stat")]
    pub hp: String,
    #[serde(rename = "mp_stat")]
    pub mp: String,
    #[serde(rename = "cp_stat")]
    pub cp: String,
}

/// Initial clamp and the source-template locator. The locator is provenance, never an
/// instruction for domain/replay code to read a filesystem or replace missing base data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenBaseRules {
    pub min_initial_regen: u32,
    pub base_regen_source: String,
}

/// Complete immutable generated regeneration policy (entity base profiles are separate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RegenRules {
    pub source_game: RuleSource,
    pub source_datapack: RuleSource,
    pub period: RegenPeriod,
    pub movement_multipliers: RegenMovement,
    pub character_multipliers: RegenMultipliers,
    pub stat_dependencies: RegenDependencies,
    pub rules: RegenBaseRules,
}

impl RegenRules {
    /// Refuse incomplete/impossible periods, unsupported stat dependencies or bad literals.
    pub fn validate(&self) -> Result<(), String> {
        if !self.source_game.valid(false) || !self.source_datapack.valid(true) {
            return Err("regen rules source provenance mismatch".into());
        }
        if self.period.hp_regenerate_period_ticks == 0
            || self.period.hp_regenerate_period_ticks.checked_mul(100)
                != Some(self.period.hp_regenerate_period_ms)
            || self.rules.min_initial_regen != 1
            || self.rules.base_regen_source.is_empty()
            || self.stat_dependencies.hp != "CON"
            || self.stat_dependencies.mp != "MEN"
            || self.stat_dependencies.cp != "CON"
            || [
                &self.movement_multipliers.sitting,
                &self.movement_multipliers.standing,
                &self.movement_multipliers.moving,
                &self.character_multipliers.hp,
                &self.character_multipliers.mp,
                &self.character_multipliers.cp,
            ]
            .iter()
            .any(|literal| !positive_literal(literal))
        {
            return Err("regen rules contain an invalid or unsupported period/coefficient".into());
        }
        Ok(())
    }
}

fn literal_eq(value: &str, numerator: i128, denominator: i128) -> bool {
    ExactRatio::parse_literal(value)
        .ok()
        .zip(ExactRatio::new(numerator, denominator).ok())
        .is_some_and(|(actual, expected)| actual == expected)
}

fn positive_literal(value: &str) -> bool {
    ExactRatio::parse_literal(value).is_ok_and(|v| v.parts().0 > 0 && v.binary64().is_ok())
}

/// One generated source constant, including its exact source location and assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SourceConstant {
    pub value: u32,
    pub file: String,
    pub line: u32,
    pub symbol: String,
    pub literal: String,
}

/// Additive C5 constants from tables/formulas.toml. Legacy physical rule hashes exclude
/// this new table; the complete V5 rules hash includes it with its source metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3Constants {
    #[serde(rename = "MinAbnormalStateSuccessRate")]
    pub min_abnormal_success: SourceConstant,
    #[serde(rename = "MaxAbnormalStateSuccessRate")]
    pub max_abnormal_success: SourceConstant,
    #[serde(rename = "MaxMAtkSpeed")]
    pub max_magic_attack_speed: SourceConstant,
    #[serde(rename = "MaxPAtkSpeed")]
    pub max_physical_attack_speed: SourceConstant,
    #[serde(rename = "MaxEvasion")]
    pub max_evasion: SourceConstant,
}

impl Phase3Constants {
    /// Require exactly the pinned C5 source values; custom rule policy needs a new version.
    pub fn validate(&self) -> Result<(), String> {
        for (constant, name, value, line) in [
            (&self.min_abnormal_success, "MinAbnormalStateSuccessRate", 10, 277),
            (&self.max_abnormal_success, "MaxAbnormalStateSuccessRate", 90, 278),
            (&self.max_magic_attack_speed, "MaxMAtkSpeed", 1999, 268),
            (&self.max_physical_attack_speed, "MaxPAtkSpeed", 1500, 264),
            (&self.max_evasion, "MaxEvasion", 250, 272),
        ] {
            if constant.value != value
                || constant.line != line
                || constant.symbol != name
                || constant.literal != format!("{name} = {value}")
                || constant.file != "src/main/resources/config/character.properties"
            {
                return Err(format!("Phase3 constant {name} differs from pinned C5 source"));
            }
        }
        Ok(())
    }
}
