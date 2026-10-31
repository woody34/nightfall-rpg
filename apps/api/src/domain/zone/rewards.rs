//! B16 XP/SP distribution under V7 (design §4.5; R5, E30, E32; DR3 P1-1, P2-2, P2-10): the
//! pinned `L2Attackable.calculateRewards` (`l2j-server-game` abfde049,
//! `model/actor/L2Attackable.java:332-535`), `calcExpSp` (`:1139-1176`) and
//! `L2Party.distributeXpAndSp` (`model/L2Party.java:644-714`) over the reward snapshot taken in
//! `death_v7`, with every Java `double` step one `fl` and every `float` step one `fl32`.
//! Pure and draw-free.

use std::collections::{BTreeMap, BTreeSet};

use super::binary64::{pow56, Binary64, Rational};
use super::drops::DropError;
use super::phase5_rules::Phase5Rules;
use super::EntityId;

/// One rewarded player's facts at resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewardPlayer {
    /// `getLevel()`.
    pub level: u32,
    /// Alive (dead players are skipped exactly as pinned).
    pub alive: bool,
    /// Within `PartyRange` 1,600 of the NPC.
    pub in_range: bool,
}

/// One award, in emission order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Award {
    /// The recipient.
    pub player: EntityId,
    /// XP before the existing `award_xp` path.
    pub exp: u64,
    /// SP.
    pub sp: u64,
}

fn fl(r: &Rational) -> Result<Rational, DropError> {
    Ok(r.fl()?.rational()?)
}

fn fl32(r: &Rational) -> Result<Rational, DropError> {
    Ok(r.fl32()?.rational()?)
}

fn int(n: u64) -> Result<Rational, DropError> {
    Ok(Rational::integer(
        i64::try_from(n).map_err(|_| DropError::Rules("reward beyond i64"))?,
    ))
}

fn floor(r: &Rational) -> Result<u64, DropError> {
    Ok(r.floor_u64()?)
}

/// The NPC base rewards (`L2Npc.getExpReward/getSpReward`, `:961-970`):
/// `X = ⌊fl(fl(lvl² × expRate) × RateXp')⌋` and `S = ⌊fl(sp × RateSp')⌋`, with the G-folded
/// `RateXp'`/`RateSp'` of the bound rules (E29). `expRate` absent is "0".
pub fn base_rewards(
    rules: &Phase5Rules,
    level: u32,
    exp_rate: &str,
    sp: &str,
) -> Result<(u64, u64), DropError> {
    let experience_rate = rules
        .rate_prime("RateXp")
        .ok_or(DropError::Rules("RateXp'"))?
        .rational()?;
    let skill_point_rate = rules
        .rate_prime("RateSp")
        .ok_or(DropError::Rules("RateSp'"))?
        .rational()?;
    let squared = int(u64::from(level).saturating_mul(u64::from(level)))?;
    let exp_rate = Rational::parse_decimal(exp_rate)?.fl()?.rational()?;
    let sp = Rational::parse_decimal(sp)?.fl()?.rational()?;
    let x = floor(&fl(&fl(&squared.mul(&exp_rate))?.mul(&experience_rate))?)?;
    let s = floor(&fl(&sp.mul(&skill_point_rate))?)?;
    Ok((x, s))
}

/// `calcExpSp(diff, damage, totalDamage)` with `ExponentXp = ExponentSp = 0` (`:1139-1176`).
pub fn calc_exp_sp(
    diff: i64,
    damage: u64,
    total: u64,
    x: u64,
    s: u64,
) -> Result<(u64, u64), DropError> {
    let diff = diff.max(-5);
    let total = int(total)?;
    let mut xp = fl(&fl(&int(x)?.mul(&int(damage)?))?.div(&total)?)?;
    let mut sp = fl(&fl(&int(s)?.mul(&int(damage)?))?.div(&total)?)?;
    if diff > 5 {
        let k = i32::try_from(diff.saturating_sub(5)).map_err(|_| DropError::Rules("level gap"))?;
        // E32: the exact power of fl(5/6) rounded once; k outside [1, 79] fails closed.
        let p = pow56(k)?.rational()?;
        xp = fl(&xp.mul(&p))?;
        sp = fl(&sp.mul(&p))?;
    }
    let zero = Rational::integer(0);
    if xp.compare(&zero).is_le() {
        // Pinned: no XP ⇒ no SP (DR3 P1-1), tested on the double before truncation.
        return Ok((0, 0));
    }
    let sp = if sp.compare(&zero).is_le() { zero } else { sp };
    Ok((floor(&xp)?, floor(&sp)?))
}

