//! Unit and property tests for the stat engine (plan §3.1 worked vectors, Story E1.3).
//! Rules here are a small synthetic fixture with the HF formula constants; the HF tables
//! themselves are tested where they are loaded (`infrastructure::rules_data`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use proptest::prelude::*;

use super::*;
use crate::domain::{BaseStats, Race};

const fn q(raw: i64) -> Scaled {
    Scaled::from_raw(raw)
}

/// The HF constants as `tables/formulas.toml` resolves them.
fn hf_constants() -> FormulaConstants {
    FormulaConstants {
        level_mod_offset: 89,
        level_mod_divisor: 100,
        accuracy_dex_multiplier: 6,
        evasion_dex_multiplier: 6,
        evasion_cap: 250,
        hit_base: 80,
        hit_per_point: 2,
        hit_scale: 10,
        hit_min_permille: 200,
        hit_max_permille: 980,
        hit_roll_range: 1000,
        crit_scale: 10,
        crit_cap_permille: 500,
        crit_roll_range: 1000,
        damage_coefficient: 76,
        crit_multiplier: 2,
        fist_random_base: 5,
        random_divisor: 100,
        attack_speed_cap: 1500,
        attack_interval_ms: 500_000,
        impact_divisor: 2,
        hate_numerator: 100,
        hate_level_offset: 7,
        hate_cap: 999_999_999,
        respawn_restore_hp: q(650_000),
        respawn_restore_mp: q(0),
        spawn_protection_seconds: 600,
    }
}

const HUMAN_FIGHTER: BaseStats = BaseStats {
    str: 40,
    dex: 30,
    con: 43,
    int: 21,
    wit: 11,
    men: 25,
};

fn fighter(id: &str, class_id: u32, race: Race) -> ClassTemplate {
    ClassTemplate {
        id: id.to_owned(),
        class_id,
        name: id.to_owned(),
        race,
        archetype: Archetype::Fighter,
        base: HUMAN_FIGHTER,
        fist_p_atk: q(4_000_000),
        fist_crit_rate: 4,
        fist_attack_speed: 300,
        p_def_unarmoured: q(80_000_000),
        hp: Quadratic {
            base: q(80_000_000),
            per_level: q(11_765_000),
            accel: q(65_000),
        },
        mp: Quadratic {
            base: q(30_000_000),
            per_level: q(5_430_000),
            accel: q(30_000),
        },
    }
}

/// A table whose row `i` is `start + i * step` (in raw `1/Q`).
fn linear(start: i64, step: i64) -> BonusTable {
    BonusTable::new((0..100).map(|i| q(start + i * step)).collect())
}

/// Synthetic rules: three levels, HF XP thresholds and loss for levels 1-3, the HF table
/// values at the indices the vectors use (STR 40 = 1.20, DEX 30 = 1.10, CON 43 = 1.58,
/// MEN 25 = 1.28), and one fighter per race.
fn parts() -> StatRulesParts {
    let mut str_rows: Vec<Scaled> = (0..100).map(|i| q(200_000 + i * 25_000)).collect();
    str_rows[40] = q(1_200_000);
    let mut dex_rows: Vec<Scaled> = (0..100).map(|i| q(800_000 + i * 10_000)).collect();
    dex_rows[30] = q(1_100_000);
    let mut con_rows: Vec<Scaled> = (0..100).map(|i| q(400_000 + i * 27_000)).collect();
    con_rows[43] = q(1_580_000);
    let mut men_rows: Vec<Scaled> = (0..100).map(|i| q(1_000_000 + i * 11_000)).collect();
    men_rows[25] = q(1_280_000);
    StatRulesParts {
        max_level: 3,
        bonus: StatBonusTables {
            str: BonusTable::new(str_rows),
            dex: BonusTable::new(dex_rows),
            con: BonusTable::new(con_rows),
            int: linear(500_000, 10_000),
            wit: linear(400_000, 50_000),
            men: BonusTable::new(men_rows),
        },
        constants: hf_constants(),
        accuracy_level_add: vec![Scaled::ZERO; 3],
        evasion_level_add: vec![Scaled::ZERO; 3],
        xp_to_level: vec![0, 68, 363, 1168],
        death_loss: vec![q(100_000), q(98_750), q(97_500)],
        classes: vec![
            fighter("dark_fighter", 31, Race::DarkElf),
            fighter("dwarven_fighter", 53, Race::Dwarf),
            fighter("elven_fighter", 18, Race::Elf),
            fighter("human_fighter", 0, Race::Human),
            fighter("orc_fighter", 44, Race::Orc),
        ],
        starter_weapon: WeaponBlock {
            id: "squires_sword".to_owned(),
            p_atk: q(6_000_000),
            crit_rate: 8,
            attack_speed: 379,
            random_damage: 10,
            accuracy: Scaled::ZERO,
            attack_range_l2: 40,
        },
    }
}

