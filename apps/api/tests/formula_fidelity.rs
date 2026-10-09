//! E1.4: independent Python Fraction oracle, consumed without executing Python.
//! Regenerate/check with `python3 packages/data/scripts/oracle.py [--check]`.
//! Expected results come from source literals, never the Rust implementation.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::sync::OnceLock;

use nightfall_api::domain::zone::*;
use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource, EMBEDDED};
use proptest::prelude::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Oracle {
    schema: u32,
    grid: Vec<usize>,
    bonus: Vec<Vec<i64>>,
    // LM_Q, accuracy addition Q, evasion addition Q, rounded death loss.
    levels: Vec<[i64; 4]>,
    dex: Vec<Vec<[i64; 2]>>,
    // Armed then fist: speed_Q, crit, integer ms, impact ticks, cycle ticks.
    speed: Vec<[i64; 10]>,
    classes: Vec<Profile>,
    xp: Vec<u64>,
    xp_cases: Vec<[u64; 2]>,
    death_cases: Vec<[u64; 4]>,
    hit_cases: Vec<[i64; 3]>,
    damage: Vec<[i64; 6]>,
    hate: Vec<[u64; 3]>,
    speed_cases: Vec<[i64; 6]>,
    crit_cases: Vec<[i64; 3]>,
    hit_by_difference: Vec<u32>,
    legacy: BTreeMap<String, i64>,
    inputs_sha256: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Profile {
    id: String,
    base: [u32; 6],
    rows: Vec<Resources>,
}

#[derive(Deserialize)]
struct Resources {
    hp_curve: i128,
    mp_curve: i128,
    hp: Vec<u32>,
    mp: u32,
    p_def: i64,
    p_atk: Vec<i64>,
    fist_p_atk: Vec<i64>,
}

fn oracle() -> &'static Oracle {
    static ORACLE: OnceLock<Oracle> = OnceLock::new();
    ORACLE.get_or_init(|| {
        serde_json::from_str(include_str!("../../../packages/data/fixtures/oracle.json")).unwrap()
    })
}

fn rules() -> &'static StatRules {
    static RULES: OnceLock<StatRules> = OnceLock::new();
    RULES.get_or_init(|| (*load_rules(&RulesSource::embedded()).unwrap().rules).clone())
}

fn u32_of(value: impl TryInto<u32>) -> u32 {
    value.try_into().ok().unwrap()
}

fn i64_of(value: impl TryInto<i64>) -> i64 {
    value.try_into().ok().unwrap()
}

fn check_sheet(
    ci: usize,
    li: usize,
    dex: usize,
    str_: usize,
    con: usize,
    armed: bool,
) -> StatSheet {
    let o = oracle();
    let r = rules();
    let p = &o.classes[ci];
    let mut class = r.class(&p.id).unwrap().clone();
    class.base.dex = u32_of(dex);
    class.base.str = u32_of(str_);
    class.base.con = u32_of(con);
    let s = StatSheet::for_player(r, &class, u32_of(li + 1), armed.then(|| r.starter_weapon()))
        .unwrap();
    let row = &p.rows[li];
    let offset = if armed { 0 } else { 5 };
    let speed = &o.speed[dex];
    let timing = attack_timing(r.constants(), s.attack_speed()).unwrap();
    let actual = [
        i64::from(s.max_hp()),
        i64::from(s.max_mp()),
        s.p_atk().raw(),
        s.p_def().raw(),
        s.accuracy().raw(),
        s.evasion().raw(),
        s.attack_speed().raw(),
        i64::from(s.crit_permille()),
        i64_of(timing.interval_ms.0),
        i64_of(timing.impact_ticks),
        i64_of(timing.cycle_ticks),
    ];
    let expected = [
        i64::from(row.hp[con]),
        i64::from(row.mp),
        if armed {
            row.p_atk[str_]
        } else {
            row.fist_p_atk[str_]
        },
        row.p_def,
        o.dex[li][dex][0],
        o.dex[li][dex][1],
        speed[offset],
        speed[offset + 1],
        speed[offset + 2],
        speed[offset + 3],
        speed[offset + 4],
    ];
    assert_eq!(
        actual,
        expected,
        "{} L{} DEX/STR/CON={dex}/{str_}/{con}, armed={armed}",
        p.id,
        li + 1
    );
    assert_eq!(timing.interval_ms.1, 1);
    s
}

#[test]
fn golden_is_bound_to_every_input_and_the_independent_generator() {
    let o = oracle();
    assert_eq!(o.schema, 1);
    let mut inputs = EMBEDDED.to_vec();
    inputs.push(("npcs/keltir.toml", include_str!("../../../packages/data/npcs/keltir.toml")));
    inputs.push(("scripts/oracle.py", include_str!("../../../packages/data/scripts/oracle.py")));
    assert_eq!(inputs.len(), o.inputs_sha256.len());
    for (name, source) in inputs {
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            o.inputs_sha256[name],
            "{name}: regenerate oracle.py, then review the golden diff"
        );
    }
}

