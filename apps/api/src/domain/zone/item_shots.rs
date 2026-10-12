//! Source charge/payment selection shared by manual use and automatic recharge.
use super::phase4::ShotKind;
use super::StatError;

/// Skill.java:700–705: physical attack effect and magic flag select consumed kinds.
pub(crate) fn skill_kinds(skill: &super::skill::SkillDef) -> (bool, bool) {
    (
        skill.is_magic == 1,
        skill
            .effects
            .iter()
            .any(|handler| matches!(handler.name.as_str(), "PhysicalAttack" | "FatalBlow")),
    )
}

/// Heal.java:68–104: charged mage/NPC bonus, otherwise the held grade coefficient.
pub(crate) fn heal_terms(
    mage: bool,
    npc: bool,
    grade: Option<&str>,
    shots: u8,
    mp_end: super::skill_math::ExactRatio,
    is_static: bool,
) -> Result<(u32, super::skill_math::ExactRatio), StatError> {
    use super::skill_math::ExactRatio;
    if is_static {
        return Ok((0, ExactRatio::whole(0)));
    }
    let blessed = shots & 4 != 0;
    if shots & 6 != 0 && (mage || npc) {
        return Ok((
            if blessed || npc { 4 } else { 2 },
            mp_end.checked_mul(if blessed || npc {
                ExactRatio::new(12, 5)?
            } else {
                ExactRatio::whole(1)
            })?,
        ));
    }
    let base: u32 = match grade {
        Some("S84") => 4,
        Some("S80") => 2,
        _ => 1,
    };
    let coefficient = if blessed {
        base.checked_mul(4)
    } else {
        base.checked_add(1)
    }
    .ok_or(StatError::Overflow)?;
    Ok((coefficient, ExactRatio::whole(0)))
}

/// Result of the source guards, before any inventory changes are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeOutcome {
    /// Missing usable weapon or zero configured shot count.
    Unavailable,
    /// Grade or default action does not match the source handler.
    WrongGrade,
    /// Same kind already charged; no payment or draw.
    AlreadyCharged,
    /// The selected source cost exceeds the selected stack.
    Insufficient,
    /// One immediate inventory payment charges the weapon.
    Charged,
}
impl ChargeOutcome {
    /// Frozen E6 oracle spelling.
    pub const fn evidence_name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::WrongGrade => "wrong_grade",
            Self::AlreadyCharged => "already_charged",
            Self::Insufficient => "insufficient",
            Self::Charged => "charged",
        }
    }
}

/// Captured source getters, not a second inventory or shot counter.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub struct ChargeTerms<'a> {
    pub kind: ShotKind,
    pub charges: u8,
    pub count: u32,
    pub available: u64,
    pub weapon_grade: Option<&'a str>,
    pub shot_grade: &'a str,
    pub correct_action: bool,
    pub reduction: &'a [u32],
}
/// Resolved charge cost; insufficient stock disables the configured auto template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ChargeResult {
    pub outcome: ChargeOutcome,
    pub consumed: u32,
    pub charges: u8,
    pub disable_auto: bool,
}
fn splus(grade: &str) -> &str {
    match grade {
        "S80" | "S84" => "S",
        _ => grade,
    }
}
impl ChargeTerms<'_> {
    /// SoulShots.java:61–98; SpiritShot.java:60–92; BlessedSpiritShot.java:63–95.
    pub fn resolve(
        self,
        mut draw: impl FnMut(u32) -> Result<u32, StatError>,
    ) -> Result<ChargeResult, StatError> {
        if self.charges & !7 != 0 || !matches!(self.reduction.len(), 0 | 2) {
            return Err(StatError::Overflow);
        }
        let unchanged = |outcome| ChargeResult {
            outcome,
            consumed: 0,
            charges: self.charges,
            disable_auto: outcome == ChargeOutcome::Insufficient,
        };
        let Some(weapon_grade) = self.weapon_grade.filter(|_| self.count > 0) else {
            return Ok(unchanged(ChargeOutcome::Unavailable));
        };
        let charged = self.charges & self.kind.bit() != 0;
        if self.kind != ShotKind::Soul && charged {
            return Ok(unchanged(ChargeOutcome::AlreadyCharged));
        }
        if !self.correct_action || splus(weapon_grade) != splus(self.shot_grade) {
            return Ok(unchanged(ChargeOutcome::WrongGrade));
        }
        if charged {
            return Ok(unchanged(ChargeOutcome::AlreadyCharged));
        }
        let mut count = self.count;
        if self.kind == ShotKind::Soul {
            if let [chance, reduced] = self.reduction {
                if *reduced > 0 {
                    let residue = draw(100)?;
                    if residue >= 100 {
                        return Err(StatError::DrawOutOfRange(i64::from(residue)));
                    }
                    if residue < *chance {
                        count = *reduced;
                    }
                }
            }
        }
        if self.available < u64::from(count) {
            return Ok(unchanged(ChargeOutcome::Insufficient));
        }
        Ok(ChargeResult {
            outcome: ChargeOutcome::Charged,
            consumed: count,
            charges: self.charges | self.kind.bit(),
            disable_auto: false,
        })
    }
}