fn rules() -> StatRules {
    StatRules::new(parts()).unwrap()
}

fn npc(p_atk: i64, p_def: i64, level: u32) -> StatSheet {
    StatSheet::from_final(FinalStats {
        level,
        max_hp: 100,
        max_mp: 0,
        p_atk: Scaled::from_int(p_atk).unwrap(),
        p_def: Scaled::from_int(p_def).unwrap(),
        accuracy: Scaled::ZERO,
        evasion: Scaled::ZERO,
        crit_permille: 0,
        attack_speed: Scaled::from_int(300).unwrap(),
        random_damage: 10,
    })
    .unwrap()
}

// ---------------------------------------------------------------------------
// Fixed-point primitives
// ---------------------------------------------------------------------------

#[test]
fn floor_and_ceil_division_are_mathematical_for_negative_numerators() {
    assert_eq!(floor_div(-7, 2).unwrap(), -4);
    assert_eq!(floor_div(7, 2).unwrap(), 3);
    assert_eq!(ceil_div(7, 2).unwrap(), 4);
    assert_eq!(ceil_div(-7, 2).unwrap(), -3);
    assert_eq!(floor_div(1, 0), Err(StatError::NonPositiveDivisor));
    assert_eq!(floor_div(1, -1), Err(StatError::NonPositiveDivisor));
    assert_eq!(Scaled::from_raw(-500_000).floor_units(), -1);
}

#[test]
fn scaled_displays_exact_decimals() {
    assert_eq!(q(15_317_197).to_string(), "15.317197");
    assert_eq!(q(-1).to_string(), "-0.000001");
    assert_eq!(Scaled::from_int(i64::MAX), Err(StatError::Overflow));
}

#[test]
fn sqrt_of_dex_floors_to_one_millionth() {
    // sqrt(30) = 5.4772255750...
    assert_eq!(sqrt_dex(30).unwrap(), q(5_477_225));
    assert_eq!(sqrt_dex(25).unwrap(), q(5_000_000));
    assert_eq!(sqrt_dex(0).unwrap(), Scaled::ZERO);
}

// ---------------------------------------------------------------------------
// Worked vector 1: level modifier, P.Def, P.Atk
// ---------------------------------------------------------------------------

#[test]
fn vector_1_level_mod_and_unarmoured_p_def() {
    let c = hf_constants();
    let lm20 = level_mod(&c, 20).unwrap();
    assert_eq!(lm20, q(1_090_000));
    assert_eq!(level_mod(&c, 1).unwrap(), q(900_000));
    assert_eq!(p_def(q(80_000_000), lm20).unwrap(), q(87_200_000));
}

#[test]
fn vector_1_hf_p_atk_with_the_starter_weapon_replacing_the_fist() {
    let lm1 = level_mod(&hf_constants(), 1).unwrap();
    // HF Human Fighter STR 40 = 1.20. Squire's Sword P.Atk 6 replaces fist 4 (L2J <set>):
    // F(6000000 * 1200000 * 900000 / 10^12) = 6480000.
    assert_eq!(p_atk(q(6_000_000), q(1_200_000), lm1).unwrap(), q(6_480_000));
    assert_eq!(p_atk(q(4_000_000), q(1_200_000), lm1).unwrap(), q(4_320_000));
}