#[test]
fn all_six_bonus_tables_and_all_level_additions_match_literal_source_oracle() {
    let r = rules();
    for (ki, kind) in [
        StatKind::Str,
        StatKind::Dex,
        StatKind::Con,
        StatKind::Int,
        StatKind::Wit,
        StatKind::Men,
    ]
    .into_iter()
    .enumerate()
    {
        for index in 0..100 {
            assert_eq!(
                r.bonus().bonus(kind, u32_of(index)).unwrap().raw(),
                oracle().bonus[ki][index]
            );
        }
        assert!(r.bonus().bonus(kind, 100).is_err());
    }
    for (li, [lm, acc, eva, loss]) in oracle().levels.iter().copied().enumerate() {
        let level = u32_of(li + 1);
        assert_eq!(level_mod(r.constants(), level).unwrap().raw(), lm);
        assert_eq!(r.accuracy_level_add(level).unwrap().raw(), acc);
        assert_eq!(r.evasion_level_add(level).unwrap().raw(), eva);
        assert_eq!(i64_of(death_xp_loss(r, level).unwrap()), loss);
    }
}

#[test]
fn all_nine_profiles_all_85_levels_and_full_cartesian_attribute_grid() {
    let o = oracle();
    assert_eq!(o.classes.len(), 9);
    for (ci, profile) in o.classes.iter().enumerate() {
        let class = rules().class(&profile.id).unwrap();
        let b = &class.base;
        assert_eq!([b.str, b.dex, b.con, b.int, b.wit, b.men], profile.base);
        assert_eq!((class.fist_attack_speed, class.fist_crit_rate), (300, 4));
        assert_eq!(profile.rows.len(), 85);
        for (li, row) in profile.rows.iter().enumerate() {
            assert_eq!(class.hp.at(u32_of(li + 1)).unwrap(), row.hp_curve);
            assert_eq!(class.mp.at(u32_of(li + 1)).unwrap(), row.mp_curve);
            for armed in [false, true] {
                check_sheet(ci, li, b.dex as usize, b.str as usize, b.con as usize, armed);
                for &dex in &o.grid {
                    for &str_ in &o.grid {
                        for &con in &o.grid {
                            check_sheet(ci, li, dex, str_, con, armed);
                        }
                    }
                }
            }
            // Exhaust every axis, including values absent from the Cartesian grid.
            for index in 1..100 {
                check_sheet(ci, li, index, index, index, true);
            }
        }
    }
}

#[test]
fn oracle_xp_thresholds_sentinel_and_each_death_boundary() {
    let o = oracle();
    let r = rules();
    for (li, &xp) in o.xp.iter().enumerate() {
        assert_eq!(r.xp_to_level(u32_of(li + 1)).unwrap(), xp);
    }
    for &[xp, level] in &o.xp_cases {
        assert_eq!(level_for_xp(r, xp), u32_of(level));
    }
    assert_eq!(xp_cap(r).unwrap(), o.xp[85] - 1);
    assert_eq!(add_xp(r, u64::MAX, u64::MAX).unwrap(), o.xp[85] - 1);
    for &[level, xp, after, after_level] in &o.death_cases {
        let actual = xp_after_death(r, xp, u32_of(level)).unwrap();
        assert_eq!(actual, after, "L{level}, XP {xp}");
        assert_eq!(level_for_xp(r, actual), u32_of(after_level));
    }
    // Old floor left XP=2884 (L5); source rounding leaves 2883 (L4).
    assert_eq!(xp_after_death(r, 3183, 5).unwrap(), 2883);
}

#[test]
fn oracle_hit_clamps_and_every_roll_at_fractional_rounding_boundaries() {
    let c = rules().constants();
    for &[a, e, chance] in &oracle().hit_cases {
        let actual = hit_chance_permille(c, Scaled::from_raw(a), Scaled::from_raw(e)).unwrap();
        assert_eq!(actual, u32_of(chance));
        for roll in 0..1000 {
            assert_eq!(hit_lands(c, actual, roll).unwrap(), i64::from(roll) <= chance);
        }
    }
    for speed in &oracle().speed {
        for crit in [speed[1], speed[6]] {
            for roll in 0..1000 {
                assert_eq!(crit_lands(c, u32_of(crit), roll).unwrap(), i64::from(roll) < crit);
            }
        }
    }
}

fn keltir_target() -> StatSheet {
    StatSheet::from_final(FinalStats {
        level: 1,
        max_hp: 44,
        max_mp: 0,
        p_atk: Scaled::from_raw(8_800_000),
        p_def: Scaled::from_raw(26_900_000),
        accuracy: Scaled::ZERO,
        evasion: Scaled::ZERO,
        crit_permille: 0,
        attack_speed: Scaled::from_int(300).unwrap(),
        random_damage: 0,
    })
    .unwrap()
}

