//! Derived combat stats (plan §3.1): one [`StatSheet`] per living entity, recomputed when
//! level or equipment changes, never per tick.
//!
//! Players derive theirs from a [`ClassTemplate`], their level and the starter weapon. NPC
//! template values are final (plan §3.1: no class bonuses applied a second time) and enter
//! through [`StatSheet::from_final`]. Every function here is pure integer math over
//! [`Scaled`]; the rounding of each step is the one the plan's table names.

use crate::domain::BaseStats;

use super::scaled::{add, floor_div, isqrt, mul, narrow, round_div, Scaled, StatError, Q128};
use super::stat_rules::{ClassTemplate, FormulaConstants, StatKind, StatRules, WeaponBlock};

/// Derived stats an attack or a resource bar reads. Fields are private so every sheet has
/// passed its constructor's checks (positive defence and attack speed, crit within cap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSheet {
    level: u32,
    max_hp: u32,
    max_mp: u32,
    p_atk: Scaled,
    p_def: Scaled,
    accuracy: Scaled,
    evasion: Scaled,
    crit_permille: u32,
    attack_speed: Scaled,
    random_damage: u32,
}

/// Final stat values for an entity that does not derive from a class (NPC templates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct FinalStats {
    pub level: u32,
    pub max_hp: u32,
    pub max_mp: u32,
    pub p_atk: Scaled,
    pub p_def: Scaled,
    pub accuracy: Scaled,
    pub evasion: Scaled,
    pub crit_permille: u32,
    pub attack_speed: Scaled,
    pub random_damage: u32,
}

impl StatSheet {
    /// A player of `class` at `level`, holding `weapon` (or bare-handed). The weapon's P.Atk,
    /// crit base and attack speed replace the fist values, as L2J `<set>` funcs do.
    pub fn for_player(
        rules: &StatRules,
        class: &ClassTemplate,
        level: u32,
        weapon: Option<&WeaponBlock>,
    ) -> Result<Self, StatError> {
        if level == 0 || level > rules.max_level() {
            return Err(StatError::LevelOutOfRange(level));
        }
        let c = rules.constants();
        let bonus = |kind: StatKind| rules.bonus().bonus(kind, kind.of(&class.base));
        let lm = level_mod(c, level)?;
        let (atk_base, crit_base, speed_base, acc_bonus, random) = match weapon {
            Some(w) => (w.p_atk, w.crit_rate, w.attack_speed, w.accuracy, w.random_damage),
            None => (
                class.fist_p_atk,
                class.fist_crit_rate,
                class.fist_attack_speed,
                Scaled::ZERO,
                fist_random_damage(c, level)?,
            ),
        };
        let dex = bonus(StatKind::Dex)?;
        Self::from_final(FinalStats {
            level,
            max_hp: resource_max(class.hp.at(level)?, bonus(StatKind::Con)?)?,
            max_mp: resource_max(class.mp.at(level)?, bonus(StatKind::Men)?)?,
            p_atk: p_atk(atk_base, bonus(StatKind::Str)?, lm)?,
            p_def: p_def(class.p_def_unarmoured, lm)?,
            accuracy: accuracy(c, &class.base, level, rules.accuracy_level_add(level)?, acc_bonus)?,
            evasion: evasion(c, &class.base, level, rules.evasion_level_add(level)?)?,
            crit_permille: crit_permille(c, crit_base, dex)?,
            attack_speed: attack_speed(c, speed_base, dex)?,
            random_damage: random,
        })
    }

    /// Final values (NPC templates). Rejects zero P.Def, zero attack speed and zero max HP,
    /// which the damage and timing formulas divide by or depend on.
    pub fn from_final(s: FinalStats) -> Result<Self, StatError> {
        if s.p_def.raw() <= 0 || s.attack_speed.raw() <= 0 {
            return Err(StatError::NonPositiveDivisor);
        }
        if s.p_atk.raw() < 0 || s.accuracy.raw() < 0 || s.evasion.raw() < 0 {
            return Err(StatError::Overflow);
        }
        if s.level == 0 {
            return Err(StatError::LevelOutOfRange(0));
        }
        Ok(Self {
            level: s.level,
            max_hp: s.max_hp,
            max_mp: s.max_mp,
            p_atk: s.p_atk,
            p_def: s.p_def,
            accuracy: s.accuracy,
            evasion: s.evasion,
            crit_permille: s.crit_permille,
            attack_speed: s.attack_speed,
            random_damage: s.random_damage,
        })
    }

    /// Level.
    #[must_use]
    pub const fn level(&self) -> u32 {
        self.level
    }
    /// Maximum HP (whole units).
    #[must_use]
    pub const fn max_hp(&self) -> u32 {
        self.max_hp
    }
    /// Maximum MP (whole units).
    #[must_use]
    pub const fn max_mp(&self) -> u32 {
        self.max_mp
    }
    /// P.Atk, unrounded.
    #[must_use]
    pub const fn p_atk(&self) -> Scaled {
        self.p_atk
    }
    /// P.Def, unrounded; always > 0.
    #[must_use]
    pub const fn p_def(&self) -> Scaled {
        self.p_def
    }
    /// Accuracy, rounded to whole units as in HF `CharStat.getAccuracy`.
    #[must_use]
    pub const fn accuracy(&self) -> Scaled {
        self.accuracy
    }
    /// Evasion, rounded to whole units and capped.
    #[must_use]
    pub const fn evasion(&self) -> Scaled {
        self.evasion
    }
    /// Critical rate in per mille, capped.
    #[must_use]
    pub const fn crit_permille(&self) -> u32 {
        self.crit_permille
    }
    /// Attack speed, rounded to whole units and capped; always > 0.
    #[must_use]
    pub const fn attack_speed(&self) -> Scaled {
        self.attack_speed
    }
    /// Random damage radius in percent.
    #[must_use]
    pub const fn random_damage(&self) -> u32 {
        self.random_damage
    }
}

