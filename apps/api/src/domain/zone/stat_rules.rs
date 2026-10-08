//! Immutable stat rules: the validated form of `packages/data/tables/*.toml` and
//! `packages/data/classes/*.toml` (plan §3.1, Story E1.2).
//!
//! The infrastructure loader parses the files and hands the values to [`StatRules::new`],
//! which checks every semantic invariant the calculations rely on (coverage, monotone XP,
//! positive defence and speed, coefficient bounds, one fighter template per race) and
//! derives every class at every level once, so data that would overflow is rejected at
//! startup instead of at the first fight. All violations are returned together.

use std::fmt;

use crate::domain::{BaseStats, Race};

use super::scaled::{Scaled, StatError, Q};
use super::stat_sheet::StatSheet;

/// Number of rows in each bonus table: L2J `BaseStats.MAX_STAT_VALUE` (indices `0..=99`).
pub const BONUS_TABLE_LEN: usize = 100;

/// The six base stats, in the order the tables list them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum StatKind {
    Str,
    Dex,
    Con,
    Int,
    Wit,
    Men,
}

impl StatKind {
    /// All six, in table order.
    pub const ALL: [StatKind; 6] = [
        StatKind::Str,
        StatKind::Dex,
        StatKind::Con,
        StatKind::Int,
        StatKind::Wit,
        StatKind::Men,
    ];

    /// This stat's value in a [`BaseStats`].
    #[must_use]
    pub const fn of(self, base: &BaseStats) -> u32 {
        match self {
            StatKind::Str => base.str,
            StatKind::Dex => base.dex,
            StatKind::Con => base.con,
            StatKind::Int => base.int,
            StatKind::Wit => base.wit,
            StatKind::Men => base.men,
        }
    }
}

impl fmt::Display for StatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            StatKind::Str => "STR",
            StatKind::Dex => "DEX",
            StatKind::Con => "CON",
            StatKind::Int => "INT",
            StatKind::Wit => "WIT",
            StatKind::Men => "MEN",
        };
        f.write_str(name)
    }
}

/// One stat's bonus multipliers, index = stat value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BonusTable(Vec<Scaled>);

impl BonusTable {
    /// Wraps the rows; [`StatRules::new`] checks length and shape.
    #[must_use]
    pub fn new(rows: Vec<Scaled>) -> Self {
        Self(rows)
    }

    /// The multiplier for `value`. `StatOutOfRange` past the last row.
    pub fn get(&self, value: u32) -> Result<Scaled, StatError> {
        usize::try_from(value)
            .ok()
            .and_then(|i| self.0.get(i))
            .copied()
            .ok_or(StatError::StatOutOfRange(value))
    }
}

/// The six bonus tables.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct StatBonusTables {
    pub str: BonusTable,
    pub dex: BonusTable,
    pub con: BonusTable,
    pub int: BonusTable,
    pub wit: BonusTable,
    pub men: BonusTable,
}

impl StatBonusTables {
    /// The table for one stat.
    #[must_use]
    pub const fn table(&self, kind: StatKind) -> &BonusTable {
        match kind {
            StatKind::Str => &self.str,
            StatKind::Dex => &self.dex,
            StatKind::Con => &self.con,
            StatKind::Int => &self.int,
            StatKind::Wit => &self.wit,
            StatKind::Men => &self.men,
        }
    }

    /// `kind`'s multiplier at `value`.
    pub fn bonus(&self, kind: StatKind, value: u32) -> Result<Scaled, StatError> {
        self.table(kind).get(value)
    }
}

/// `base + per_level*n + accel*n²` with `n = level - 1`: the exact fit of a class's
/// per-level HP or MP table (the loader proves it reproduces every source row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Quadratic {
    pub base: Scaled,
    pub per_level: Scaled,
    pub accel: Scaled,
}

impl Quadratic {
    /// The curve at `level` (>= 1), as a raw `1/Q` count.
    pub fn at(&self, level: u32) -> Result<i128, StatError> {
        use super::scaled::{add, mul};
        let n = i128::from(
            level
                .checked_sub(1)
                .ok_or(StatError::LevelOutOfRange(level))?,
        );
        let linear = mul(i128::from(self.per_level.raw()), n)?;
        let square = mul(mul(i128::from(self.accel.raw()), n)?, n)?;
        add(add(i128::from(self.base.raw()), linear)?, square)
    }
}