#[test]
fn every_profile_damage_spread_crit_and_hate_match_oracle() {
    let o = oracle();
    let r = rules();
    let target = keltir_target();
    for &[ci, level, crit, spread, damage, hate] in &o.damage {
        let class = r
            .class(&o.classes[usize::try_from(ci).unwrap()].id)
            .unwrap();
        let sheet =
            StatSheet::for_player(r, class, u32_of(level), Some(r.starter_weapon())).unwrap();
        let actual = physical_damage(r.constants(), &sheet, &target, crit == 1, spread).unwrap();
        assert_eq!(i64::from(actual), damage);
        assert_eq!(i64_of(damage_hate(r.constants(), actual, 1).unwrap()), hate);
    }
    for &[damage, level, hate] in &o.hate {
        assert_eq!(damage_hate(r.constants(), u32_of(damage), u32_of(level)).unwrap(), hate);
        assert_eq!(
            add_hate(r.constants(), 999_999_998, hate),
            (999_999_998 + hate).min(999_999_999)
        );
    }
}

#[test]
fn legacy_inputs_are_explicitly_separate_from_hf_profiles() {
    let expected = &oracle().legacy;
    assert_eq!(
        p_atk(
            Scaled::from_int(12).unwrap(),
            Scaled::from_raw(1_418_259),
            Scaled::from_raw(900_000)
        )
        .unwrap()
        .raw(),
        expected["ertheia_atk_q"]
    );
    let legacy = FormulaConstants {
        crit_scale: 1,
        ..rules().constants().clone()
    };
    assert_eq!(
        i64::from(crit_permille(&legacy, 44, Scaled::from_raw(1_100_000)).unwrap()),
        expected["interlude_crit"]
    );
    assert_eq!(
        i64::from(resource_max(637_700_000, Scaled::from_raw(1_570_000)).unwrap()),
        expected["fixture_hp"]
    );
    assert_eq!(
        i64::from(resource_max(287_400_000, Scaled::from_raw(1_280_000)).unwrap()),
        expected["fixture_mp"]
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]
    #[test]
    fn arbitrary_profiles_attributes_and_opponents_match_independent_oracle(
        ci in 0_usize..9, li in 0_usize..85, dex in 1_usize..100,
        str_ in 1_usize..100, con in 1_usize..100, armed in any::<bool>(),
        target_level in 0_usize..85, target_dex in 1_usize..100, roll in 0_u32..1000,
    ) {
        let s = check_sheet(ci, li, dex, str_, con, armed);
        let o = oracle();
        let e = o.dex[target_level][target_dex][1];
        let difference = (o.dex[li][dex][0] - e) / Q;
        let expected = o.hit_by_difference[usize::try_from(difference + 250).unwrap()];
        let actual = hit_chance_permille(rules().constants(), s.accuracy(), Scaled::from_raw(e)).unwrap();
        prop_assert_eq!(actual, expected);
        prop_assert_eq!(hit_lands(rules().constants(), actual, roll).unwrap(), roll <= expected);
    }

    #[test]
    fn arbitrary_xp_rewards_and_death_match_source_threshold_oracle(
        xp in 0_u64..20_000_000_000, reward in any::<u64>(),
    ) {
        let o = oracle();
        let expected = xp.saturating_add(reward).min(o.xp[85] - 1);
        prop_assert_eq!(add_xp(rules(), xp, reward).unwrap(), expected);
        let level = o.xp.iter().take(85).filter(|&&x| x <= expected).count();
        prop_assert_eq!(level_for_xp(rules(), expected), u32_of(level));
        let after = expected.saturating_sub(u64::try_from(o.levels[level - 1][3]).unwrap());
        prop_assert_eq!(xp_after_death(rules(), expected, u32_of(level)).unwrap(), after);
        let after_level = o.xp.iter().take(85).filter(|&&x| x <= after).count();
        prop_assert_eq!(level_for_xp(rules(), after), u32_of(after_level));
    }
}

#[test]
fn speed_and_crit_caps_match_oracle_including_synthetic_cap_inputs() {
    let c = rules().constants();
    for &[base, dex, speed, ms, impact, cycle] in &oracle().speed_cases {
        let bonus = Scaled::from_raw(oracle().bonus[1][usize::try_from(dex).unwrap()]);
        let actual = attack_speed(c, u32_of(base), bonus).unwrap();
        assert_eq!(actual.raw(), speed);
        let timing = attack_timing(c, actual).unwrap();
        assert_eq!(timing.interval_ms, (i128::from(ms), 1));
        assert_eq!((i64_of(timing.impact_ticks), i64_of(timing.cycle_ticks)), (impact, cycle));
    }
    for &[base, dex, rate] in &oracle().crit_cases {
        let bonus = Scaled::from_raw(oracle().bonus[1][usize::try_from(dex).unwrap()]);
        assert_eq!(crit_permille(c, u32_of(base), bonus).unwrap(), u32_of(rate));
    }
}