#[test]
fn legacy_vector_1_docs_ertheia_str_88_is_arithmetic_only() {
    // The docs' "HF STR 88 -> 1.009^(s-49)" row is Ertheia (l2j-mobius) data, not HF
    // (SOURCES.md errata E-1). Kept as a pure arithmetic check of the P.Atk rounding.
    let lm1 = level_mod(&hf_constants(), 1).unwrap();
    assert_eq!(p_atk(q(12_000_000), q(1_418_259), lm1).unwrap(), q(15_317_197));
}

#[test]
fn derived_human_fighter_sheet_at_level_1() {
    let r = rules();
    let class = r.fighter_for(Race::Human).unwrap();
    let bare = StatSheet::for_player(&r, class, 1, None).unwrap();
    assert_eq!(bare.p_atk(), q(4_320_000));
    assert_eq!(bare.p_def(), q(72_000_000));
    assert_eq!(bare.max_hp(), 126); // F(80 * 1.58)
    assert_eq!(bare.max_mp(), 38); // F(30 * 1.28)
    assert_eq!(bare.accuracy(), q(33_863_350)); // 6 * 5.477225 + 1
    assert_eq!(bare.evasion(), q(33_863_350));
    assert_eq!(bare.crit_permille(), 44);
    assert_eq!(bare.attack_speed(), q(330_000_000));
    assert_eq!(bare.random_damage(), 6); // fist: 5 + isqrt(1)

    let armed = StatSheet::for_player(&r, class, 1, Some(r.starter_weapon())).unwrap();
    assert_eq!(armed.p_atk(), q(6_480_000));
    assert_eq!(armed.crit_permille(), 88); // F(8 * 1.10 * 10)
    assert_eq!(armed.attack_speed(), q(416_900_000)); // F(379 * 1.10)
    assert_eq!(armed.random_damage(), 10);
}

#[test]
fn a_level_outside_the_tables_is_rejected() {
    let r = rules();
    let class = r.fighter_for(Race::Human).unwrap();
    assert_eq!(StatSheet::for_player(&r, class, 0, None), Err(StatError::LevelOutOfRange(0)));
    assert_eq!(StatSheet::for_player(&r, class, 4, None), Err(StatError::LevelOutOfRange(4)));
}

// ---------------------------------------------------------------------------
// Worked vector 2: HP / MP
// ---------------------------------------------------------------------------

#[test]
fn vector_2_human_fighter_level_40_resources() {
    let class = fighter("human_fighter", 0, Race::Human);
    let hp40 = class.hp.at(40).unwrap();
    let mp40 = class.mp.at(40).unwrap();
    assert_eq!(hp40, 637_700_000); // 80 + 11.765*39 + 0.065*39²
    assert_eq!(mp40, 287_400_000); // 30 + 5.430*39 + 0.030*39²
                                   // The docs' fixture bonuses (arithmetic only, not an HF profile).
    assert_eq!(resource_max(hp40, q(1_570_000)).unwrap(), 1001);
    assert_eq!(resource_max(mp40, q(1_280_000)).unwrap(), 367);
    // HF table CON 43 = 1.58 (the docs print 1.57: errata E-4).
    assert_eq!(resource_max(hp40, q(1_580_000)).unwrap(), 1007);
}

// ---------------------------------------------------------------------------
// Worked vector 3: hit and crit
// ---------------------------------------------------------------------------

#[test]
fn vector_3_hit_chance_and_its_inclusive_comparator() {
    let c = hf_constants();
    let chance = hit_chance_permille(&c, q(40_000_000), q(35_000_000)).unwrap();
    assert_eq!(chance, 900);
    assert!(hit_lands(&c, chance, 900).unwrap());
    assert!(!hit_lands(&c, chance, 901).unwrap());
    // Equal accuracy and evasion: nominal 80 % is 801 outcomes of 1000.
    let even = hit_chance_permille(&c, q(10_000_000), q(10_000_000)).unwrap();
    assert_eq!(even, 800);
    assert_eq!(
        (0..1000)
            .filter(|&r| hit_lands(&c, even, r).unwrap())
            .count(),
        801
    );
}