/// Fighter or mystic starting template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub enum Archetype {
    Fighter,
    Mystic,
}

/// A starting class template (HF `stats/chars/baseStats/*.xml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassTemplate {
    /// Data id, e.g. `human_fighter`.
    pub id: String,
    /// L2 class id.
    pub class_id: u32,
    /// Display name.
    pub name: String,
    /// Nightfall race.
    pub race: Race,
    /// Fighter or mystic.
    pub archetype: Archetype,
    /// Fixed base stats.
    pub base: BaseStats,
    /// Fist P.Atk, replaced by an equipped weapon's.
    pub fist_p_atk: Scaled,
    /// Fist base critical rate, replaced by an equipped weapon's.
    pub fist_crit_rate: u32,
    /// Fist base attack speed, replaced by an equipped weapon's.
    pub fist_attack_speed: u32,
    /// Sum of the per-slot P.Def bases (nothing worn).
    pub p_def_unarmoured: Scaled,
    /// Max HP before CON.
    pub hp: Quadratic,
    /// Max MP before MEN.
    pub mp: Quadratic,
}

/// The fixed starter weapon (plan D1). Its values replace the class fist values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBlock {
    /// Data id.
    pub id: String,
    /// P.Atk (replaces the fist value).
    pub p_atk: Scaled,
    /// Base critical rate (replaces the fist value).
    pub crit_rate: u32,
    /// Base attack speed (replaces the fist value).
    pub attack_speed: u32,
    /// Random damage radius `r`: damage is scaled by `(100 + j) / 100`, `j` in `-r..=r`.
    pub random_damage: u32,
    /// Accuracy bonus.
    pub accuracy: Scaled,
    /// Attack range in L2 world units (literal; not tiles).
    pub attack_range_l2: u32,
}

/// Formula constants, each parsed from a literal line of the pinned source
/// (`tables/formulas.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct FormulaConstants {
    pub level_mod_offset: u32,
    pub level_mod_divisor: u32,
    pub accuracy_dex_multiplier: u32,
    pub evasion_dex_multiplier: u32,
    pub evasion_cap: u32,
    pub hit_base: u32,
    pub hit_per_point: u32,
    pub hit_scale: u32,
    pub hit_min_permille: u32,
    pub hit_max_permille: u32,
    pub hit_roll_range: u32,
    pub crit_scale: u32,
    pub crit_cap_permille: u32,
    pub crit_roll_range: u32,
    pub damage_coefficient: u32,
    pub crit_multiplier: u32,
    pub fist_random_base: u32,
    pub random_divisor: u32,
    pub attack_speed_cap: u32,
    pub attack_interval_ms: u32,
    pub impact_divisor: u32,
    pub hate_numerator: u32,
    pub hate_level_offset: u32,
    pub hate_cap: u64,
    pub respawn_restore_hp: Scaled,
    pub respawn_restore_mp: Scaled,
    pub spawn_protection_seconds: u32,
}

/// Everything [`StatRules::new`] needs, as parsed. Index `i` of every per-level vector is
/// level `i + 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct StatRulesParts {
    pub max_level: u32,
    pub bonus: StatBonusTables,
    pub constants: FormulaConstants,
    pub accuracy_level_add: Vec<Scaled>,
    pub evasion_level_add: Vec<Scaled>,
    /// Cumulative XP to reach levels `1..=max_level + 1` (the last is the sentinel).
    pub xp_to_level: Vec<u64>,
    /// Death XP loss fraction for levels `1..=max_level`.
    pub death_loss: Vec<Scaled>,
    pub classes: Vec<ClassTemplate>,
    pub starter_weapon: WeaponBlock,
}

/// One broken invariant, phrased for the startup error list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleViolation(pub String);

