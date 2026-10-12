//! Rebuildable Phase 3 calculators. Pinned game `model/stats/Calculator.java:110-117`
//! preserves equal-priority insertion order. No calculator is canonical snapshot state.

use std::collections::BTreeMap;

use super::scaled::{add, mul, narrow, round_div, sub};
use super::skill::{RankValue, SkillCondition, SkillHandler, SkillModifier};
use super::skill_math::{evasion_builtin, ExactRatio, RootExpression};
use super::{StatError, Q};

/// Stable key across passive and timed sources. Sequence zero is reserved for builtins.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[allow(missing_docs)]
pub struct FunctionKey {
    pub stat: String,
    pub priority: i32,
    pub install_sequence: u64,
    pub handler_ordinal: u32,
    pub function_ordinal: u32,
}

/// Reconstructed operation; integer casts inside builtins are distinct from getter casts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum FunctionOp {
    /// Source SET (priority zero unless overridden).
    Set(i64),
    /// Source MUL (priority20 unless overridden).
    Mul(i64),
    /// Source DIV with strictly positive divisor.
    Div(i64),
    /// Source ADD (priority30 unless overridden).
    Add(i64),
    /// Source SUB; signed intermediate is retained until the getter.
    Sub(i64),
    /// Entire builtin product evaluated before converting to Q, not one Q rounding per term.
    Product(ExactRatio),
    /// `FuncAtkEvasion.java:37-61`: truncate its complete value BEFORE later modifiers.
    Evasion {
        /// Raw DEX.
        dex: u32,
        /// Source level.
        level: u32,
        /// Player/NPC high-level branch.
        player: bool,
    },
    /// `FuncAtkAccuracy.java:42-51`: no intermediate whole cast.
    Accuracy {
        /// Raw DEX.
        dex: u32,
        /// Source level.
        level: u32,
    },
}
impl FunctionOp {
    fn apply(&self, current: i64) -> Result<i64, StatError> {
        let q = i128::from(Q);
        let value = i128::from(current);
        match self {
            Self::Set(amount) => Ok(*amount),
            Self::Mul(factor) => narrow(round_div(mul(value, i128::from(*factor))?, q)?),
            Self::Div(divisor) => narrow(round_div(mul(value, q)?, i128::from(*divisor))?),
            Self::Add(amount) => narrow(add(value, i128::from(*amount))?),
            Self::Sub(amount) => narrow(sub(value, i128::from(*amount))?),
            Self::Product(factor) => ExactRatio::new(value, q)?.checked_mul(*factor)?.to_q(),
            Self::Evasion { dex, level, player } => {
                let result = evasion_builtin(ExactRatio::new(value, q)?, *dex, *level, *player)?;
                let floor = result.floor_scaled(1)?;
                // Java (int) truncates negative values toward zero, unlike floor.
                let whole = if floor < 0 && result.checked_cmp(ExactRatio::whole(floor))?.is_gt() {
                    floor.checked_add(1).ok_or(StatError::Overflow)?
                } else {
                    floor
                };
                narrow(mul(i128::from(whole), q)?)
            },
            Self::Accuracy { dex, level } => {
                let extra = level
                    .saturating_sub(76)
                    .checked_mul(u32::from(*level > 77))
                    .and_then(|first| first.checked_add(level.saturating_sub(69)))
                    .ok_or(StatError::Overflow)?;
                let offset = ExactRatio::new(value, q)?
                    .checked_add(ExactRatio::whole(i64::from(*level)))?
                    .checked_add(ExactRatio::whole(i64::from(extra)))?;
                let root = RootExpression::new(
                    offset,
                    ExactRatio::whole(6),
                    ExactRatio::whole(i64::from(*dex)),
                )?;
                // Q quantization uses nearest, ties up. No premature whole rounding.
                root.checked_mul(ExactRatio::whole(Q))?.round()
            },
        }
    }
}

/// Declarative function. Hook-owned function ordinal zero precedes XML functions 1..n.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct StatFunction {
    pub key: FunctionKey,
    pub operation: FunctionOp,
    pub conditions: Vec<SkillCondition>,
}