#[test]
fn vector_3_hit_chance_clamps() {
    let c = hf_constants();
    assert_eq!(hit_chance_permille(&c, q(0), q(31_000_000)).unwrap(), 200);
    assert_eq!(hit_chance_permille(&c, q(10_000_000), q(0)).unwrap(), 980);
    assert_eq!(hit_lands(&c, 980, 1000), Err(StatError::DrawOutOfRange(1000)));
}

#[test]
fn hit_chance_floors_fractional_differences() {
    let c = hf_constants();
    // diff -0.05: 800 - 1 = 799 exactly; diff -0.051: F(798.98) = 798.
    assert_eq!(hit_chance_permille(&c, q(0), q(50_000)).unwrap(), 799);
    assert_eq!(hit_chance_permille(&c, q(0), q(51_000)).unwrap(), 798);
}

#[test]
fn vector_3_hf_crit_and_its_strict_comparator() {
    let c = hf_constants();
    let crit = crit_permille(&c, 4, q(1_100_000)).unwrap();
    assert_eq!(crit, 44);
    assert!(crit_lands(&c, crit, 43).unwrap());
    assert!(!crit_lands(&c, crit, 44).unwrap());
    assert_eq!(crit_permille(&c, 4, q(100_000_000)).unwrap(), 500);
}

#[test]
fn legacy_vector_3_interlude_base_44_crit_is_48() {
    // Interlude: baseCrit 44 * DEX bonus, no x10 (labelled legacy input only).
    let legacy = FormulaConstants {
        crit_scale: 1,
        ..hf_constants()
    };
    assert_eq!(crit_permille(&legacy, 44, q(1_100_000)).unwrap(), 48);
}

// ---------------------------------------------------------------------------
// Worked vector 4: physical damage and hate
// ---------------------------------------------------------------------------

#[test]
fn vector_4_damage_with_k_76() {
    let c = hf_constants();
    let a = npc(100, 1, 1);
    let t = npc(1, 50, 20);
    assert_eq!(physical_damage(&c, &a, &t, false, 0).unwrap(), 152);
    assert_eq!(physical_damage(&c, &a, &t, true, 0).unwrap(), 304);
    assert_eq!(physical_damage(&c, &a, &t, false, -10).unwrap(), 136); // F(136.8)
    assert_eq!(physical_damage(&c, &a, &t, false, 10).unwrap(), 167); // F(167.2)
    assert_eq!(damage_hate(&c, 152, 20).unwrap(), 562);
}

#[test]
fn damage_is_at_least_one_and_spread_is_bounded_by_the_radius() {
    let c = hf_constants();
    let weak = npc(0, 1, 1);
    let wall = npc(1, 1_000_000, 1);
    assert_eq!(physical_damage(&c, &weak, &wall, false, -10).unwrap(), 1);
    assert_eq!(physical_damage(&c, &weak, &wall, false, 11), Err(StatError::DrawOutOfRange(11)));
    assert_eq!(
        physical_damage(&c, &weak, &wall, false, -11),
        Err(StatError::DrawOutOfRange(-11))
    );
}

#[test]
fn hate_accumulates_up_to_its_cap() {
    let c = hf_constants();
    assert_eq!(add_hate(&c, 999_999_000, 5_000), 999_999_999);
    assert_eq!(add_hate(&c, 1, 1), 2);
    assert_eq!(damage_hate(&c, 0, 1).unwrap(), 0);
}

// ---------------------------------------------------------------------------
// Worked vector 5: attack timing
// ---------------------------------------------------------------------------

#[test]
fn vector_5_speed_300_and_cap_1500() {
    let c = hf_constants();
    let t300 = attack_timing(&c, Scaled::from_int(300).unwrap()).unwrap();
    assert_eq!((t300.impact_ticks, t300.cycle_ticks), (9, 17));
    let (n, d) = t300.interval_ms;
    assert_eq!((n * 3, d * 3), (1_500_000_000_000, 900_000_000)); // 1666 2/3 ms
    assert_eq!(n * 3 / d, 5000);
    let t1500 = attack_timing(&c, Scaled::from_int(1500).unwrap()).unwrap();
    assert_eq!((t1500.impact_ticks, t1500.cycle_ticks), (2, 4));
}

