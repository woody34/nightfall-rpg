//! XP, level and death loss (plan §3.1 "XP / death", HF tables).
//!
//! `X[L]` is the cumulative XP to reach level `L` for `1..=max_level + 1`. The extra row is
//! the HF sentinel: XP is capped at `X[max + 1] - 1` (L2J `PlayableStat.addExp`) and the
//! max level's death-loss span is `X[max + 1] - X[max]`. Level is derived from XP by
//! threshold search, never stored independently of it.

use super::scaled::{floor_div, mul, narrow, StatError, Q128};
use super::stat_rules::StatRules;

/// Highest XP a character can hold: `X[max_level + 1] - 1`.
pub fn xp_cap(rules: &StatRules) -> Result<u64, StatError> {
    let sentinel = rules
        .max_level()
        .checked_add(1)
        .ok_or(StatError::Overflow)?;
    rules
        .xp_to_level(sentinel)?
        .checked_sub(1)
        .ok_or(StatError::Overflow)
}

/// The level for `xp`: the highest `L` in `1..=max_level` with `X[L] <= xp`.
#[must_use]
pub fn level_for_xp(rules: &StatRules, xp: u64) -> u32 {
    let reachable = rules
        .xp_table()
        .iter()
        .take_while(|&&threshold| threshold <= xp)
        .count();
    u32::try_from(reachable)
        .unwrap_or(u32::MAX)
        .clamp(1, rules.max_level())
}

/// `xp + reward`, capped at [`xp_cap`]. Rate 1, no level-gap multiplier (plan §3.1).
pub fn add_xp(rules: &StatRules, xp: u64, reward: u64) -> Result<u64, StatError> {
    Ok(xp.saturating_add(reward).min(xp_cap(rules)?))
}

/// XP lost dying at `level`: `F((X[L+1] − X[L]) * loss_Q[L] / Q)`.
pub fn death_xp_loss(rules: &StatRules, level: u32) -> Result<u64, StatError> {
    let fraction = rules.death_loss_fraction(level)?;
    let next = level.checked_add(1).ok_or(StatError::Overflow)?;
    let span = rules
        .xp_to_level(next)?
        .checked_sub(rules.xp_to_level(level)?)
        .ok_or(StatError::Overflow)?;
    narrow(floor_div(mul(span.into(), fraction.raw().into())?, Q128)?)
}

/// XP after dying with `xp` at `level`: `max(0, xp − loss)`. The caller re-derives the
/// level with [`level_for_xp`] (de-levelling is allowed, HF `Delevel = True`).
pub fn xp_after_death(rules: &StatRules, xp: u64, level: u32) -> Result<u64, StatError> {
    Ok(xp.saturating_sub(death_xp_loss(rules, level)?))
}