/// `exp *= penalty` etc.: a `long *= float` is `⌊fl32(fl32(v) × m)⌋` (float arithmetic).
fn float_scale(v: u64, m: &Rational) -> Result<u64, DropError> {
    floor(&fl32(&fl32(&int(v)?)?.mul(m))?)
}

/// `L2Party.distributeXpAndSp` (`:644-714`) with the `highfive` cutoff.
fn party_distribute(
    rules: &Phase5Rules,
    exp: u64,
    sp: u64,
    rewarded: &[(EntityId, RewardPlayer)],
    top: u32,
    out: &mut Vec<Award>,
) -> Result<(), DropError> {
    let party = &rules.parts().party.party;
    let n = rewarded.len();
    let bonus = |key: &str| -> Result<Rational, DropError> {
        if n < 2 {
            return Ok(Rational::integer(1));
        }
        let index = n.saturating_sub(1).min(8);
        let b = Binary64::parse_hex(
            party
                .bonus_exp_sp_bits
                .get(index)
                .ok_or(DropError::Rules("BONUS_EXP_SP"))?,
        )?
        .rational()?;
        // RatePartyXp/RatePartySp are pinned float configs, never G-multiplied (P5).
        let rate = rules
            .parts()
            .rates
            .float_rates
            .get(key)
            .ok_or(DropError::Rules("RateParty"))?;
        let rate = super::binary64::fl32_decimal(&rate.value)?.rational()?;
        fl(&b.mul(&rate))
    };
    let exp = floor(&fl(&int(exp)?.mul(&bonus("RatePartyXp")?))?)?;
    let sp = floor(&fl(&int(sp)?.mul(&bonus("RatePartySp")?))?)?;
    let squares: u64 = rewarded
        .iter()
        .map(|(_, p)| u64::from(p.level).saturating_mul(u64::from(p.level)))
        .sum();
    let squares = int(squares)?;
    let gaps = &party.xp_cutoff.cutoff_gaps;
    for (player, facts) in rewarded {
        if !facts.alive {
            continue;
        }
        let lsq = int(u64::from(facts.level).saturating_mul(u64::from(facts.level)))?;
        // pre = fl(fl(lvl² / Σsq) × penalty), penalty = 1.
        let pre = fl(&fl(&lsq.div(&squares)?)?.mul(&Rational::integer(1)))?;
        // Math.round(v) = ⌊v + 1/2⌋ on the exact double v (Java 7+ semantics).
        let v = fl(&int(exp)?.mul(&pre))?;
        let half = Rational::integer(1).div(&Rational::integer(2))?;
        let ae = floor(&v.add(&half))?;
        let as_ = floor(&fl(&int(sp)?.mul(&pre))?)?;
        let gap = i64::from(top).saturating_sub(i64::from(facts.level));
        let award = gaps
            .iter()
            .find(|g| i64::from(g.min) <= gap && gap <= i64::from(g.max))
            .map_or((0, 0), |g| {
                (
                    ae.saturating_mul(u64::from(g.percent)) / 100,
                    as_.saturating_mul(u64::from(g.percent)) / 100,
                )
            });
        out.push(Award {
            player: *player,
            exp: award.0,
            sp: award.1,
        });
    }
    Ok(())
}