impl fmt::Display for RuleViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Validated, immutable stat rules. Constructed only through [`StatRules::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatRules {
    parts: StatRulesParts,
}

/// Largest level the tables may cover. Bounds every per-level loop and keeps `L * Q` and
/// `L + offset` far from overflow.
pub const MAX_SUPPORTED_LEVEL: u32 = 1000;

impl StatRules {
    /// Validates `parts`; on failure returns every violation found, not just the first.
    pub fn new(parts: StatRulesParts) -> Result<Self, Vec<RuleViolation>> {
        let mut v = Violations::default();
        check_levels(&parts, &mut v);
        check_bonus(&parts.bonus, &mut v);
        check_constants(&parts.constants, &mut v);
        check_per_level(&parts, &mut v);
        check_classes(&parts, &mut v);
        check_weapon(&parts.starter_weapon, &parts.constants, &mut v);
        if !v.0.is_empty() {
            return Err(v.0);
        }
        let rules = Self { parts };
        rules.prove_every_sheet(&mut v);
        if v.0.is_empty() {
            Ok(rules)
        } else {
            Err(v.0)
        }
    }

    /// Highest attainable level.
    #[must_use]
    pub const fn max_level(&self) -> u32 {
        self.parts.max_level
    }

    /// The six bonus tables.
    #[must_use]
    pub const fn bonus(&self) -> &StatBonusTables {
        &self.parts.bonus
    }

    /// Formula constants.
    #[must_use]
    pub const fn constants(&self) -> &FormulaConstants {
        &self.parts.constants
    }

    /// The starter weapon.
    #[must_use]
    pub const fn starter_weapon(&self) -> &WeaponBlock {
        &self.parts.starter_weapon
    }

    /// Every class, in data-id order.
    #[must_use]
    pub fn classes(&self) -> &[ClassTemplate] {
        &self.parts.classes
    }

    /// A class by data id.
    #[must_use]
    pub fn class(&self, id: &str) -> Option<&ClassTemplate> {
        self.parts.classes.iter().find(|c| c.id == id)
    }

    /// The fighter template of `race` (validated to exist for every race).
    #[must_use]
    pub fn fighter_for(&self, race: Race) -> Option<&ClassTemplate> {
        self.parts
            .classes
            .iter()
            .find(|c| c.race == race && c.archetype == Archetype::Fighter)
    }

    /// Accuracy level addition at `level`.
    pub fn accuracy_level_add(&self, level: u32) -> Result<Scaled, StatError> {
        per_level(&self.parts.accuracy_level_add, level)
    }

    /// Evasion level addition at `level`.
    pub fn evasion_level_add(&self, level: u32) -> Result<Scaled, StatError> {
        per_level(&self.parts.evasion_level_add, level)
    }

    /// Cumulative XP to reach `level`, for `1..=max_level + 1` (the sentinel).
    pub fn xp_to_level(&self, level: u32) -> Result<u64, StatError> {
        per_level(&self.parts.xp_to_level, level)
    }

    /// Death XP loss fraction at `level` (`1..=max_level`).
    pub fn death_loss_fraction(&self, level: u32) -> Result<Scaled, StatError> {
        per_level(&self.parts.death_loss, level)
    }

    /// The XP table, levels `1..=max_level + 1`.
    #[must_use]
    pub fn xp_table(&self) -> &[u64] {
        &self.parts.xp_to_level
    }

    /// Derives every class at every level, with and without the starter weapon, so a table
    /// combination that overflows or yields a non-positive maximum fails at startup.
    fn prove_every_sheet(&self, v: &mut Violations) {
        for class in &self.parts.classes {
            for level in 1..=self.parts.max_level {
                for weapon in [None, Some(&self.parts.starter_weapon)] {
                    match StatSheet::for_player(self, class, level, weapon) {
                        Ok(sheet) if sheet.max_hp() == 0 => {
                            v.push(format!("class {}: max HP is 0 at level {level}", class.id));
                        },
                        Ok(_) => {},
                        Err(e) => v.push(format!("class {}: level {level}: {e}", class.id)),
                    }
                }
            }
        }
    }
}

fn per_level<T: Copy>(rows: &[T], level: u32) -> Result<T, StatError> {
    level
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| rows.get(i))
        .copied()
        .ok_or(StatError::LevelOutOfRange(level))
}

#[derive(Default)]
struct Violations(Vec<RuleViolation>);

impl Violations {
    fn push(&mut self, msg: String) {
        self.0.push(RuleViolation(msg));
    }

    fn require(&mut self, ok: bool, msg: impl FnOnce() -> String) {
        if !ok {
            self.push(msg());
        }
    }
}

fn expected_len(level_count: u32) -> usize {
    usize::try_from(level_count).unwrap_or(usize::MAX)
}

fn check_levels(p: &StatRulesParts, v: &mut Violations) {
    let max = p.max_level;
    v.require((1..MAX_SUPPORTED_LEVEL).contains(&max), || {
        format!("max_level {max} must be in 1..{MAX_SUPPORTED_LEVEL}")
    });
    let n = expected_len(max);
    v.require(p.accuracy_level_add.len() == n, || {
        format!(
            "accuracy level additions cover {} levels, expected {n}",
            p.accuracy_level_add.len()
        )
    });
    v.require(p.evasion_level_add.len() == n, || {
        format!(
            "evasion level additions cover {} levels, expected {n}",
            p.evasion_level_add.len()
        )
    });
    v.require(p.death_loss.len() == n, || {
        format!("death XP loss covers {} levels, expected {n}", p.death_loss.len())
    });
    let sentinel = n.saturating_add(1);
    v.require(p.xp_to_level.len() == sentinel, || {
        format!(
            "XP table covers {} levels, expected {sentinel} (max level plus sentinel)",
            p.xp_to_level.len()
        )
    });
    v.require(p.xp_to_level.first() == Some(&0), || "XP to reach level 1 must be 0".to_owned());
    for (i, w) in p.xp_to_level.windows(2).enumerate() {
        if let [a, b] = w {
            v.require(a < b, || {
                format!(
                    "XP table not strictly increasing at level {}: {a} >= {b}",
                    i.saturating_add(2)
                )
            });
        }
    }
    for (i, f) in p.death_loss.iter().enumerate() {
        v.require((0..=Q).contains(&f.raw()), || {
            format!("death XP loss at level {} is {f}, outside 0..=1", i.saturating_add(1))
        });
    }
}

fn check_bonus(b: &StatBonusTables, v: &mut Violations) {
    for kind in StatKind::ALL {
        let rows = &b.table(kind).0;
        v.require(rows.len() == BONUS_TABLE_LEN, || {
            format!("{kind} bonus table has {} rows, expected {BONUS_TABLE_LEN}", rows.len())
        });
        for (i, r) in rows.iter().enumerate() {
            v.require(r.raw() > 0, || format!("{kind} bonus at {i} is {r}, must be > 0"));
        }
        for (i, w) in rows.windows(2).enumerate() {
            if let [a, b] = w {
                v.require(a <= b, || {
                    format!("{kind} bonus decreases at {}: {a} > {b}", i.saturating_add(1))
                });
            }
        }
    }
}

fn check_constants(c: &FormulaConstants, v: &mut Violations) {
    let mut bounded = |name: &str, value: u64, lo: u64, hi: u64| {
        v.require((lo..=hi).contains(&value), || {
            format!("constant {name} = {value} is outside {lo}..={hi}")
        });
    };
    bounded("level_mod.offset", c.level_mod_offset.into(), 0, 1000);
    bounded("level_mod.divisor", c.level_mod_divisor.into(), 1, 1_000_000);
    bounded("accuracy.dex_sqrt_multiplier", c.accuracy_dex_multiplier.into(), 1, 100);
    bounded("evasion.dex_sqrt_multiplier", c.evasion_dex_multiplier.into(), 1, 100);
    bounded("evasion.cap", c.evasion_cap.into(), 1, 100_000);
    bounded("hit_chance.base", c.hit_base.into(), 0, 1000);
    bounded("hit_chance.per_point", c.hit_per_point.into(), 0, 1000);
    bounded("hit_chance.scale", c.hit_scale.into(), 1, 1000);
    bounded("hit_chance.roll_range", c.hit_roll_range.into(), 1, 1_000_000);
    bounded("hit_chance.max_permille", c.hit_max_permille.into(), 0, c.hit_roll_range.into());
    bounded(
        "hit_chance.min_permille",
        c.hit_min_permille.into(),
        0,
        c.hit_max_permille.into(),
    );
    bounded("crit_rate.scale", c.crit_scale.into(), 1, 1000);
    bounded("crit_rate.roll_range", c.crit_roll_range.into(), 1, 1_000_000);
    bounded(
        "crit_rate.cap_permille",
        c.crit_cap_permille.into(),
        0,
        c.crit_roll_range.into(),
    );
    bounded("phys_damage.coefficient", c.damage_coefficient.into(), 1, 1000);
    bounded("phys_damage.crit_multiplier", c.crit_multiplier.into(), 1, 10);
    bounded("random_damage.fist_base", c.fist_random_base.into(), 0, 99);
    bounded("random_damage.divisor", c.random_divisor.into(), 1, 10_000);
    bounded("attack_speed.cap", c.attack_speed_cap.into(), 1, 100_000);
    bounded("attack_interval.numerator_ms", c.attack_interval_ms.into(), 1, 100_000_000);
    bounded("attack_interval.impact_divisor", c.impact_divisor.into(), 1, 100);
    bounded("damage_hate.numerator", c.hate_numerator.into(), 1, 10_000);
    bounded("damage_hate.level_offset", c.hate_level_offset.into(), 1, 10_000);
    bounded("damage_hate.cap", c.hate_cap, 1, u64::from(u32::MAX));
    bounded(
        "town_respawn.spawn_protection_seconds",
        c.spawn_protection_seconds.into(),
        0,
        86_400,
    );
    // LM_Q = (L + offset) * Q / divisor must be exact, as the plan requires.
    v.require(Q.checked_rem(i64::from(c.level_mod_divisor)) == Some(0), || {
        format!("level_mod.divisor {} does not divide Q", c.level_mod_divisor)
    });
    v.require(c.random_divisor > c.fist_random_base, || {
        "random_damage.divisor must exceed the fist radius".to_owned()
    });
    let hp = c.respawn_restore_hp.raw();
    v.require(hp > 0 && hp <= Q, || {
        format!("town_respawn.restore_hp {} must be in (0, 1]", c.respawn_restore_hp)
    });
    let mp = c.respawn_restore_mp.raw();
    v.require((0..=Q).contains(&mp), || {
        format!("town_respawn.restore_mp {} must be in [0, 1]", c.respawn_restore_mp)
    });
}

fn check_per_level(p: &StatRulesParts, v: &mut Violations) {
    for (name, rows) in [
        ("accuracy", &p.accuracy_level_add),
        ("evasion", &p.evasion_level_add),
    ] {
        for (i, r) in rows.iter().enumerate() {
            v.require(r.raw() >= 0, || {
                format!("{name} level addition at level {} is negative", i.saturating_add(1))
            });
        }
        for (i, w) in rows.windows(2).enumerate() {
            if let [a, b] = w {
                v.require(a <= b, || {
                    format!("{name} level addition decreases at level {}", i.saturating_add(2))
                });
            }
        }
    }
}

fn check_classes(p: &StatRulesParts, v: &mut Violations) {
    v.require(!p.classes.is_empty(), || "no class templates".to_owned());
    for (i, c) in p.classes.iter().enumerate() {
        if p.classes
            .iter()
            .skip(i.saturating_add(1))
            .any(|o| o.id == c.id)
        {
            v.push(format!("class id {} is defined twice", c.id));
        }
        if p.classes
            .iter()
            .skip(i.saturating_add(1))
            .any(|o| o.class_id == c.class_id)
        {
            v.push(format!("L2 class id {} is used twice", c.class_id));
        }
        v.require(c.base.is_valid(), || {
            format!("class {}: base stats must be within 1..={}", c.id, BaseStats::MAX_STAT)
        });
        v.require(c.p_def_unarmoured.raw() > 0, || format!("class {}: P.Def must be > 0", c.id));
        v.require(c.fist_attack_speed > 0, || format!("class {}: attack speed must be > 0", c.id));
        v.require(c.fist_p_atk.raw() >= 0, || format!("class {}: P.Atk must be >= 0", c.id));
    }
    for race in [
        Race::Human,
        Race::Elf,
        Race::DarkElf,
        Race::Orc,
        Race::Dwarf,
    ] {
        let fighters = p
            .classes
            .iter()
            .filter(|c| c.race == race && c.archetype == Archetype::Fighter)
            .count();
        v.require(fighters == 1, || {
            format!("race {} needs exactly one fighter template, found {fighters}", race.as_str())
        });
    }
}

fn check_weapon(w: &WeaponBlock, c: &FormulaConstants, v: &mut Violations) {
    v.require(w.attack_speed > 0, || format!("weapon {}: attack speed must be > 0", w.id));
    v.require(w.p_atk.raw() >= 0, || format!("weapon {}: P.Atk must be >= 0", w.id));
    v.require(w.accuracy.raw() >= 0, || format!("weapon {}: accuracy must be >= 0", w.id));
    v.require(w.random_damage < c.random_divisor, || {
        format!(
            "weapon {}: random damage {} must be below the divisor {}",
            w.id, w.random_damage, c.random_divisor
        )
    });
}
