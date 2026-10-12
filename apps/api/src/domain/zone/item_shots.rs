//! Source charge/payment selection shared by manual use and automatic recharge.
use super::phase4::ShotKind;
use super::StatError;

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
}