/// The reward pass of design §4.5 over the snapshot `damage` (players with `damage > 1`
/// within `PartyRange`, keyed by `EntityId`): visited in ascending `EntityId` (X7); a party
/// pass removes its alive members' entries, a dead member's entry stays and its later visit
/// runs a `partyDmg = 0` pass that awards nothing (DR3 P2-10). Awards are in emission order.
pub fn reward_pass(
    rules: &Phase5Rules,
    damage: &BTreeMap<EntityId, u64>,
    parties: &[Vec<EntityId>],
    players: &BTreeMap<EntityId, RewardPlayer>,
    npc_level: u32,
    base: (u64, u64),
) -> Result<Vec<Award>, DropError> {
    let total: u64 = damage.values().copied().fold(0_u64, u64::saturating_add);
    let mut remaining = damage.clone();
    let party_of: BTreeMap<EntityId, &Vec<EntityId>> = parties
        .iter()
        .flat_map(|party| party.iter().map(move |member| (*member, party)))
        .collect();
    let mut awards = Vec::new();
    let visits: Vec<EntityId> = damage.keys().copied().collect();
    let facts = |id: &EntityId| {
        players
            .get(id)
            .copied()
            .ok_or(DropError::Rules("reward player"))
    };
    for id in visits {
        let Some(dealt) = remaining.get(&id).copied() else {
            continue;
        };
        let me = facts(&id)?;
        let Some(party) = party_of.get(&id) else {
            let (e, s) = calc_exp_sp(
                i64::from(me.level).saturating_sub(i64::from(npc_level)),
                dealt,
                total,
                base.0,
                base.1,
            )?;
            // exp *= penalty (float; no servitors, penalty 1).
            let e = float_scale(e, &Rational::integer(1))?;
            if me.alive {
                awards.push(Award {
                    player: id,
                    exp: e,
                    sp: s,
                });
            }
            continue;
        };
        let mut party_damage = 0_u64;
        let mut party_level = 0_u32;
        let mut rewarded = Vec::new();
        let mut seen = BTreeSet::new();
        for member in *party {
            let pm = facts(member)?;
            if !pm.alive || !seen.insert(*member) {
                continue;
            }
            if let Some(dealt) = remaining.remove(member) {
                if pm.in_range {
                    party_damage = party_damage.saturating_add(dealt);
                    rewarded.push((*member, pm));
                    party_level = party_level.max(pm.level);
                }
            } else if pm.in_range {
                rewarded.push((*member, pm));
                party_level = party_level.max(pm.level);
            }
        }
        let multiplier = if party_damage < total {
            fl32(&fl32(&int(party_damage)?)?.div(&fl32(&int(total)?)?)?)?
        } else {
            Rational::integer(1)
        };
        let (e, s) = calc_exp_sp(
            i64::from(party_level).saturating_sub(i64::from(npc_level)),
            party_damage,
            total,
            base.0,
            base.1,
        )?;
        let e = float_scale(e, &multiplier)?;
        let s = float_scale(s, &multiplier)?;
        if party_damage > 0 {
            party_distribute(rules, e, s, &rewarded, party_level, &mut awards)?;
        }
    }
    Ok(awards)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::too_many_lines)]
mod tests {
    use super::*;

    fn rules() -> std::sync::Arc<Phase5Rules> {
        crate::infrastructure::economy_data::embedded_phase5_rules().unwrap()
    }

