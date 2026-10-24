//! Phase 3 regeneration scheduling and staged resource restoration. The zone calls this
//! during phase3 before movement; the movement input is the state at that phase boundary.

use serde::{Deserialize, Serialize};

use super::skill_math::{regen_initial, restore_resource, ExactRatio};
use super::skill_rules::RegenRules;
use super::stat_functions::{FunctionConditions, StatCalculator};
use super::{ResourceFractions, StatError, Tick};

/// Canonical entity schedule. Death dequeues; admission/revival starts a fresh period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenSchedule {
    /// None only for dead/otherwise ineligible combatants. No off-line catch-up credit.
    pub next_at: Option<Tick>,
}
impl RegenSchedule {
    /// Actual admission/revival tick, using the immutable source period (normally30).
    pub fn start(now: Tick, alive: bool, rules: &RegenRules) -> Result<Self, StatError> {
        Ok(Self {
            next_at: if alive {
                Some(next_tick(now, rules)?)
            } else {
                None
            },
        })
    }
    /// Death invalidates pending regeneration without creating credit.
    pub fn stop(&mut self) {
        self.next_at = None;
    }
    /// Take at most one due pulse. No wall-time/snapshot catch-up; the caller must stage this
    /// alongside the resource result. Overflow leaves the schedule unchanged.
    pub fn take_due(&mut self, now: Tick, rules: &RegenRules) -> Result<bool, StatError> {
        if self.next_at.is_none_or(|deadline| deadline > now) {
            return Ok(false);
        }
        let next = next_tick(now, rules)?;
        self.next_at = Some(next);
        Ok(true)
    }
}
fn next_tick(now: Tick, rules: &RegenRules) -> Result<Tick, StatError> {
    let period = rules.period.hp_regenerate_period_ticks;
    if period == 0 {
        return Err(StatError::Overflow);
    }
    now.0
        .checked_add(u64::from(period))
        .map(Tick)
        .ok_or(StatError::Overflow)
}

/// Supported movement postures. Sitting and source zone bonuses are not in Phase 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum RegenPosture {
    /// Stationary at phase3 entry.
    Standing,
    /// Moving/running at phase3 entry, even when movement reaches its destination later.
    Moving,
}

/// Immutable profile inputs supplied from validated class/level or NPC templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct RegenInputs {
    pub player: bool,
    pub base_hp: ExactRatio,
    pub base_mp: ExactRatio,
    pub base_cp: ExactRatio,
    pub level_mod: ExactRatio,
    pub con_bonus: ExactRatio,
    pub men_bonus: ExactRatio,
    pub posture: RegenPosture,
}

/// Post-formula Q credit for one pulse. NPCs never receive CP regeneration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
#[allow(clippy::struct_field_names)] // Units must remain explicit at the resource boundary.
pub struct RegenAmounts {
    pub hp_q: i64,
    pub mp_q: i64,
    pub cp_q: Option<i64>,
}

/// `Formulas.java:263-450`: clamp initial>=1 before stat functions, then configuration and
/// player posture. NPCs skip player level/CON/MEN/posture factors, including the0.9 MP case.
pub fn regen_amounts(
    input: RegenInputs,
    calculator: &StatCalculator,
    conditions: FunctionConditions<'_>,
    rules: &RegenRules,
) -> Result<RegenAmounts, StatError> {
    let one = ExactRatio::whole(1);
    let movement = if input.player {
        ExactRatio::parse_literal(match input.posture {
            RegenPosture::Standing => &rules.movement_multipliers.standing,
            RegenPosture::Moving => &rules.movement_multipliers.moving,
        })?
    } else {
        one
    };
    let amount = |stat: &str,
                  base: ExactRatio,
                  bonus: ExactRatio,
                  multiplier: &str|
     -> Result<i64, StatError> {
        let initial = regen_initial(
            base,
            if input.player { input.level_mod } else { one },
            if input.player { bonus } else { one },
        )?;
        let folded = calculator.fold(stat, initial.to_q()?, conditions)?;
        let amount = ExactRatio::new(i128::from(folded), i128::from(super::Q))?
            .checked_mul(ExactRatio::parse_literal(multiplier)?)?
            .checked_mul(movement)?
            .to_q()?;
        if amount < 0 {
            return Err(StatError::Overflow);
        }
        Ok(amount)
    };
    Ok(RegenAmounts {
        hp_q: amount("regHp", input.base_hp, input.con_bonus, &rules.character_multipliers.hp)?,
        mp_q: amount("regMp", input.base_mp, input.men_bonus, &rules.character_multipliers.mp)?,
        cp_q: if input.player {
            Some(amount(
                "regCp",
                input.base_cp,
                input.con_bonus,
                &rules.character_multipliers.cp,
            )?)
        } else {
            None
        },
    })
}

/// Whole resource and current effective/recoverable maximum, captured after stat rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct RegenResource {
    pub current: u32,
    pub maximum: u32,
}

