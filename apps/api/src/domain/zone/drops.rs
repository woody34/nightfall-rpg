//! Monster drop resolution (design §3.1–§3.4): one pass over an NPC's `<death>` or `<corpse>`
//! list in document order with the pinned binary64 chance chain, the E15 exact chance test
//! and the §2.3 draw order. Pure over the rules and a draw source, so the zone and the
//! oracle-vector tests run the same code.
//!
//! Pinned sources (`l2j-server-game` abfde049): `IDropCalculationStrategy.DEFAULT_STRATEGY`
//! (`model/drops/strategy/IDropCalculationStrategy.java:31-47`),
//! `IGroupedItemDropCalculationStrategy.DEFAULT_STRATEGY` (`:48-86`), `GroupedGeneralDropItem.
//! normalizeMe` (`model/drops/GroupedGeneralDropItem.java:193-215`), `GeneralDropItem.getChance/
//! getMin/getMax` (`model/drops/GeneralDropItem.java:138-186`), `IKillerChanceModifierStrategy`
//! (`model/drops/strategy/IKillerChanceModifierStrategy.java:33-53`) and `DropListScope`
//! (`model/drops/DropListScope.java:31-32`).

use super::binary64::{Binary64, Binary64Error, Rational};
use super::phase5_rules::{DropNode, DropNpc, DropScope, Phase5Rules};
use super::wide::{chance_binary64, WideError};

/// The draw primitives of design §2.3 the resolver consumes.
pub trait DropDraws {
    /// `draw_u64()`: two words, high first.
    fn draw_pair(&mut self) -> u64;
    /// `roll_below(n)`: the unbiased rejection draw over `next_u32`.
    fn draw_below(&mut self, n: u32) -> u32;
}