    fn oracle() -> serde_json::Value {
        let party: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-party.json"
        ))
        .unwrap();
        party["x_vectors"].clone()
    }

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    fn pair(v: &serde_json::Value) -> (u64, u64) {
        (v[0].as_u64().unwrap(), v[1].as_u64().unwrap())
    }

    /// Family X: base rewards and the solo diffs (pow56 per E32; k = 80 fails closed).
    #[test]
    fn solo_rewards_match_the_oracle() {
        let rules = rules();
        let x = oracle();
        let (base_x, base_s) = base_rewards(&rules, 40, "24.8737348778133", "17624").unwrap();
        assert_eq!(base_x, x["solo"]["X"].as_u64().unwrap());
        assert_eq!(base_s, x["solo"]["S"].as_u64().unwrap());
        for (diff, expected) in x["solo"]["results"].as_object().unwrap() {
            let diff: i64 = diff.parse().unwrap();
            let (e, s) = calc_exp_sp(diff, 700, 1000, base_x, base_s).unwrap();
            let e = float_scale(e, &Rational::integer(1)).unwrap();
            assert_eq!((e, s), pair(expected), "diff {diff}");
        }
        assert!(calc_exp_sp(85, 700, 1000, base_x, base_s).is_err(), "k = 80 fails closed");
        // 0 < x < 1 keeps its SP; the double, not the truncation, decides.
        let f = &x["fractional_x"];
        assert_eq!(calc_exp_sp(0, 1, 3, 2, 300).unwrap(), pair(&f["result"]));
        // long *= float above 2^24.
        let steps = &x["float_steps"];
        assert_eq!(
            float_scale(steps["e"].as_u64().unwrap(), &Rational::integer(1)).unwrap(),
            steps["solo"].as_u64().unwrap()
        );
        // The 36 SP-only templates (expRate absent → "0") award 0 XP and 0 SP at any level,
        // incl. the raid-class sp rows. They have no drops.toml row, so the sp values are the
        // design's (DR3): the rule is the expRate, not the sp.
        assert_eq!(x["sp_only"].as_array().unwrap().len(), 36);
        for sp in ["557272", "144137", "17624"] {
            let (bx, bs) = base_rewards(&rules, 70, "0", sp).unwrap();
            assert_eq!(bx, 0);
            assert!(bs > 0);
            for level in [1_i64, 70, 85] {
                assert_eq!(calc_exp_sp(level - 70, 1000, 1000, bx, bs).unwrap(), (0, 0));
            }
        }
    }

    /// Family X: party distribution n = 1…9, the highfive gaps and the n = 5 bonus rounding.
    #[test]
    fn party_distribution_matches_the_oracle() {
        let rules = rules();
        let x = oracle();
        let run = |levels: &[u32], top: u32| {
            let rewarded: Vec<(EntityId, RewardPlayer)> = levels
                .iter()
                .enumerate()
                .map(|(i, level)| {
                    (
                        id(u128::try_from(i).unwrap() + 1),
                        RewardPlayer {
                            level: *level,
                            alive: true,
                            in_range: true,
                        },
                    )
                })
                .collect();
            let mut out = Vec::new();
            party_distribute(&rules, 100_000, 5000, &rewarded, top, &mut out).unwrap();
            out.iter().map(|a| (a.exp, a.sp)).collect::<Vec<_>>()
        };
        for row in x["party"].as_array().unwrap() {
            let levels: Vec<u32> = row["levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
                .collect();
            let expected: Vec<(u64, u64)> =
                row["awards"].as_array().unwrap().iter().map(pair).collect();
            assert_eq!(run(&levels, 60), expected, "n = {}", levels.len());
        }
        let gaps: Vec<(u64, u64)> = x["gaps"].as_array().unwrap().iter().map(pair).collect();
        assert_eq!(run(&[70, 61, 60, 56, 55, 75], 70), gaps);
        // n = 5: ⌊fl(e × 1.40)⌋, e.g. e = 45 → 62 (not the exact 63).
        let mut one = Vec::new();
        let five: Vec<(EntityId, RewardPlayer)> = (1..=5)
            .map(|n| {
                (
                    id(n),
                    RewardPlayer {
                        level: 1,
                        alive: true,
                        in_range: true,
                    },
                )
            })
            .collect();
        party_distribute(&rules, 45, 0, &five, 1, &mut one).unwrap();
        let total: u64 = one.iter().map(|a| a.exp).sum();
        assert!(total <= 62, "the bonus-scaled 62 is split, never 63");
    }

    /// Family X: the full reward pass, incl. partyMul and the dead-member second pass.
    #[test]
    fn the_reward_pass_matches_the_oracle() {
        let rules = rules();
        let x = &oracle()["reward_pass"];
        let base = (x["X"].as_u64().unwrap(), x["S"].as_u64().unwrap());
        assert_eq!(base_rewards(&rules, 52, "24.8737348778133", "17624").unwrap(), base);
        let players: BTreeMap<EntityId, RewardPlayer> = x["players"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                (
                    id(k.parse().unwrap()),
                    RewardPlayer {
                        level: u32::try_from(v["level"].as_u64().unwrap()).unwrap(),
                        alive: v["alive"].as_bool().unwrap(),
                        in_range: v["in_range"].as_bool().unwrap(),
                    },
                )
            })
            .collect();
        let damage = |table: &serde_json::Value| -> BTreeMap<EntityId, u64> {
            table
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (id(k.parse().unwrap()), v["damage"].as_u64().unwrap()))
                .collect()
        };
        let awards = |result: &serde_json::Value| -> Vec<(EntityId, (u64, u64))> {
            result["awards"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (id(k.parse().unwrap()), pair(v)))
                .collect()
        };
        let party = vec![vec![id(1), id(2), id(3), id(4)]];
        let got = reward_pass(&rules, &damage(&x["R"]), &party, &players, 52, base).unwrap();
        let mut by_player: BTreeMap<EntityId, (u64, u64)> = BTreeMap::new();
        for a in &got {
            let row = by_player.entry(a.player).or_default();
            row.0 += a.exp;
            row.1 += a.sp;
        }
        assert_eq!(by_player.into_iter().collect::<Vec<_>>(), awards(&x["result"]));
        // Emission order: the party pass (1 then 4), then the solo attacker 5.
        assert_eq!(got.iter().map(|a| a.player).collect::<Vec<_>>(), vec![id(1), id(4), id(5)]);
        let whole = &x["whole_party"];
        let damage_whole: BTreeMap<EntityId, u64> =
            [(id(1), 500), (id(4), 500)].into_iter().collect();
        let got =
            reward_pass(&rules, &damage_whole, &[vec![id(1), id(4)]], &players, 52, base).unwrap();
        let got: Vec<(EntityId, (u64, u64))> =
            got.iter().map(|a| (a.player, (a.exp, a.sp))).collect();
        assert_eq!(got, awards(whole));
    }
}