/// Explicit read-only condition inputs. Weapon admission currently supplies SWORD for the
/// equipped Squire's Sword; resurrection is actor eligibility, never a guessed constant.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct FunctionConditions<'a> {
    pub weapon_kind: &'a str,
    pub can_resurrect: bool,
}
impl FunctionConditions<'_> {
    /// Evaluate every supported recursive condition without a mutable game-state callback.
    pub fn matches(self, condition: &SkillCondition) -> Result<bool, StatError> {
        match condition {
            SkillCondition::UsingKind { weapon_kinds } => {
                Ok(weapon_kinds.iter().any(|kind| kind == self.weapon_kind))
            },
            SkillCondition::PlayerCanResurrect { value } => Ok(*value == self.can_resurrect),
            SkillCondition::And { conditions, .. } => {
                for condition in conditions {
                    if !self.matches(condition)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            },
            SkillCondition::Or { conditions, .. } => {
                for condition in conditions {
                    if self.matches(condition)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            },
            SkillCondition::Not { conditions, .. } if conditions.len() == 1 => self
                .matches(conditions.first().ok_or(StatError::Overflow)?)
                .map(|value| !value),
            SkillCondition::PlayerLevelRange { .. }
            | SkillCondition::PlayerInsideZoneIds { .. }
            | SkillCondition::PlayerResting { .. }
            | SkillCondition::PlayerStanding { .. }
            | SkillCondition::PlayerMoving { .. }
            | SkillCondition::PlayerRunning { .. }
            | SkillCondition::PlayerCanEscape { .. }
            | SkillCondition::PlayerFlyMounted { .. }
            | SkillCondition::UsingSlot { .. }
            | SkillCondition::PlayerWeight { .. }
            | SkillCondition::PlayerInventorySize { .. }
            | SkillCondition::Not { .. }
            | SkillCondition::Unsupported { .. } => Err(StatError::Overflow),
        }
    }
}

/// Functions sorted by source insertion identity; rebuilt from immutable definitions and
/// canonical installed sources. Duplicate keys are corrupt state, never last-writer-wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatCalculator {
    functions: BTreeMap<FunctionKey, StatFunction>,
}
impl serde::Serialize for StatCalculator {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.functions.values())
    }
}
impl StatCalculator {
    /// Restore all declarative functions, preserving exact priority and insertion order.
    pub fn try_new(functions: impl IntoIterator<Item = StatFunction>) -> Result<Self, StatError> {
        let mut result = Self::default();
        for function in functions {
            if function.key.stat.is_empty()
                || matches!(function.operation, FunctionOp::Div(divisor) if divisor <= 0)
                || result.functions.contains_key(&function.key)
            {
                return Err(StatError::Overflow);
            }
            result.functions.insert(function.key.clone(), function);
        }
        Ok(result)
    }

    /// Raw Q fold; `CharStat` getter clamps/casts happen only after all functions.
    pub fn fold(
        &self,
        stat: &str,
        initial_q: i64,
        conditions: FunctionConditions<'_>,
    ) -> Result<i64, StatError> {
        let mut value = initial_q;
        for function in self
            .functions
            .values()
            .filter(|function| function.key.stat == stat)
        {
            let mut eligible = true;
            for condition in &function.conditions {
                if !conditions.matches(condition)? {
                    eligible = false;
                    break;
                }
            }
            if eligible {
                value = function.operation.apply(value)?;
            }
        }
        Ok(value)
    }
}

/// Compile a generic source function. Caller offsets its XML index by one to reserve zero
/// for the same handler's hook function. Explicit XML order overrides operation defaults.
pub fn compile_modifier(
    modifier: &SkillModifier,
    rank: u32,
    max_rank: u32,
    install_sequence: u64,
    handler_ordinal: u32,
    function_ordinal: u32,
) -> Result<StatFunction, StatError> {
    if rank == 0 || rank > max_rank || install_sequence == 0 || function_ordinal == 0 {
        return Err(StatError::Overflow);
    }
    let index = usize::try_from(rank.checked_sub(1).ok_or(StatError::Overflow)?)
        .map_err(|_| StatError::Overflow)?;
    let value = modifier.value.get(index).ok_or(StatError::Overflow)?;
    let amount = ExactRatio::parse_literal(value)?.to_q()?;
    let (priority, operation) = match modifier.op.as_str() {
        "set" => (0, FunctionOp::Set(amount)),
        "mul" => (20, FunctionOp::Mul(amount)),
        "div" if amount > 0 => (20, FunctionOp::Div(amount)),
        "add" => (30, FunctionOp::Add(amount)),
        "sub" => (30, FunctionOp::Sub(amount)),
        _ => return Err(StatError::Overflow),
    };
    let priority = match &modifier.order {
        Some(order) => *order.at(rank, max_rank).ok_or(StatError::Overflow)?,
        None => priority,
    };
    Ok(StatFunction {
        key: FunctionKey {
            stat: modifier.stat.clone(),
            priority,
            install_sequence,
            handler_ordinal,
            function_ordinal,
        },
        operation,
        conditions: modifier.conditions.clone(),
    })
}