/// A fail-closed resolution error (an invariant: validated rules never produce one).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DropError {
    /// The binary64 emulation left its domain.
    #[error(transparent)]
    Binary64(#[from] Binary64Error),
    /// A chance product exceeded 256 bits.
    #[error(transparent)]
    Wide(#[from] WideError),
    /// A rule the resolver needs is missing or out of range.
    #[error("drop rules: {0}")]
    Rules(&'static str),
}

/// The victim facts the strategies read.
#[derive(Debug, Clone, Copy)]
pub struct DropVictim {
    /// `isRaid()` from the pinned row.
    pub raid: bool,
    /// `getLevel()`.
    pub level: i64,
}

fn mul(a: Binary64, b: Binary64) -> Result<Binary64, DropError> {
    Ok(a.rational()?.mul(&b.rational()?).fl()?)
}

fn div(a: Binary64, b: Binary64) -> Result<Binary64, DropError> {
    Ok(a.rational()?.div(&b.rational()?)?.fl()?)
}

fn add(a: Binary64, b: Binary64) -> Result<Binary64, DropError> {
    Ok(a.rational()?.add(&b.rational()?).fl()?)
}

fn integer(value: i64) -> Result<Binary64, DropError> {
    Ok(Rational::integer(value).fl()?)
}

/// `chance > nextDouble() · 100` under E15: exactly one `draw_u64`, always taken (P3).
fn chance(draws: &mut impl DropDraws, c: Binary64) -> Result<bool, DropError> {
    let draw = draws.draw_pair();
    Ok(chance_binary64(c, draw)?)
}

/// The killer modifier `L` (design §3.3) read from the validated, frozen L table (E36).
fn killer_modifier(
    rules: &Phase5Rules,
    grouped: bool,
    template: u32,
    victim: DropVictim,
    killer_level: i64,
) -> Result<Binary64, DropError> {
    let d = victim.level.saturating_sub(killer_level);
    let deep = &rules.parts().rates.deep_blue;
    let strategy = if grouped {
        // IKillerChanceModifierStrategy.DEFAULT_STRATEGY: raid clamp, else the item window.
        if victim.raid && deep.use_deep_blue_drop_rules_raid {
            "raid_clamp"
        } else if deep.use_deep_blue_drop_rules {
            "item_window"
        } else {
            return Ok(Binary64::ONE);
        }
    } else if rules.deep_blue(victim.raid) {
        // DEFAULT_NONGROUP_STRATEGY: the adena window for adena, else the item window.
        if template == 57 {
            "adena_window"
        } else {
            "item_window"
        }
    } else {
        return Ok(Binary64::ONE);
    };
    rules
        .killer_modifier(strategy, d)
        .ok_or(DropError::Rules("killer modifier outside the L table"))
}

/// `(long) (n × a)` (`GeneralDropItem.getMin/getMax(victim)`): `⌊fl(n × a)⌋`.
fn scaled(n: u64, a: Binary64) -> Result<u64, DropError> {
    let n = i64::try_from(n).map_err(|_| DropError::Rules("drop count beyond i64"))?;
    Ok(mul(integer(n)?, a)?.rational()?.floor_u64()?)
}

/// `Rnd.get(min', max')` (l2j-server-commons 2.6.10.0, `RandomContainer.get(long, long)`:
/// `min + (long) (nextDouble() * ((max - min) + 1))`, no bounds check), drawn with
/// `roll_below` per design §2.3 and always taken, even at `min = max` (P3).
///
/// P5-Q7: pinned rows with `min > max` exist (e.g. npc 29181 adena 9,000,000..1,100,000). There
/// `k = max − min + 1 ≤ 0`, `nextDouble() · k ∈ (k, 0]` truncates toward zero, so the pinned
/// result is uniform over `{max + 2, …, min}` — always `min` when `k ∈ {0, −1}`. The engine
/// draws `roll_below(max(1, min − max − 1))` and subtracts; nothing is clamped or swapped.
fn quantity(draws: &mut impl DropDraws, min: u64, max: u64) -> Result<u64, DropError> {
    if max >= min {
        let span = max
            .checked_sub(min)
            .and_then(|s| s.checked_add(1))
            .and_then(|s| u32::try_from(s).ok())
            .ok_or(DropError::Rules("drop quantity span beyond u32"))?;
        let offset = u64::from(draws.draw_below(span));
        min.checked_add(offset)
            .ok_or(DropError::Rules("drop quantity overflow"))
    } else {
        let span = min
            .checked_sub(max)
            .and_then(|s| s.checked_sub(1))
            .unwrap_or(0)
            .max(1);
        let span =
            u32::try_from(span).map_err(|_| DropError::Rules("drop quantity span beyond u32"))?;
        let offset = u64::from(draws.draw_below(span));
        min.checked_sub(offset)
            .ok_or(DropError::Rules("drop quantity underflow"))
    }
}

/// The precise amount multiplier (`isPreciseCalculated()` is always true, `PreciseDrop
/// Calculation = True`): over 100 (strict for items, non-strict for groups) the multiplier is
/// `(int) c / 100`, plus one if `(c % 100) > nextDouble() · 100`; the remainder draw is taken
/// even when it is 0.
fn precise_multiplier(
    draws: &mut impl DropDraws,
    c: Binary64,
    strict: bool,
) -> Result<u64, DropError> {
    let value = c.rational()?;
    let hundred = Rational::integer(100);
    let over = if strict {
        value.compare(&hundred).is_gt()
    } else {
        value.compare(&hundred).is_ge()
    };
    if !over {
        return Ok(1);
    }
    let whole = value.floor_u64()?;
    let tens = whole / 100;
    let base = i64::try_from(tens.saturating_mul(100)).map_err(|_| DropError::Rules("chance"))?;
    // Java `%` on doubles is the exact IEEE remainder c − 100·trunc(c / 100).
    let remainder = value.sub(&Rational::integer(base)).fl()?;
    let extra = u64::from(chance(draws, remainder)?);
    Ok(tens.saturating_add(extra))
}

/// One ungrouped item (or a collapsed single-item group) with its final chance `c`.
#[allow(clippy::too_many_arguments)] // The pinned strategy's inputs, explicit.
fn roll_item(
    rules: &Phase5Rules,
    scope: DropScope,
    victim: DropVictim,
    template: u32,
    range: (u64, u64),
    c: Binary64,
    draws: &mut impl DropDraws,
) -> Result<Option<(u32, u64)>, DropError> {
    if !chance(draws, c)? {
        return Ok(None);
    }
    let mult = precise_multiplier(draws, c, true)?;
    let a = rules
        .amount_multiplier(scope, template, victim.raid)
        .ok_or(DropError::Rules("amount multiplier"))?;
    let q = quantity(draws, scaled(range.0, a)?, scaled(range.1, a)?)?;
    let count = q
        .checked_mul(mult)
        .ok_or(DropError::Rules("drop count overflow"))?;
    Ok(Some((template, count)))
}

/// Roll one drop list (design §3.2) and return its holders `(template, count)` in list order.
pub fn roll_list(
    rules: &Phase5Rules,
    npc: &DropNpc,
    scope: DropScope,
    victim: DropVictim,
    killer_level: i64,
    draws: &mut impl DropDraws,
) -> Result<Vec<(u32, u64)>, DropError> {
    let nodes = match scope {
        DropScope::Death => &npc.death,
        DropScope::Corpse => &npc.corpse,
    };
    let hundred = integer(100)?;
    let mut holders = Vec::new();
    for node in nodes {
        match node {
            DropNode::Item {
                id,
                min,
                max,
                chance_bits,
                ..
            } => {
                let m = rules
                    .chance_multiplier(scope, *id, victim.raid)
                    .ok_or(DropError::Rules("chance multiplier"))?;
                let l = killer_modifier(rules, false, *id, victim, killer_level)?;
                let cd = Binary64::parse_hex(chance_bits)?;
                // getChance(victim, killer) = killerMod * (getChance() * multiplier)
                let c = mul(l, mul(cd, m)?)?;
                holders.extend(roll_item(rules, scope, victim, *id, (*min, *max), c, draws)?);
            },
            DropNode::Group {
                chance_bits, items, ..
            } => {
                let group = Binary64::parse_hex(chance_bits)?;
                if let [item] = items.as_slice() {
                    // getSingleItem: chance (item × group) / 100, the item's rate strategies,
                    // the GROUP's killer strategy.
                    let cd = Binary64::parse_hex(&item.chance_bits)?;
                    let collapsed = div(mul(cd, group)?, hundred)?;
                    let m = rules
                        .chance_multiplier(scope, item.id, victim.raid)
                        .ok_or(DropError::Rules("chance multiplier"))?;
                    let l = killer_modifier(rules, true, item.id, victim, killer_level)?;
                    let c = mul(l, mul(collapsed, m)?)?;
                    holders.extend(roll_item(
                        rules,
                        scope,
                        victim,
                        item.id,
                        (item.min, item.max),
                        c,
                        draws,
                    )?);
                    continue;
                }
                holders.extend(roll_group(
                    rules,
                    scope,
                    victim,
                    killer_level,
                    group,
                    items,
                    draws,
                )?);
            },
        }
    }
    Ok(holders)
}

/// A multi-item group (normalizeMe, then the DEFAULT grouped strategy).
#[allow(clippy::too_many_arguments)] // The pinned strategy's inputs, explicit.
fn roll_group(
    rules: &Phase5Rules,
    scope: DropScope,
    victim: DropVictim,
    killer_level: i64,
    group: Binary64,
    items: &[super::phase5_rules::DropItemRow],
    draws: &mut impl DropDraws,
) -> Result<Option<(u32, u64)>, DropError> {
    let hundred = integer(100)?;
    let first = items.first().ok_or(DropError::Rules("empty group"))?;
    let l = killer_modifier(rules, true, first.id, victim, killer_level)?;
    // (item.getChance(victim) * getChance() * chanceModifier), left to right.
    let mut products = Vec::with_capacity(items.len());
    let mut sum = Binary64::ZERO;
    for item in items {
        let m = rules
            .chance_multiplier(scope, item.id, victim.raid)
            .ok_or(DropError::Rules("chance multiplier"))?;
        let w = mul(Binary64::parse_hex(&item.chance_bits)?, m)?;
        let product = mul(mul(w, group)?, l)?;
        sum = add(sum, div(product, hundred)?)?;
        products.push(product);
    }
    if sum == Binary64::ZERO {
        // Taken and failed: `0 > nextDouble() · 100` is false; no division happens.
        chance(draws, Binary64::ZERO)?;
        return Ok(None);
    }
    if !chance(draws, sum)? {
        return Ok(None);
    }
    let selection = draws.draw_pair();
    let mut total = Binary64::ZERO;
    let mut picked = None;
    for (item, product) in items.iter().zip(&products) {
        total = add(total, div(*product, sum)?)?;
        // `totalChance > random`, i.e. selection · 100 < T · 2^64 exactly.
        if chance_binary64(total, selection)? {
            picked = Some(item);
            break;
        }
    }
    let Some(item) = picked else {
        // The double total fell short of 100: pinned falls off the end (E30).
        return Ok(None);
    };
    let mult = precise_multiplier(draws, sum, false)?;
    let a = rules
        .amount_multiplier(scope, item.id, victim.raid)
        .ok_or(DropError::Rules("amount multiplier"))?;
    let q = quantity(draws, scaled(item.min, a)?, scaled(item.max, a)?)?;
    let count = q
        .checked_mul(mult)
        .ok_or(DropError::Rules("drop count overflow"))?;
    Ok(Some((item.id, count)))
}

/// The final chance `S` of every multi-item group at `L` (evidence and the D-E fidelity
/// units): `(S, final walk total)`.
pub fn group_totals(
    rules: &Phase5Rules,
    scope: DropScope,
    victim: DropVictim,
    killer_level: i64,
    group: Binary64,
    items: &[super::phase5_rules::DropItemRow],
) -> Result<(Binary64, Binary64), DropError> {
    let hundred = integer(100)?;
    let first = items.first().ok_or(DropError::Rules("empty group"))?;
    let l = killer_modifier(rules, true, first.id, victim, killer_level)?;
    let mut products = Vec::with_capacity(items.len());
    let mut sum = Binary64::ZERO;
    for item in items {
        let m = rules
            .chance_multiplier(scope, item.id, victim.raid)
            .ok_or(DropError::Rules("chance multiplier"))?;
        let product = mul(mul(mul(Binary64::parse_hex(&item.chance_bits)?, m)?, group)?, l)?;
        sum = add(sum, div(product, hundred)?)?;
        products.push(product);
    }
    let mut total = Binary64::ZERO;
    if sum != Binary64::ZERO {
        for product in products {
            total = add(total, div(product, sum)?)?;
        }
    }
    Ok((sum, total))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines
)]
mod tests {
    use super::*;

    /// A scripted word stream with the engine's exact `roll_below` rejection.
    pub(crate) struct Words {
        words: std::vec::IntoIter<u32>,
    }

    impl Words {
        pub(crate) fn new(words: Vec<u32>) -> Self {
            Self {
                words: words.into_iter(),
            }
        }

        fn next(&mut self) -> u32 {
            self.words.next().unwrap()
        }

        pub(crate) fn left(&self) -> usize {
            self.words.len()
        }
    }

    impl DropDraws for Words {
        fn draw_pair(&mut self) -> u64 {
            let hi = self.next();
            let lo = self.next();
            (u64::from(hi) << 32) | u64::from(lo)
        }

        fn draw_below(&mut self, n: u32) -> u32 {
            let n = u64::from(n.max(1));
            let span = 1_u64 << 32;
            let limit = span - span % n;
            loop {
                let v = u64::from(self.next());
                if v < limit {
                    return u32::try_from(v % n).unwrap();
                }
            }
        }
    }

    fn rules() -> std::sync::Arc<Phase5Rules> {
        crate::infrastructure::economy_data::embedded_phase5_rules().unwrap()
    }

    fn oracle() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-drops.json"
        ))
        .unwrap()
    }

    fn pairs(value: &serde_json::Value) -> Vec<(u32, u64)> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| {
                (u32::try_from(pair[0].as_u64().unwrap()).unwrap(), pair[1].as_u64().unwrap())
            })
            .collect()
    }

    /// Family D-O on actual inputs: replay each oracle whole-death trace's word stream.
    #[test]
    fn whole_death_traces_reproduce_the_oracle_draw_for_draw() {
        let rules = rules();
        let traces = oracle()["do_traces"].as_array().unwrap().clone();
        assert!(traces.len() >= 5);
        for trace in &traces {
            let npc = rules
                .drop_npc(u32::try_from(trace["npc"].as_u64().unwrap()).unwrap())
                .unwrap()
                .clone();
            let words: Vec<u32> = trace["words"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| u32::try_from(w.as_u64().unwrap()).unwrap())
                .collect();
            let mut draws = Words::new(words);
            let victim = DropVictim {
                raid: npc.is_raid,
                level: i64::from(npc.level),
            };
            let killer = trace["killer_level"].as_i64().unwrap();
            let sweep = if trace["spoiled"].as_bool().unwrap() {
                roll_list(&rules, &npc, DropScope::Corpse, victim, killer, &mut draws).unwrap()
            } else {
                Vec::new()
            };
            let holders =
                roll_list(&rules, &npc, DropScope::Death, victim, killer, &mut draws).unwrap();
            assert_eq!(sweep, pairs(&trace["sweep"]), "npc {} sweep", npc.id);
            assert_eq!(holders, pairs(&trace["holders"]), "npc {} holders", npc.id);
            // Positions follow in holder order: dx then dy, roll_below(141) − 70 each.
            for site in trace["trace"].as_array().unwrap() {
                if site["draw"] == "position" {
                    let dx = i64::from(draws.draw_below(141)) - 70;
                    let dy = i64::from(draws.draw_below(141)) - 70;
                    assert_eq!(
                        (dx, dy),
                        (site["dx"].as_i64().unwrap(), site["dy"].as_i64().unwrap())
                    );
                }
            }
            if trace["respawn_random"].as_u64().unwrap() == 0 {
                assert_eq!(draws.left(), 0, "npc {}: every logged word consumed", npc.id);
            }
        }
    }

    /// Family D-E (E30) at L = 1: every multi-item death group's binary64 `S` and walk total.
    #[test]
    fn group_chances_match_the_oracle_binary64_fidelity_summary() {
        let rules = rules();
        let summary = &oracle()["de_summary"];
        let mut groups = 0_u64;
        let mut falloff = 0_u64;
        let mut exact_100 = 0_u64;
        let mut below = std::collections::BTreeSet::new();
        let mut above = std::collections::BTreeSet::new();
        let hundred = integer(100).unwrap();
        for npc in &rules.parts().drops.npc {
            let victim = DropVictim {
                raid: npc.is_raid,
                level: i64::from(npc.level),
            };
            for node in &npc.death {
                let DropNode::Group {
                    chance_bits, items, ..
                } = node
                else {
                    continue;
                };
                if items.len() < 2 {
                    continue;
                }
                groups += 1;
                let group = Binary64::parse_hex(chance_bits).unwrap();
                let (sum, total) =
                    group_totals(&rules, DropScope::Death, victim, victim.level, group, items)
                        .unwrap();
                if total
                    .rational()
                    .unwrap()
                    .compare(&hundred.rational().unwrap())
                    .is_lt()
                {
                    falloff += 1;
                }
                // The exact sum over the DECIMAL strings (the oracle's definition), against the
                // binary64 S the engine computes.
                let DropNode::Group { chance, .. } = node else {
                    continue;
                };
                let group_decimal = Rational::parse_decimal(chance).unwrap();
                let mut exact = Rational::integer(0);
                for item in items {
                    let cd = Rational::parse_decimal(&item.chance).unwrap();
                    exact =
                        exact.add(&cd.mul(&group_decimal).div(&Rational::integer(100)).unwrap());
                }
                if exact.compare(&Rational::integer(100)).is_eq() {
                    exact_100 += 1;
                    match sum.rational().unwrap().compare(&Rational::integer(100)) {
                        std::cmp::Ordering::Less => {
                            below.insert(npc.id);
                        },
                        std::cmp::Ordering::Greater => {
                            above.insert(npc.id);
                        },
                        std::cmp::Ordering::Equal => {},
                    }
                }
            }
        }
        let ids = |key: &str| -> std::collections::BTreeSet<u32> {
            summary[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
                .collect()
        };
        assert_eq!(groups, summary["groups"].as_u64().unwrap());
        assert_eq!(falloff, summary["falloff_L1"].as_u64().unwrap());
        assert_eq!(exact_100, summary["exact_100"].as_u64().unwrap());
        assert_eq!(below, ids("below_100_npcs"));
        assert_eq!(above, ids("above_100_npcs"));
        // Design §14.1 D-E literals (z-review-2 P2-5), so oracle drift cannot pass silently.
        assert_eq!(groups, 11_557);
        assert_eq!(falloff, 1_127);
        assert_eq!(exact_100, 121);
        assert_eq!(below, [29068, 29150, 29186].into());
        assert_eq!(above, [29028, 29068, 29118, 29163].into());
    }

    /// Bits of the widest side of the E15 test `r · 100 · 2^k < num · 2^64` (`r < 2^64`)
    /// for a binary64 chance `num / 2^k`: `(reduced, as the engine builds it)`. The oracle
    /// measures the reduced fraction; [`dyadic_chance`] keeps the unreduced mantissa.
    fn chance_width(c: Binary64) -> (u32, u32) {
        let (num, k) = crate::domain::zone::wide::dyadic_chance(c).unwrap();
        if num == crate::domain::zone::wide::U256::ZERO {
            return (0, 0);
        }
        let side = |num_bits: u32, k: u32| (71 + k).max(num_bits + 64);
        let bits = num.bits();
        // With k > 0 the numerator is the bare 53-bit mantissa, so limb 0 holds it.
        let trailing = if k == 0 {
            0
        } else {
            num.limbs()[0].trailing_zeros().min(k)
        };
        (side(bits - trailing, k - trailing), side(bits, k))
    }

    /// Family D-W (design §3.2 / §14.1, z-review-2 P2-5): the widest U256 chance-test
    /// operand over every row and group of both scopes × the 19 distinct L values × G in
    /// {1, 2^-32, 2^32}, folded as the oracle does (every rate 1, so `m = fl32(fl(G))`).
    /// It equals the oracle widths and stays at most 255 bits. Identical inputs are
    /// measured once (the width is a maximum).
    #[test]
    fn chance_widths_match_the_oracle_and_fit_u256() {
        use std::collections::BTreeSet;
        let rules = rules();
        let widths = &oracle()["dw_widths"];
        let mut ls = BTreeSet::new();
        for strategy in ["item_window", "raid_clamp", "adena_window"] {
            for d in -20..=2 {
                ls.insert(rules.killer_modifier(strategy, d).unwrap().0);
            }
        }
        assert_eq!(ls.len(), 19);
        let ls: Vec<Binary64> = ls.into_iter().map(Binary64).collect();
        let mut items = BTreeSet::new();
        let mut singles = BTreeSet::new();
        let mut groups = BTreeSet::new();
        for npc in &rules.parts().drops.npc {
            for node in npc.death.iter().chain(&npc.corpse) {
                match node {
                    DropNode::Item { chance_bits, .. } => {
                        items.insert(Binary64::parse_hex(chance_bits).unwrap().0);
                    },
                    DropNode::Group {
                        chance_bits,
                        items: rows,
                        ..
                    } => {
                        let group = Binary64::parse_hex(chance_bits).unwrap().0;
                        let rows: Vec<u64> = rows
                            .iter()
                            .map(|row| Binary64::parse_hex(&row.chance_bits).unwrap().0)
                            .collect();
                        if let [row] = rows.as_slice() {
                            singles.insert((*row, group));
                        } else {
                            groups.insert((group, rows));
                        }
                    },
                }
            }
        }
        let hundred = integer(100).unwrap();
        let remainder = |c: Binary64| {
            let value = c.rational().unwrap();
            let base = i64::try_from(value.floor_u64().unwrap() / 100 * 100).unwrap();
            value.sub(&Rational::integer(base)).fl().unwrap()
        };
        for (label, g) in [
            ("G=1", Binary64::ONE),
            (
                "G=2^-32",
                Rational::integer(1)
                    .div(&Rational::integer(1 << 32))
                    .unwrap()
                    .fl()
                    .unwrap(),
            ),
            ("G=2^32", integer(1 << 32).unwrap()),
        ] {
            let m = crate::domain::zone::binary64::g_fold(
                Binary64::ONE,
                crate::domain::zone::binary64::g_fold(Binary64::ONE, g).unwrap(),
            )
            .unwrap();
            let mut widest = (0_u32, 0_u32);
            let mut measure = |c: Binary64| {
                let (reduced, engine) = chance_width(c);
                widest = (widest.0.max(reduced), widest.1.max(engine));
            };
            // Ungrouped C = L · fl(cd · m); single-item C = L · fl(fl(fl(cd · g) / 100) · m).
            let bases: BTreeSet<u64> = items
                .iter()
                .map(|cd| mul(Binary64(*cd), m).unwrap().0)
                .chain(singles.iter().map(|(cd, group)| {
                    let collapsed =
                        div(mul(Binary64(*cd), Binary64(*group)).unwrap(), hundred).unwrap();
                    mul(collapsed, m).unwrap().0
                }))
                .collect();
            for base in &bases {
                for l in &ls {
                    let c = mul(*l, Binary64(*base)).unwrap();
                    measure(c);
                    if c.rational()
                        .unwrap()
                        .compare(&hundred.rational().unwrap())
                        .is_gt()
                    {
                        measure(remainder(c));
                    }
                }
            }
            for (group, rows) in &groups {
                let weighted: Vec<Binary64> = rows
                    .iter()
                    .map(|cd| mul(mul(Binary64(*cd), m).unwrap(), Binary64(*group)).unwrap())
                    .collect();
                for l in &ls {
                    let raw: Vec<Binary64> =
                        weighted.iter().map(|w| mul(*w, *l).unwrap()).collect();
                    let mut sum = Binary64::ZERO;
                    for product in &raw {
                        sum = add(sum, div(*product, hundred).unwrap()).unwrap();
                    }
                    if sum == Binary64::ZERO {
                        continue;
                    }
                    measure(sum);
                    if sum
                        .rational()
                        .unwrap()
                        .compare(&hundred.rational().unwrap())
                        .is_ge()
                    {
                        measure(remainder(sum));
                    }
                    let mut total = Binary64::ZERO;
                    for product in &raw {
                        total = add(total, div(*product, sum).unwrap()).unwrap();
                        measure(total);
                    }
                }
            }
            assert_eq!(u64::from(widest.0), widths[label].as_u64().unwrap(), "{label}");
            assert!(widest.1 <= 255, "{label}: engine operand width {}", widest.1);
        }
        assert_eq!(widths["G=1"].as_u64(), Some(141));
        assert_eq!(widths["G=2^-32"].as_u64(), Some(173));
        assert_eq!(widths["G=2^32"].as_u64(), Some(131));
    }

    #[test]
    fn quantity_reproduces_pinned_rnd_get_including_min_above_max() {
        // Normal rows: min + roll_below(max − min + 1).
        let mut w = Words::new(vec![0, 7]);
        assert_eq!(quantity(&mut w, 5, 5).unwrap(), 5, "min = max still draws (P3)");
        assert_eq!(w.left(), 1);
        // min > max: uniform over {max + 2, …, min}; k ∈ {0, −1} always gives min.
        let mut w = Words::new(vec![2, 0, 0, 0]);
        assert_eq!(quantity(&mut w, 5, 1).unwrap(), 3, "get(5, 1) ∈ {{3, 4, 5}}");
        assert_eq!(quantity(&mut w, 5, 4).unwrap(), 5, "k = 0");
        assert_eq!(quantity(&mut w, 5, 3).unwrap(), 5, "k = −1");
        assert_eq!(quantity(&mut w, 9_000_000, 1_100_000).unwrap(), 9_000_000);
        // The 29181 row: 7,899,999 outcomes, the lowest is max + 2.
        let mut w = Words::new(vec![7_899_998]);
        assert_eq!(quantity(&mut w, 9_000_000, 1_100_000).unwrap(), 1_100_002);
    }
}