#[test]
fn starter_weapon_timing_uses_the_exact_fraction() {
    // 379 * 1.10 = 416.9: interval 1199.33 ms, impact C(5.997) = 6, next C(11.99) = 12.
    let t = attack_timing(&hf_constants(), q(416_900_000)).unwrap();
    assert_eq!((t.impact_ticks, t.cycle_ticks), (6, 12));
}

#[test]
fn attack_speed_is_capped_and_never_zero() {
    let c = hf_constants();
    assert_eq!(attack_speed(&c, 2000, q(1_000_000)).unwrap(), Scaled::from_int(1500).unwrap());
    assert_eq!(attack_speed(&c, 0, q(1_000_000)), Err(StatError::NonPositiveDivisor));
    assert_eq!(attack_timing(&c, Scaled::ZERO), Err(StatError::NonPositiveDivisor));
}

// ---------------------------------------------------------------------------
// Worked vector 6: XP, death, respawn
// ---------------------------------------------------------------------------

#[test]
fn vector_6_xp_level_and_death_loss() {
    let r = rules();
    let xp = add_xp(&r, 60, 10).unwrap();
    assert_eq!(xp, 70);
    assert_eq!(level_for_xp(&r, xp), 2);
    assert_eq!(death_xp_loss(&r, 2).unwrap(), 29); // F(295 * 0.09875)
    let after = xp_after_death(&r, xp, 2).unwrap();
    assert_eq!(after, 41);
    assert_eq!(level_for_xp(&r, after), 1);
}

#[test]
fn xp_is_capped_one_below_the_sentinel() {
    let r = rules();
    assert_eq!(xp_cap(&r).unwrap(), 1167);
    assert_eq!(add_xp(&r, 1000, 10_000).unwrap(), 1167);
    assert_eq!(level_for_xp(&r, 1167), 3);
    assert_eq!(level_for_xp(&r, u64::MAX), 3);
    // The max level's loss spans to the sentinel: F((1168 - 363) * 0.0975) = 78.
    assert_eq!(death_xp_loss(&r, 3).unwrap(), 78);
    assert_eq!(xp_after_death(&r, 10, 3).unwrap(), 0);
    assert_eq!(death_xp_loss(&r, 4), Err(StatError::LevelOutOfRange(4)));
}

#[test]
fn vector_6_town_respawn_and_protection() {
    let c = hf_constants();
    let sheet = StatSheet::from_final(FinalStats {
        max_hp: 126,
        max_mp: 38,
        ..final_template()
    })
    .unwrap();
    assert_eq!(town_respawn_vitals(&c, &sheet).unwrap(), (81, 0));
    let one = StatSheet::from_final(FinalStats {
        max_hp: 1,
        ..final_template()
    })
    .unwrap();
    assert_eq!(town_respawn_vitals(&c, &one).unwrap(), (1, 0));
    // HF PlayerSpawnProtection = 600 seconds (errata E-6).
    assert_eq!(spawn_protection_ticks(&c).unwrap(), 6000);
}

#[test]
fn npc_respawn_adds_delay_and_inclusive_jitter_in_ticks() {
    assert_eq!(npc_respawn_tick(Tick(100), 30, 5, 0).unwrap(), Tick(400));
    assert_eq!(npc_respawn_tick(Tick(100), 30, 5, 5).unwrap(), Tick(450));
    assert_eq!(npc_respawn_tick(Tick(100), 30, 5, 6), Err(StatError::DrawOutOfRange(6)));
    assert_eq!(npc_respawn_tick(Tick(u64::MAX), 1, 0, 0), Err(StatError::Overflow));
}

