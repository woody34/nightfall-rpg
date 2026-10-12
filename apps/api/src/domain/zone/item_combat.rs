//! V6 caller-specific source arithmetic and conditional draw order.
use super::phase4::ShieldOutcome;
use super::skill_math::ExactRatio;
use super::state_damage::DamageRollSite;
use super::{StatError, Q};

/// Victim-relative last-movement facing, with inclusive front/back boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proximity {
    /// At most sixty degrees from the victim's forward vector, including coincidence.
    Front,
    /// Outside the front and rear cones.
    Side,
    /// At most sixty degrees from the victim's reversed forward vector.
    Back,
}
impl Proximity {
    /// L2Character.java:4930–4993, with the accepted integer/facing adaptation.
    pub fn classify(
        facing: super::item_angles::FacingVector,
        dx: i64,
        dy: i64,
    ) -> Result<Self, StatError> {
        let (x, y) = facing.components();
        let (x, y, dx, dy) = (i128::from(x), i128::from(y), i128::from(dx), i128::from(dy));
        let dot = x
            .checked_mul(dx)
            .and_then(|n| y.checked_mul(dy).and_then(|m| n.checked_add(m)))
            .ok_or(StatError::Overflow)?;
        let norm = |a: i128, b: i128| {
            a.checked_mul(a)
                .and_then(|n| b.checked_mul(b).and_then(|m| n.checked_add(m)))
        };
        let denominator = norm(x, y)
            .and_then(|n| norm(dx, dy).and_then(|m| n.checked_mul(m)))
            .ok_or(StatError::Overflow)?;
        let numerator = dot
            .checked_mul(dot)
            .and_then(|n| n.checked_mul(4))
            .ok_or(StatError::Overflow)?;
        Ok(if numerator < denominator {
            Self::Side
        } else if dot < 0 {
            Self::Back
        } else {
            Self::Front
        })
    }
    /// Formulas.java:1624–1628; before the caller's `BLOW_RATE` fold.
    pub fn blow_rate(self, chance: ExactRatio, dex: ExactRatio) -> Result<ExactRatio, StatError> {
        chance.checked_mul(dex)?.checked_mul(match self {
            Self::Front => ExactRatio::whole(1),
            Self::Side => ExactRatio::new(3, 2)?,
            Self::Back => ExactRatio::whole(2),
        })
    }
    /// Formulas.java:516, source damage proximity distinct from success proximity.
    pub fn blow_damage(self) -> Result<ExactRatio, StatError> {
        ExactRatio::new(
            match self {
                Self::Front => 10,
                Self::Side => 11,
                Self::Back => 12,
            },
            10,
        )
    }
}

fn roll(bound: u32, value: u32) -> Result<ExactRatio, StatError> {
    if value >= bound {
        return Err(StatError::DrawOutOfRange(i64::from(value)));
    }
    Ok(ExactRatio::whole(i64::from(value)))
}

/// Formulas.java:1640 with D50 per-mille adaptation; rates are not rounded or clamped.
pub fn blow_lands(rate: ExactRatio, residue: u32) -> Result<bool, StatError> {
    Ok(rate
        .checked_mul(ExactRatio::whole(10))?
        .checked_cmp(roll(1000, residue)?)?
        .is_gt())
}

/// Full/half/none selection; the caller applies the source resource setters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LethalOutcome {
    /// No lethal branch selected, including eligibility skip.
    None,
    /// Full chance succeeded; half chance was not drawn.
    Full,
    /// Full chance failed and half chance succeeded.
    Half,
}

/// Lethal.java:60–96. Even zero probabilities draw when eligible; half draws only on
/// full failure. Eligibility is resolved before this helper, including invulnerability.
pub fn lethal(
    eligible: bool,
    full: ExactRatio,
    half: ExactRatio,
    multiplier: ExactRatio,
    mut draw: impl FnMut(DamageRollSite, u32) -> Result<u32, StatError>,
) -> Result<LethalOutcome, StatError> {
    if !eligible {
        return Ok(LethalOutcome::None);
    }
    let multiplier = multiplier.checked_mul(ExactRatio::whole(10))?;
    if full
        .checked_mul(multiplier)?
        .checked_cmp(roll(1000, draw(DamageRollSite::LethalFull, 1000)?)?)?
        .is_gt()
    {
        return Ok(LethalOutcome::Full);
    }
    Ok(
        if half
            .checked_mul(multiplier)?
            .checked_cmp(roll(1000, draw(DamageRollSite::LethalHalf, 1000)?)?)?
            .is_gt()
        {
            LethalOutcome::Half
        } else {
            LethalOutcome::None
        },
    )
}