/// Complete immutable pulse result. Installing it and the staged next deadline is atomic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[allow(missing_docs)]
pub struct RegenRestoration {
    pub hp: u32,
    pub mp: u32,
    pub cp: Option<u32>,
    pub fractions: ResourceFractions,
    pub hp_granted: u32,
    pub mp_granted: u32,
    pub cp_granted: u32,
}

/// Living-only Q restoration, retaining fractions and preserving already-above-cap values.
/// Damage/MP drains use their separate whole-value path; this routine never truncates credit.
pub fn restore_regen(
    hp: RegenResource,
    mp: RegenResource,
    cp: Option<RegenResource>,
    fractions: ResourceFractions,
    amounts: RegenAmounts,
) -> Result<RegenRestoration, StatError> {
    fractions.validate().map_err(|_| StatError::Overflow)?;
    if (hp.current == 0 && fractions.hp == 0)
        || cp.is_some() != amounts.cp_q.is_some()
        || (cp.is_none() && fractions.cp != 0)
    {
        return Err(StatError::Overflow);
    }
    let restore = |resource: RegenResource, remainder: i64, amount: i64| {
        restore_resource(
            resource.current,
            resource.maximum,
            u32::try_from(remainder).map_err(|_| StatError::Overflow)?,
            amount,
        )
    };
    let hp = restore(hp, fractions.hp, amounts.hp_q)?;
    let mp = restore(mp, fractions.mp, amounts.mp_q)?;
    let cp = cp
        .zip(amounts.cp_q)
        .map(|(resource, amount)| restore(resource, fractions.cp, amount))
        .transpose()?;
    Ok(RegenRestoration {
        hp: hp.current,
        mp: mp.current,
        cp: cp.map(|result| result.current),
        fractions: ResourceFractions {
            hp: i64::from(hp.remainder_q),
            mp: i64::from(mp.remainder_q),
            cp: cp.map_or(0, |result| i64::from(result.remainder_q)),
        },
        hp_granted: hp.granted,
        mp_granted: mp.granted,
        cp_granted: cp.map_or(0, |result| result.granted),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::domain::zone::stat_functions::{FunctionKey, FunctionOp, StatFunction};
    use crate::domain::zone::Q;

    const CONDITIONS: FunctionConditions<'static> = FunctionConditions {
        weapon_kind: "SWORD",
        can_resurrect: false,
    };
    fn rules() -> super::super::Phase3Rules {
        crate::infrastructure::skill_data::load_phase3_rules(
            &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
            &[],
        )
        .unwrap()
    }
    fn input() -> RegenInputs {
        RegenInputs {
            player: false,
            base_hp: ExactRatio::whole(2),
            base_mp: ExactRatio::new(9, 10).unwrap(),
            base_cp: ExactRatio::whole(2),
            level_mod: ExactRatio::whole(2),
            con_bonus: ExactRatio::whole(2),
            men_bonus: ExactRatio::whole(2),
            posture: RegenPosture::Moving,
        }
    }
    #[test]
    fn living_schedule_death_and_revival_have_no_offline_credit() {
        let rules = rules();
        let mut schedule = RegenSchedule::start(Tick(100), true, rules.regen()).unwrap();
        assert_eq!(schedule.next_at, Some(Tick(130)));
        assert!(!schedule.take_due(Tick(129), rules.regen()).unwrap());
        assert!(schedule.take_due(Tick(130), rules.regen()).unwrap());
        assert_eq!(schedule.next_at, Some(Tick(160)));
        let restored: RegenSchedule =
            serde_json::from_slice(&serde_json::to_vec(&schedule).unwrap()).unwrap();
        assert_eq!(restored, schedule);
        schedule.stop();
        assert!(!schedule.take_due(Tick(2000), rules.regen()).unwrap());
        schedule = RegenSchedule::start(Tick(2000), true, rules.regen()).unwrap();
        assert_eq!(schedule.next_at, Some(Tick(2030)));
        let before = schedule;
        assert!(schedule.take_due(Tick(u64::MAX), rules.regen()).is_err());
        assert_eq!(schedule, before);
        assert_eq!(
            RegenSchedule::start(Tick(u64::MAX), false, rules.regen())
                .unwrap()
                .next_at,
            None
        );
    }

    #[test]
    fn npc_initial_minimum_precedes_functions_and_excludes_player_multipliers() {
        let rules = rules();
        let calculator = StatCalculator::try_new([StatFunction {
            key: FunctionKey {
                stat: "regMp".into(),
                priority: 20,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 1,
            },
            operation: FunctionOp::Mul(2 * Q),
            conditions: Vec::new(),
        }])
        .unwrap();
        let amounts = regen_amounts(input(), &calculator, CONDITIONS, rules.regen()).unwrap();
        assert_eq!(
            amounts,
            RegenAmounts {
                hp_q: 2 * Q,
                mp_q: 2 * Q,
                cp_q: None
            }
        );
        let player = RegenInputs {
            player: true,
            ..input()
        };
        assert_eq!(
            regen_amounts(player, &calculator, CONDITIONS, rules.regen()).unwrap(),
            RegenAmounts {
                hp_q: 5_600_000,
                mp_q: 5_040_000,
                cp_q: Some(5_600_000)
            }
        );
    }

    #[test]
    fn restoration_preserves_fractions_clears_them_at_cap_and_refuses_dead_hp() {
        let amounts = RegenAmounts {
            hp_q: 584_000,
            mp_q: 584_000,
            cp_q: Some(584_000),
        };
        let mut fractions = ResourceFractions::default();
        let mut current = 1;
        for expected in [(1, 584_000), (2, 168_000), (2, 752_000)] {
            let resource = RegenResource {
                current,
                maximum: 10,
            };
            let restored =
                restore_regen(resource, resource, Some(resource), fractions, amounts).unwrap();
            assert_eq!((restored.hp, restored.fractions.hp), expected);
            assert_eq!(restored.mp, restored.hp);
            assert_eq!(restored.cp, Some(restored.hp));
            current = restored.hp;
            fractions = restored.fractions;
        }
        let resource = RegenResource {
            current: 9,
            maximum: 10,
        };
        let capped = restore_regen(
            resource,
            resource,
            Some(resource),
            fractions,
            RegenAmounts {
                hp_q: 3 * Q,
                mp_q: 3 * Q,
                cp_q: Some(3 * Q),
            },
        )
        .unwrap();
        assert_eq!((capped.hp, capped.mp, capped.cp), (10, 10, Some(10)));
        assert_eq!(capped.fractions, ResourceFractions::default());
        let over = RegenResource {
            current: 12,
            maximum: 10,
        };
        let restored = restore_regen(over, over, Some(over), fractions, amounts).unwrap();
        assert_eq!((restored.hp, restored.mp, restored.cp), (12, 12, Some(12)));
        assert_eq!(restored.fractions, ResourceFractions::default());
        assert!(restore_regen(
            RegenResource {
                current: 0,
                maximum: 10
            },
            over,
            Some(over),
            ResourceFractions { hp: 0, ..fractions },
            amounts
        )
        .is_err());
    }

    #[test]
    fn fractional_zero_whole_hp_is_alive_and_regen_carries_the_remainder() {
        let restored = restore_regen(
            RegenResource {
                current: 0,
                maximum: 10,
            },
            RegenResource {
                current: 0,
                maximum: 10,
            },
            None,
            ResourceFractions {
                hp: Q / 2,
                ..Default::default()
            },
            RegenAmounts {
                hp_q: 3 * Q / 4,
                mp_q: 0,
                cp_q: None,
            },
        )
        .unwrap();
        assert_eq!((restored.hp, restored.fractions.hp, restored.hp_granted), (1, Q / 4, 1));
    }

    #[test]
    fn supported_regen_oracle_vectors_and_three_pulse_credit_match() {
        let rules = rules();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/skill-oracle.json"
        ))
        .unwrap();
        for family in ["calcHpRegen", "calcMpRegen", "calcCpRegen"] {
            for vector in fixture["formulas"][family].as_array().unwrap() {
                let p = &vector["inputs"];
                // Sitting is source-only oracle coverage (already checked in skill_math),
                // intentionally absent from the executable Phase 3 posture enum.
                if p["posture"] == "sitting" {
                    continue;
                }
                let ratio = |value: &serde_json::Value| {
                    ExactRatio::parse_literal(value.as_str().unwrap()).unwrap()
                };
                let base = ratio(&p["base_regen"]);
                let stat = ratio(&vector["intermediate"]["stat_bonus"]);
                let amounts = regen_amounts(
                    RegenInputs {
                        player: p["is_player"].as_bool().unwrap(),
                        base_hp: base,
                        base_mp: base,
                        base_cp: base,
                        level_mod: ratio(&vector["intermediate"]["level_mod"]),
                        con_bonus: stat,
                        men_bonus: stat,
                        posture: if p["posture"] == "moving" {
                            RegenPosture::Moving
                        } else {
                            RegenPosture::Standing
                        },
                    },
                    &StatCalculator::default(),
                    CONDITIONS,
                    rules.regen(),
                )
                .unwrap();
                let amount = match family {
                    "calcHpRegen" => amounts.hp_q,
                    "calcMpRegen" => amounts.mp_q,
                    _ => amounts.cp_q.unwrap(),
                };
                assert_eq!(amount, vector["q_result"].as_i64().unwrap(), "{vector}");
                let (mut current, mut fraction) = (0, 0);
                for pulse in vector["pulse_trace"].as_array().unwrap() {
                    let restored = restore_resource(current, u32::MAX, fraction, amount).unwrap();
                    assert_eq!(
                        u64::from(restored.current),
                        pulse["current"].as_u64().unwrap(),
                        "{vector}"
                    );
                    assert_eq!(
                        u64::from(restored.remainder_q),
                        pulse["remainder_q"].as_u64().unwrap(),
                        "{vector}"
                    );
                    current = restored.current;
                    fraction = restored.remainder_q;
                }
            }
        }
    }
}