fn final_template() -> FinalStats {
    FinalStats {
        level: 1,
        max_hp: 10,
        max_mp: 0,
        p_atk: Scaled::ONE,
        p_def: Scaled::ONE,
        accuracy: Scaled::ZERO,
        evasion: Scaled::ZERO,
        crit_permille: 0,
        attack_speed: Scaled::ONE,
        random_damage: 0,
    }
}

#[test]
fn final_stats_reject_zero_defence_speed_and_level() {
    let zero_def = FinalStats {
        p_def: Scaled::ZERO,
        ..final_template()
    };
    assert_eq!(StatSheet::from_final(zero_def), Err(StatError::NonPositiveDivisor));
    let zero_speed = FinalStats {
        attack_speed: Scaled::ZERO,
        ..final_template()
    };
    assert_eq!(StatSheet::from_final(zero_speed), Err(StatError::NonPositiveDivisor));
    let zero_level = FinalStats {
        level: 0,
        ..final_template()
    };
    assert_eq!(StatSheet::from_final(zero_level), Err(StatError::LevelOutOfRange(0)));
}

// ---------------------------------------------------------------------------
// Rule validation
// ---------------------------------------------------------------------------

#[test]
fn every_violation_is_reported_at_once() {
    let mut p = parts();
    p.xp_to_level = vec![0, 68, 68, 1168];
    p.constants.level_mod_divisor = 7;
    p.constants.hit_min_permille = 990;
    p.classes.pop();
    p.death_loss.push(q(1));
    let errs = StatRules::new(p).unwrap_err();
    let text: Vec<String> = errs.iter().map(ToString::to_string).collect();
    assert!(text.iter().any(|e| e.contains("not strictly increasing")), "{text:?}");
    assert!(text.iter().any(|e| e.contains("does not divide Q")), "{text:?}");
    assert!(text.iter().any(|e| e.contains("hit_chance.min_permille")), "{text:?}");
    assert!(text.iter().any(|e| e.contains("race orc")), "{text:?}");
    assert!(text.iter().any(|e| e.contains("death XP loss covers 4")), "{text:?}");
}

#[test]
fn rules_reject_holes_bounds_and_zero_divisors() {
    type Mutation = Box<dyn Fn(&mut StatRulesParts)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "bonus table has 99 rows",
            Box::new(|p| p.bonus.str = BonusTable::new(vec![Scaled::ONE; 99])),
        ),
        ("must be > 0", Box::new(|p| p.bonus.dex = linear(0, 0))),
        ("bonus decreases", Box::new(|p| p.bonus.con = linear(2_000_000, -1))),
        ("phys_damage.coefficient", Box::new(|p| p.constants.damage_coefficient = 0)),
        ("crit_rate.cap_permille", Box::new(|p| p.constants.crit_cap_permille = 1001)),
        ("P.Def must be > 0", Box::new(|p| p.classes[0].p_def_unarmoured = Scaled::ZERO)),
        ("attack speed must be > 0", Box::new(|p| p.starter_weapon.attack_speed = 0)),
        ("XP to reach level 1", Box::new(|p| p.xp_to_level[0] = 1)),
        (
            "expected 4 (max level plus sentinel)",
            Box::new(|p| {
                p.xp_to_level.pop();
            }),
        ),
        ("outside 0..=1", Box::new(|p| p.death_loss[0] = q(1_000_001))),
        ("defined twice", Box::new(|p| p.classes[1].id = "dark_fighter".to_owned())),
        ("base stats", Box::new(|p| p.classes[0].base.str = 100)),
        ("restore_hp", Box::new(|p| p.constants.respawn_restore_hp = Scaled::ZERO)),
        ("max HP is 0", Box::new(|p| p.classes[0].hp.base = Scaled::ZERO)),
        ("overflow", Box::new(|p| p.classes[0].hp.base = q(i64::MAX))),
    ];
    for (needle, mutate) in cases {
        let mut p = parts();
        mutate(&mut p);
        let errs = StatRules::new(p).unwrap_err();
        assert!(errs.iter().any(|e| e.0.contains(needle)), "expected {needle:?} in {errs:?}");
    }
}