/// Reconstruct MaxHp/MaxMp's hook-owned order-one function. This NEVER invokes onStart or
/// grants healing. Live installation separately applies healing using captured pre-install
/// current/maximum values, then installs generic XML functions. `pump/MaxHp.java:50-98` and
/// `pump/MaxMp.java:48-96` are the source; both use priority1 rather than MUL20/ADD30.
pub fn compile_maximum_hook(
    handler: &SkillHandler,
    rank: u32,
    max_rank: u32,
    install_sequence: u64,
    handler_ordinal: u32,
) -> Result<Option<StatFunction>, StatError> {
    let stat = match handler.name.as_str() {
        "MaxHp" => "maxHp",
        "MaxMp" => "maxMp",
        _ => return Ok(None),
    };
    if rank == 0 || rank > max_rank || install_sequence == 0 {
        return Err(StatError::Overflow);
    }
    let parameter = |name: &str| -> Result<Option<&str>, StatError> {
        handler
            .params
            .get(name)
            .map(|value: &RankValue<String>| {
                value
                    .at(rank, max_rank)
                    .map(String::as_str)
                    .ok_or(StatError::Overflow)
            })
            .transpose()
    };
    let power = ExactRatio::parse_literal(parameter("power")?.unwrap_or("0"))?;
    let operation = match parameter("type")?.unwrap_or("DIFF") {
        "DIFF" => FunctionOp::Add(power.to_q()?),
        "PER" => FunctionOp::Mul(
            ExactRatio::whole(1)
                .checked_add(power.checked_div(ExactRatio::whole(100))?)?
                .to_q()?,
        ),
        _ => return Err(StatError::Overflow),
    };
    Ok(Some(StatFunction {
        key: FunctionKey {
            stat: stat.into(),
            priority: 1,
            install_sequence,
            handler_ordinal,
            function_ordinal: 0,
        },
        operation,
        conditions: Vec::new(),
    }))
}