/// `LM_Q = (L + offset) * Q / divisor`; with the HF constants `(L + 89) * 10000`, exactly
/// `(L + 89) / 100`.
pub fn level_mod(c: &FormulaConstants, level: u32) -> Result<Scaled, StatError> {
    let n = add(level.into(), c.level_mod_offset.into())?;
    Scaled::from_i128(floor_div(mul(n, Q128)?, c.level_mod_divisor.into())?)
}

/// `A_Q = F(base_Q * STR_Q * LM_Q / Q²)`.
pub fn p_atk(base: Scaled, str_bonus: Scaled, lm: Scaled) -> Result<Scaled, StatError> {
    let product = mul(mul(base.raw().into(), str_bonus.raw().into())?, lm.raw().into())?;
    Scaled::from_i128(floor_div(product, mul(Q128, Q128)?)?)
}

/// `D_Q = F(base_Q * LM_Q / Q)` (no armour slots occupied in Phase 1).
pub fn p_def(base_unarmoured: Scaled, lm: Scaled) -> Result<Scaled, StatError> {
    let product = mul(base_unarmoured.raw().into(), lm.raw().into())?;
    Scaled::from_i128(floor_div(product, Q128)?)
}

/// `max = F(curve_Q * bonus_Q / Q²)` for HP (CON) and MP (MEN).
pub fn resource_max(curve_raw: i128, bonus: Scaled) -> Result<u32, StatError> {
    let product = mul(curve_raw, bonus.raw().into())?;
    narrow(floor_div(product, mul(Q128, Q128)?)?)
}

/// `root_Q = isqrt(DEX * Q²)`: `sqrt(DEX)` floored to `1/Q`.
pub fn sqrt_dex(dex: u32) -> Result<Scaled, StatError> {
    let radicand = u128::from(dex)
        .checked_mul(1_000_000_000_000)
        .ok_or(StatError::Overflow)?;
    Scaled::from_i128(i128::try_from(isqrt(radicand)).map_err(|_| StatError::Overflow)?)
}

/// `acc_Q = Q * round((m * root_Q + L*Q + levelAdd_Q + weapon_Q) / Q)` (HF `FuncAtkAccuracy`, m = 6).
pub fn accuracy(
    c: &FormulaConstants,
    base: &BaseStats,
    level: u32,
    level_add: Scaled,
    weapon: Scaled,
) -> Result<Scaled, StatError> {
    let root = mul(sqrt_dex(base.dex)?.raw().into(), c.accuracy_dex_multiplier.into())?;
    let lvl = mul(level.into(), Q128)?;
    let sum = add(add(add(root, lvl)?, level_add.raw().into())?, weapon.raw().into())?;
    Scaled::from_i128(mul(round_div(sum, Q128)?, Q128)?)
}

/// `eva_Q = Q * min(cap, round((m * root_Q + L*Q + levelAdd_Q) / Q))` (HF `FuncAtkEvasion`, player branch).
pub fn evasion(
    c: &FormulaConstants,
    base: &BaseStats,
    level: u32,
    level_add: Scaled,
) -> Result<Scaled, StatError> {
    let root = mul(sqrt_dex(base.dex)?.raw().into(), c.evasion_dex_multiplier.into())?;
    let sum = add(add(root, mul(level.into(), Q128)?)?, level_add.raw().into())?;
    let cap = mul(c.evasion_cap.into(), Q128)?;
    Scaled::from_i128(mul(round_div(sum, Q128)?, Q128)?.min(cap))
}

/// `crit‰ = min(cap, F(base * DEX_Q * scale / Q))`.
pub fn crit_permille(c: &FormulaConstants, base: u32, dex: Scaled) -> Result<u32, StatError> {
    let product = mul(mul(base.into(), dex.raw().into())?, c.crit_scale.into())?;
    let rate = floor_div(product, Q128)?.min(c.crit_cap_permille.into());
    narrow(rate)
}

/// `speed_Q = Q * min(cap, round(base * DEX_Q / Q))`; must stay positive.
pub fn attack_speed(c: &FormulaConstants, base: u32, dex: Scaled) -> Result<Scaled, StatError> {
    let product = mul(mul(base.into(), Q128)?, dex.raw().into())?;
    let speed = mul(round_div(product, mul(Q128, Q128)?)?, Q128)?
        .min(mul(c.attack_speed_cap.into(), Q128)?);
    if speed <= 0 {
        return Err(StatError::NonPositiveDivisor);
    }
    Scaled::from_i128(speed)
}

/// Bare-handed random damage radius: `5 + isqrt(level)` (HF `getRandomDamageMultiplier`).
pub fn fist_random_damage(c: &FormulaConstants, level: u32) -> Result<u32, StatError> {
    let root = u32::try_from(isqrt(level.into())).map_err(|_| StatError::Overflow)?;
    c.fist_random_base
        .checked_add(root)
        .ok_or(StatError::Overflow)
}