#[test]
fn bonus_lookup_outside_the_table_is_an_error() {
    let r = rules();
    assert_eq!(r.bonus().bonus(StatKind::Str, 99).unwrap(), q(200_000 + 99 * 25_000));
    assert_eq!(r.bonus().bonus(StatKind::Str, 100), Err(StatError::StatOutOfRange(100)));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn isqrt_is_the_floor_square_root(n in any::<u64>()) {
        let n = u128::from(n);
        let r = isqrt(n);
        prop_assert!(r * r <= n && (r + 1) * (r + 1) > n);
    }

    #[test]
    fn floor_div_matches_the_rational_floor(n in any::<i64>(), d in 1_i64..1_000_000) {
        let f = floor_div(n.into(), d.into()).unwrap();
        prop_assert!(f * i128::from(d) <= i128::from(n));
        prop_assert!((f + 1) * i128::from(d) > i128::from(n));
    }

    #[test]
    fn hit_chance_is_clamped_and_monotone_in_accuracy(
        acc in -1_000_000_000_i64..1_000_000_000,
        eva in -1_000_000_000_i64..1_000_000_000,
        bump in 0_i64..1_000_000,
    ) {
        let c = hf_constants();
        let lo = hit_chance_permille(&c, q(acc), q(eva)).unwrap();
        let hi = hit_chance_permille(&c, q(acc + bump), q(eva)).unwrap();
        prop_assert!((200..=980).contains(&lo));
        prop_assert!(lo <= hi);
    }

    #[test]
    fn crit_and_speed_respect_their_caps(base in 0_u32..10_000, dex in 1_i64..100_000_000) {
        let c = hf_constants();
        prop_assert!(crit_permille(&c, base, q(dex)).unwrap() <= 500);
        if base > 0 && i64::from(base) * dex >= 1_000_000 {
            let s = attack_speed(&c, base, q(dex)).unwrap();
            prop_assert!(s.raw() > 0 && s <= Scaled::from_int(1500).unwrap());
            let t = attack_timing(&c, s).unwrap();
            prop_assert!(t.impact_ticks >= 1 && t.impact_ticks <= t.cycle_ticks);
        }
    }

    #[test]
    fn damage_is_positive_or_an_overflow_error_never_a_panic(
        atk in 0_i64..i64::MAX,
        def in 1_i64..i64::MAX,
        spread in -10_i64..=10,
        crit in any::<bool>(),
    ) {
        let c = hf_constants();
        let a = StatSheet::from_final(FinalStats { p_atk: q(atk), random_damage: 10, ..final_template() }).unwrap();
        let t = StatSheet::from_final(FinalStats { p_def: q(def), ..final_template() }).unwrap();
        match physical_damage(&c, &a, &t, crit, spread) {
            Ok(d) => prop_assert!(d >= 1),
            Err(e) => prop_assert_eq!(e, StatError::Overflow),
        }
    }

    #[test]
    fn xp_level_is_monotone_and_death_never_raises_xp(xp in 0_u64..2_000, reward in 0_u64..2_000) {
        let r = rules();
        let before = level_for_xp(&r, xp);
        let gained = add_xp(&r, xp, reward).unwrap();
        prop_assert!(gained <= xp_cap(&r).unwrap());
        prop_assert!(level_for_xp(&r, gained) >= before.min(level_for_xp(&r, xp_cap(&r).unwrap())));
        let after = xp_after_death(&r, gained, level_for_xp(&r, gained)).unwrap();
        prop_assert!(after <= gained);
        prop_assert!(level_for_xp(&r, after) <= level_for_xp(&r, gained));
    }

    #[test]
    fn every_legal_player_sheet_has_positive_resources(level in 1_u32..=3, armed in any::<bool>()) {
        let r = rules();
        for class in r.classes() {
            let weapon = armed.then(|| r.starter_weapon());
            let s = StatSheet::for_player(&r, class, level, weapon).unwrap();
            prop_assert!(s.max_hp() >= 1 && s.p_def().raw() > 0 && s.attack_speed().raw() > 0);
            prop_assert!(s.evasion() <= Scaled::from_int(250).unwrap());
        }
    }
}