/// Skill.java:1144–1157 and I2: one bundle, blessed before spirit before soul.
/// Cancellation and all-miss auto bundles preserve their prepaid charge.
pub fn discharge(charges: u8, used: bool, spirit: bool, soul: bool) -> Result<u8, StatError> {
    if charges & !7 != 0 {
        return Err(StatError::Overflow);
    }
    let bit = if !used {
        0
    } else if spirit {
        if charges & ShotKind::BlessedSpirit.bit() != 0 {
            ShotKind::BlessedSpirit.bit()
        } else {
            ShotKind::Spirit.bit()
        }
    } else if soul {
        ShotKind::Soul.bit()
    } else {
        0
    };
    Ok(charges & !bit)
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
    use serde_json::{json, Value};
    fn mask(names: &Value) -> u8 {
        names.as_array().unwrap().iter().fold(0, |mask, name| {
            mask | match name.as_str().unwrap() {
                "soul" => 1,
                "spirit" => 2,
                "blessed" => 4,
                _ => panic!("unknown kind"),
            }
        })
    }
    fn names(mask: u8) -> Vec<&'static str> {
        [(4, "blessed"), (1, "soul"), (2, "spirit")]
            .into_iter()
            .filter_map(|(bit, name)| (mask & bit != 0).then_some(name))
            .collect()
    }
    #[test]
    fn independent_shot_payment_vectors() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for vector in fixture["vectors"].as_array().unwrap() {
            let id = vector["id"].as_str().unwrap();
            if !id.starts_with("shot-payment-") {
                continue;
            }
            let input = &vector["inputs"];
            let number = |key| u32::try_from(input[key].as_u64().unwrap()).unwrap();
            let reduction = input.get("reduction").map_or_else(Vec::new, |r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|r| u32::try_from(r.as_u64().unwrap()).unwrap())
                    .collect()
            });
            let mut draws = Vec::new();
            let result = ChargeTerms {
                kind: match input["kind"].as_str().unwrap() {
                    "soul" => ShotKind::Soul,
                    "spirit" => ShotKind::Spirit,
                    "blessed" => ShotKind::BlessedSpirit,
                    _ => panic!("kind"),
                },
                charges: mask(&input["charges"]),
                count: number("count"),
                available: input["available"].as_u64().unwrap(),
                weapon_grade: input["weapon_grade"].as_str(),
                shot_grade: input["shot_grade"].as_str().unwrap(),
                correct_action: true,
                reduction: &reduction,
            }
            .resolve(|bound| {
                let residue = number("residue");
                draws.push(json!({"site":"soulshot_reduction","bound":bound,"residue":residue}));
                Ok(residue)
            })
            .unwrap();
            assert_eq!(
                json!({"charges":names(result.charges),"available":input["available"].as_u64().unwrap()-u64::from(result.consumed),
                "consumed":result.consumed,"toggle":input["toggle"].as_bool().unwrap() && !result.disable_auto,"draws":draws,"outcome":result.outcome.evidence_name()}),
                vector["expected"],
                "{id}"
            );
            checked += 1;
        }
        assert_eq!(checked, 36);
    }

    #[test]
    fn independent_heal_shot_terms_and_shared_bundle_vectors() {
        use super::super::skill_math::ExactRatio;
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/item-oracle.json"
        ))
        .unwrap();
        let mut checked = 0;
        for row in fixture["vectors"].as_array().unwrap() {
            let id = row["id"].as_str().unwrap();
            let p = &row["inputs"];
            let expected = &row["expected"];
            let flag = |key, default| p.get(key).and_then(Value::as_bool).unwrap_or(default);
            let number = |key, default| {
                p.get(key).map_or(ExactRatio::whole(default), |v| {
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
                })
            };
            if id.starts_with("heal-") {
                let bits = if flag("is_magic", true) {
                    (if flag("spirit", false) { 2 } else { 0 })
                        | (if flag("blessed", false) { 4 } else { 0 })
                } else {
                    0
                };
                let (coefficient, bonus) = heal_terms(
                    flag("mage", false),
                    flag("npc", false),
                    p.get("grade").and_then(Value::as_str),
                    bits,
                    number("mp_consume2", 0),
                    flag("static", false),
                )
                .unwrap();
                assert_eq!(
                    u64::from(coefficient),
                    expected["coefficient"]["numerator"].as_u64().unwrap(),
                    "{id}"
                );
                assert_eq!(
                    bonus.parts(),
                    (
                        i128::from(expected["static_bonus"]["numerator"].as_i64().unwrap()),
                        i128::from(expected["static_bonus"]["denominator"].as_i64().unwrap())
                    ),
                    "{id}"
                );
            } else if id.starts_with("shot-bundle-") {
                let frozen = mask(&p["charges"]);
                let hits = p["hits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|hit| hit.as_bool().unwrap() && !flag("canceled", false))
                    .count();
                let remaining = if flag("unequip", false) {
                    0
                } else {
                    discharge(frozen, hits > 0, flag("use_spirit", false), flag("use_soul", false))
                        .unwrap()
                };
                assert_eq!(
                    json!({"recipient_charges":vec![names(frozen);hits],"after":names(remaining),"payments":0}),
                    *expected,
                    "{id}"
                );
            } else {
                continue;
            }
            checked += 1;
        }
        assert_eq!(checked, 69);
    }
}