/// Standard funcs from `Formulas.java:134-174,210-227`. Initial values must be raw template,
/// weapon and growth values, never a rounded legacy `StatSheet`. Armor/jewelry subtraction
/// is absent because no such equipment is available in the supported Phase 3 capability set.
pub fn standard_functions(
    base: &crate::domain::BaseStats,
    bonuses: &super::StatBonusTables,
    level_mod: super::Scaled,
    level: u32,
    player: bool,
    weapon_equipped: bool,
) -> Result<Vec<StatFunction>, StatError> {
    if !(1..=85).contains(&level) {
        return Err(StatError::LevelOutOfRange(level));
    }
    let bonus = |kind: super::StatKind| -> Result<ExactRatio, StatError> {
        ExactRatio::new(i128::from(bonuses.bonus(kind, kind.of(base))?.raw()), i128::from(Q))
    };
    let lm = ExactRatio::new(i128::from(level_mod.raw()), i128::from(Q))?;
    let strength = bonus(super::StatKind::Str)?;
    let dexterity = bonus(super::StatKind::Dex)?;
    let constitution = bonus(super::StatKind::Con)?;
    let mentality = bonus(super::StatKind::Men)?;
    let intelligence = bonus(super::StatKind::Int)?;
    let wit = bonus(super::StatKind::Wit)?;
    let mut operations = vec![
        ("maxHp", FunctionOp::Product(constitution)),
        ("maxMp", FunctionOp::Product(mentality)),
        ("pAtk", FunctionOp::Product(strength.checked_mul(lm)?)),
        ("pDef", FunctionOp::Product(lm)),
        (
            "mAtk",
            FunctionOp::Product(
                intelligence
                    .checked_mul(intelligence)?
                    .checked_mul(lm)?
                    .checked_mul(lm)?,
            ),
        ),
        ("mDef", FunctionOp::Product(mentality.checked_mul(lm)?)),
        ("pAtkSpd", FunctionOp::Product(dexterity)),
        ("mAtkSpd", FunctionOp::Product(wit)),
        ("runSpd", FunctionOp::Product(dexterity)),
        ("critRate", FunctionOp::Product(dexterity.checked_mul(ExactRatio::whole(10))?)),
        (
            "accCombat",
            FunctionOp::Accuracy {
                dex: base.dex,
                level,
            },
        ),
        (
            "rEvas",
            FunctionOp::Evasion {
                dex: base.dex,
                level,
                player,
            },
        ),
    ];
    if player {
        operations.push(("maxCp", FunctionOp::Product(constitution)));
    }
    // FuncMAtkCritical.java:42-46: NPCs and equipped players get the first x10;
    // CharStat.getMCriticalHit truncates and applies its separate x10 afterwards.
    if !player || weapon_equipped {
        operations
            .push(("mCritRate", FunctionOp::Product(wit.checked_mul(ExactRatio::whole(10))?)));
    }
    Ok(operations
        .into_iter()
        .map(|(stat, operation)| StatFunction {
            key: FunctionKey {
                stat: stat.into(),
                priority: 1,
                install_sequence: 0,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation,
            conditions: Vec::new(),
        })
        .collect())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    const CONDITIONS: FunctionConditions<'static> = FunctionConditions {
        weapon_kind: "SWORD",
        can_resurrect: false,
    };

    fn function(stat: &str, priority: i32, sequence: u64, operation: FunctionOp) -> StatFunction {
        StatFunction {
            key: FunctionKey {
                stat: stat.into(),
                priority,
                install_sequence: sequence,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation,
            conditions: Vec::new(),
        }
    }

    #[test]
    fn explicit_priority_and_shared_sequence_control_noncommuting_functions() {
        let add = function("maxHp", 30, 1, FunctionOp::Add(10 * Q));
        let times = function("maxHp", 20, 2, FunctionOp::Mul(2 * Q));
        let calculator = StatCalculator::try_new([add.clone(), times.clone()]).unwrap();
        assert_eq!(calculator.fold("maxHp", 100 * Q, CONDITIONS).unwrap(), 210 * Q);
        let mut early_add = add.clone();
        early_add.key.priority = 10;
        assert_eq!(
            StatCalculator::try_new([times.clone(), early_add])
                .unwrap()
                .fold("maxHp", 100 * Q, CONDITIONS)
                .unwrap(),
            220 * Q
        );
        let mut same_priority = add;
        same_priority.key.priority = 20;
        assert_eq!(
            StatCalculator::try_new([times.clone(), same_priority.clone()])
                .unwrap()
                .fold("maxHp", 100 * Q, CONDITIONS)
                .unwrap(),
            220 * Q
        );
        same_priority.key.install_sequence = 3;
        assert_eq!(
            StatCalculator::try_new([same_priority, times])
                .unwrap()
                .fold("maxHp", 100 * Q, CONDITIONS)
                .unwrap(),
            210 * Q
        );
    }

    #[test]
    fn builtin_evasion_cast_precedes_both_multiplication_and_addition() {
        let functions = [
            function(
                "rEvas",
                1,
                0,
                FunctionOp::Evasion {
                    dex: 20,
                    level: 1,
                    player: true,
                },
            ),
            function("rEvas", 20, 2, FunctionOp::Mul(1_100_000)),
            function("rEvas", 30, 1, FunctionOp::Add(400_000)),
        ];
        assert_eq!(functions[0].operation.apply(0).unwrap(), 27 * Q);
        let value = StatCalculator::try_new(functions)
            .unwrap()
            .fold("rEvas", 0, CONDITIONS)
            .unwrap();
        assert_eq!(value, 30_100_000);
        assert_eq!(round_div(value.into(), Q.into()).unwrap(), 30);
        // Negative intermediate uses Java truncation rather than floor.
        let negative = FunctionOp::Evasion {
            dex: 1,
            level: 1,
            player: true,
        };
        assert_eq!(negative.apply(-8_400_000).unwrap(), -Q);
    }

    #[test]
    fn maximum_hook_is_order_one_and_restoration_never_grants_resources() {
        let handler = SkillHandler {
            name: "MaxHp".into(),
            section: "GENERAL".into(),
            params: [
                ("type".into(), RankValue::Scalar("PER".into())),
                ("power".into(), RankValue::Scalar("50".into())),
                ("heal".into(), RankValue::Scalar("true".into())),
            ]
            .into_iter()
            .collect(),
            stat_children: Vec::new(),
            conditions: Vec::new(),
            raw_conditions: Vec::new(),
            section_conditions: Vec::new(),
            section_raw_conditions: Vec::new(),
        };
        let hook = compile_maximum_hook(&handler, 1, 1, 2, 0).unwrap().unwrap();
        assert_eq!(hook.key.priority, 1);
        assert_eq!(hook.key.function_ordinal, 0);
        let calculator = StatCalculator::try_new([
            function("maxHp", 1, 0, FunctionOp::Product(ExactRatio::whole(2))),
            hook,
            function("maxHp", 30, 1, FunctionOp::Add(20 * Q)),
        ])
        .unwrap();
        assert_eq!(calculator.fold("maxHp", 100 * Q, CONDITIONS).unwrap(), 320 * Q);
        // No resource argument/output exists here: replay/admission reconstruction is declarative.
        assert_eq!(calculator.clone(), calculator);
    }

    #[test]
    fn signed_subtraction_conditions_and_invalid_functions_do_not_silently_clamp() {
        let mut subtract = function("maxHp", 30, 1, FunctionOp::Sub(200 * Q));
        subtract.conditions = vec![SkillCondition::UsingKind {
            weapon_kinds: vec!["BLUNT".into()],
        }];
        let calculator = StatCalculator::try_new([subtract.clone()]).unwrap();
        assert_eq!(calculator.fold("maxHp", 100 * Q, CONDITIONS).unwrap(), 100 * Q);
        assert_eq!(
            calculator
                .fold(
                    "maxHp",
                    100 * Q,
                    FunctionConditions {
                        weapon_kind: "BLUNT",
                        ..CONDITIONS
                    }
                )
                .unwrap(),
            -100 * Q
        );
        assert!(StatCalculator::try_new([subtract.clone(), subtract]).is_err());
        assert!(StatCalculator::try_new([function("maxHp", 20, 1, FunctionOp::Div(0))]).is_err());
        assert!(FunctionOp::Mul(i64::MAX).apply(i64::MAX).is_err());
        assert_eq!(FunctionOp::Sub(-Q).apply(0).unwrap(), Q);
    }

    #[test]
    fn calculator_executes_every_independent_evasion_oracle_vector() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        let vectors = fixture["formulas"]["FuncAtkEvasion"].as_array().unwrap();
        assert!(!vectors.is_empty());
        for vector in vectors {
            let p = &vector["inputs"];
            let mut functions = vec![function(
                "rEvas",
                1,
                0,
                FunctionOp::Evasion {
                    dex: u32::try_from(p["dex"].as_u64().unwrap()).unwrap(),
                    level: u32::try_from(p["level"].as_u64().unwrap()).unwrap(),
                    player: p["is_player"].as_bool().unwrap(),
                },
            )];
            let modifier = &p["modifier"];
            if modifier["op"] != "none" {
                let value = &modifier["val"];
                let text = value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_owned);
                let value = ExactRatio::parse_literal(&text).unwrap().to_q().unwrap();
                let operation = match modifier["op"].as_str().unwrap() {
                    "mul" => FunctionOp::Mul(value),
                    "add" => FunctionOp::Add(value),
                    _ => unreachable!(),
                };
                functions.push(function("rEvas", 20, 1, operation));
            }
            let value = StatCalculator::try_new(functions)
                .unwrap()
                .fold("rEvas", 0, CONDITIONS)
                .unwrap();
            assert_eq!(
                round_div(value.into(), Q.into()).unwrap(),
                i128::from(
                    vector["rounded_result"]["evasion_with_intermediate_cast"]
                        .as_i64()
                        .unwrap()
                ),
                "{vector}"
            );
        }
    }
}