/// Formulas.java:1077–1105; pinned character.properties:136–140 has perfect rate10.
/// Eligibility includes the certified angle; rate is the pre-bow DEX-adjusted getter.
pub fn shield(
    eligible: bool,
    rate: ExactRatio,
    bow: bool,
    mut draw: impl FnMut(DamageRollSite, u32) -> Result<u32, StatError>,
) -> Result<ShieldOutcome, StatError> {
    if !eligible
        || !rate
            .checked_cmp(ExactRatio::new(1, i128::from(Q))?)?
            .is_gt()
    {
        return Ok(ShieldOutcome::None);
    }
    let rate = if bow {
        rate.checked_mul(ExactRatio::new(13, 10)?)?
    } else {
        rate
    };
    if roll(100, draw(DamageRollSite::ShieldPerfect, 100)?)?
        .checked_cmp(ExactRatio::whole(90))?
        .is_gt()
    {
        return Ok(ShieldOutcome::Perfect);
    }
    Ok(
        if rate
            .checked_cmp(roll(100, draw(DamageRollSite::ShieldNormal, 100)?)?)?
            .is_gt()
        {
            ShieldOutcome::Normal
        } else {
            ShieldOutcome::None
        },
    )
}

/// Formulas.java:528–560. Inputs have already passed their source stat getters.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct BlowTerms {
    pub power: ExactRatio,
    pub attack: ExactRatio,
    pub defence: ExactRatio,
    pub critical: ExactRatio,
    pub critical_position: ExactRatio,
    pub critical_vulnerability: ExactRatio,
    pub critical_add: ExactRatio,
    pub critical_add_vulnerability: ExactRatio,
    pub proximity: Proximity,
    pub soulshot: bool,
    pub pvp_bonus: ExactRatio,
}
impl BlowTerms {
    /// Trait/attribute/spread/penalty and max(1) follow this expression at the live caller.
    pub fn damage(self) -> Result<ExactRatio, StatError> {
        let one = ExactRatio::whole(1);
        let base = self
            .power
            .checked_add(self.attack.checked_mul(if self.soulshot {
                ExactRatio::new(1458, 1000)?
            } else {
                one
            })?)?
            .checked_mul(ExactRatio::whole(77))?
            .checked_div(self.defence)?;
        let position = self
            .critical_position
            .checked_sub(one)?
            .checked_div(ExactRatio::whole(2))?
            .checked_add(one)?;
        let additive = self
            .critical_add
            .checked_mul(ExactRatio::new(61, 10)?)?
            .checked_mul(ExactRatio::whole(77))?
            .checked_div(self.defence)?;
        base.checked_mul(self.critical)?
            .checked_mul(position)?
            .checked_mul(self.critical_vulnerability)?
            .checked_mul(self.proximity.blow_damage()?)?
            .checked_mul(self.pvp_bonus)?
            .checked_add(additive)?
            .checked_add(self.critical_add_vulnerability)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm
)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    fn ratio(v: &Value) -> ExactRatio {
        if let Some(n) = v.as_i64() {
            ExactRatio::whole(n)
        } else if let Some(s) = v.as_str() {
            ExactRatio::parse_literal(s).unwrap()
        } else {
            ExactRatio::new(
                i128::from(v["numerator"].as_i64().unwrap()),
                i128::from(v["denominator"].as_i64().unwrap()),
            )
            .unwrap()
        }
    }
    fn number(input: &Value, key: &str, default: i64) -> ExactRatio {
        input.get(key).map_or(ExactRatio::whole(default), ratio)
    }
    fn proximity(text: &str) -> Proximity {
        match text {
            "front" => Proximity::Front,
            "side" => Proximity::Side,
            "back" => Proximity::Back,
            _ => panic!("unknown source proximity"),
        }
    }
    fn label(value: Proximity) -> &'static str {
        match value {
            Proximity::Front => "front",
            Proximity::Side => "side",
            Proximity::Back => "back",
        }
    }
    fn source_draws<'a>(
        input: &'a Value,
        trace: &'a mut Vec<Value>,
    ) -> impl FnMut(DamageRollSite, u32) -> Result<u32, StatError> + 'a {
        move |site, bound| {
            let (key, name) = match site {
                DamageRollSite::ShieldPerfect => ("perfect_residue", "shield_perfect"),
                DamageRollSite::ShieldNormal => ("normal_residue", "shield_normal"),
                DamageRollSite::LethalFull => ("full_residue", "lethal_full"),
                DamageRollSite::LethalHalf => ("half_residue", "lethal_half"),
                _ => panic!("unexpected draw {site:?}"),
            };
            let value = u32::try_from(input[key].as_u64().unwrap()).unwrap();
            trace.push(json!({"site":name,"bound":bound,"residue":value}));
            Ok(value)
        }
    }
    #[test]
    #[allow(clippy::too_many_lines)]
    fn independent_blow_lethal_position_and_shield_vectors() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for v in fixture["vectors"].as_array().unwrap() {
            let id = v["id"].as_str().unwrap();
            let input = &v["inputs"];
            let expected = &v["expected"];
            let flag = |key, default| input.get(key).and_then(Value::as_bool).unwrap_or(default);
            if id.starts_with("position-") {
                let facing =
                    serde_json::from_value(json!({"x":input["facing"][0],"y":input["facing"][1]}))
                        .unwrap();
                assert_eq!(
                    json!(label(
                        Proximity::classify(
                            facing,
                            input["relative"][0].as_i64().unwrap(),
                            input["relative"][1].as_i64().unwrap()
                        )
                        .unwrap()
                    )),
                    *expected,
                    "{id}"
                );
            } else if id.starts_with("blow-") && input.get("chance").is_some() {
                let rate = proximity(input["proximity"].as_str().unwrap_or("front"))
                    .blow_rate(number(input, "chance", 0), number(input, "dex", 1))
                    .unwrap();
                assert_eq!(
                    json!(blow_lands(
                        rate.checked_mul(number(input, "modifier", 1)).unwrap(),
                        u32::try_from(input["residue"].as_u64().unwrap()).unwrap()
                    )
                    .unwrap()),
                    *expected,
                    "{id}"
                );
            } else if id.starts_with("shield-") {
                let mut trace = Vec::new();
                let outcome = shield(
                    flag("eligible", true) && flag("facing", true) && !flag("sigil", false),
                    number(input, "rate", 0)
                        .checked_mul(number(input, "dex_bonus", 1))
                        .unwrap(),
                    flag("bow", false),
                    source_draws(input, &mut trace),
                )
                .unwrap();
                let name = match outcome {
                    ShieldOutcome::None => "none",
                    ShieldOutcome::Normal => "normal",
                    ShieldOutcome::Perfect => "perfect",
                };
                assert_eq!(json!({"outcome":name,"draws":trace}), *expected, "{id}");
            } else if id.starts_with("lethal-") {
                let mut trace = Vec::new();
                let outcome = lethal(
                    flag("eligible", true),
                    number(input, "full", 0),
                    number(input, "half", 0),
                    number(input, "attribute", 1)
                        .checked_mul(number(input, "trait", 1))
                        .unwrap(),
                    source_draws(input, &mut trace),
                )
                .unwrap();
                let name = match outcome {
                    LethalOutcome::None => "none",
                    LethalOutcome::Full => "full",
                    LethalOutcome::Half => "half",
                };
                assert_eq!(json!({"outcome":name,"draws":trace}), *expected, "{id}");
            } else if input["caller"] == "blow" {
                let damage = if flag("perfect", false) {
                    ExactRatio::whole(1)
                } else {
                    BlowTerms {
                        power: number(input, "power", 0),
                        attack: number(input, "p_atk", 0),
                        defence: number(input, "defence", 0)
                            .checked_add(number(input, "shield_def", 0))
                            .unwrap(),
                        critical: number(input, "crit_dmg", 1),
                        critical_position: number(input, "crit_pos", 1),
                        critical_vulnerability: number(input, "def_crit_dmg", 1),
                        critical_add: number(input, "crit_add", 0),
                        critical_add_vulnerability: number(input, "def_crit_add", 0),
                        proximity: proximity(input["proximity"].as_str().unwrap_or("front")),
                        soulshot: flag("shot", false),
                        pvp_bonus: number(input, "pvp", 1),
                    }
                    .damage()
                    .unwrap()
                    .checked_mul(number(input, "trait", 1))
                    .unwrap()
                    .checked_mul(number(input, "attribute", 1))
                    .unwrap()
                    .checked_mul(number(input, "spread", 1))
                    .unwrap()
                    .checked_mul(number(input, "npc", 1))
                    .unwrap()
                };
                let damage = if damage.checked_cmp(ExactRatio::whole(1)).unwrap().is_lt() {
                    ExactRatio::whole(1)
                } else {
                    damage
                };
                if let Some(formula) = expected.get("formula") {
                    assert_eq!(damage, ratio(formula), "{id}");
                    let final_damage = damage
                        .checked_mul(ExactRatio::whole(if flag("critical", false) { 2 } else { 1 }))
                        .unwrap();
                    assert_eq!(final_damage, ratio(&expected["after_handler_critical"]), "{id}");
                    assert_eq!(
                        final_damage.trunc().unwrap(),
                        i128::from(expected["getter"].as_i64().unwrap()),
                        "{id}"
                    );
                } else {
                    assert_eq!(damage, ratio(expected), "{id}");
                }
            } else {
                continue;
            }
            checked += 1;
        }
        assert_eq!(checked, 73);
    }
    #[test]
    fn lethal_eligibility_and_zero_chance_draw_order_and_invalid_residues() {
        let zero = ExactRatio::whole(0);
        let one = ExactRatio::whole(1);
        assert_eq!(
            lethal(false, one, one, one, |_, _| panic!("ineligible lethal draw")).unwrap(),
            LethalOutcome::None
        );
        let mut trace = Vec::new();
        assert_eq!(
            lethal(true, zero, zero, one, |site, bound| {
                trace.push((site, bound));
                Ok(0)
            })
            .unwrap(),
            LethalOutcome::None
        );
        assert_eq!(
            trace,
            [
                (DamageRollSite::LethalFull, 1000),
                (DamageRollSite::LethalHalf, 1000)
            ]
        );
        assert!(blow_lands(one, 1000).is_err());
        assert!(shield(true, one, false, |_, _| Ok(100)).is_err());
        assert!(lethal(true, one, one, one, |_, _| Ok(1000)).is_err());
    }
}
