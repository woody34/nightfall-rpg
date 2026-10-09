//! Tests for the stat-rule loader (Story E1.2) and the golden HF transcription (E1.1).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::*;
use crate::domain::zone::{
    attack_timing, death_xp_loss, level_for_xp, xp_cap, StatKind, StatSheet,
};

fn embedded() -> ResolvedRules {
    load_rules(&RulesSource::embedded()).unwrap()
}

/// The embedded files with `from` replaced by `to` in `rel` (which must contain `from`).
fn edited(rel: &str, from: &str, to: &str) -> RulesSource {
    let mut src = RulesSource::embedded();
    let text = src.0.get_mut(rel).unwrap();
    assert!(text.contains(from), "{rel} does not contain {from:?}");
    *text = text.replacen(from, to, 1);
    src
}

fn errors(src: &RulesSource) -> Vec<String> {
    load_rules(src).unwrap_err().0
}

fn assert_rejected(src: &RulesSource, needle: &str) {
    let errs = errors(src);
    assert!(errs.iter().any(|e| e.contains(needle)), "expected {needle:?} in {errs:#?}");
}

// ---------------------------------------------------------------------------
// Golden HF values (pinned l2j-server-datapack 3ca488dd / l2j-server-game abfde049)
// ---------------------------------------------------------------------------

#[test]
fn the_embedded_rules_load() {
    let r = embedded();
    assert_eq!(r.rules.max_level(), 85);
    assert_eq!(r.rules.classes().len(), 9);
    assert!(r.config_hash.starts_with("sha256:"));
    assert_eq!(r.config_hash.len(), "sha256:".len() + 64);
}

#[test]
fn hf_stat_bonus_rows_match_the_source_table() {
    let r = embedded().rules;
    let b = r.bonus();
    let row = |k, v| b.bonus(k, v).unwrap().raw();
    assert_eq!(row(StatKind::Str, 40), 1_200_000);
    assert_eq!(row(StatKind::Str, 99), 9_670_000);
    assert_eq!(row(StatKind::Dex, 30), 1_100_000);
    assert_eq!(row(StatKind::Con, 43), 1_580_000);
    assert_eq!(row(StatKind::Men, 25), 1_280_000);
    assert_eq!(row(StatKind::Wit, 50), 4_320_000);
    assert_eq!(row(StatKind::Str, 0), 290_000);
    assert!(b.bonus(StatKind::Str, 100).is_err());
}

#[test]
fn hf_formula_constants_are_the_resolved_ledger_values() {
    let r = embedded().rules;
    let c = r.constants();
    assert_eq!(c.damage_coefficient, 76);
    assert_eq!(c.crit_multiplier, 2);
    assert_eq!(c.attack_interval_ms, 500_000);
    assert_eq!(c.impact_divisor, 2);
    assert_eq!(c.accuracy_dex_multiplier, 6);
    assert_eq!(c.evasion_dex_multiplier, 6);
    assert_eq!((c.level_mod_offset, c.level_mod_divisor), (89, 100));
    assert_eq!((c.hit_base, c.hit_per_point, c.hit_scale), (80, 2, 10));
    assert_eq!((c.hit_min_permille, c.hit_max_permille), (200, 980));
    assert_eq!((c.crit_scale, c.crit_cap_permille), (10, 500));
    assert_eq!((c.evasion_cap, c.attack_speed_cap), (250, 1500));
    assert_eq!((c.hate_numerator, c.hate_level_offset, c.hate_cap), (100, 7, 999_999_999));
    assert_eq!(c.respawn_restore_hp.raw(), 650_000);
    assert_eq!(c.respawn_restore_mp.raw(), 0);
    assert_eq!(c.spawn_protection_seconds, 600);
    assert_eq!(r.accuracy_level_add(69).unwrap().raw(), 0);
    assert_eq!(r.accuracy_level_add(70).unwrap().raw(), 1_000_000);
    assert_eq!(r.accuracy_level_add(78).unwrap().raw(), 11_000_000);
    assert_eq!(r.evasion_level_add(78).unwrap().raw(), 10_800_000);
    assert_eq!(r.evasion_level_add(85).unwrap().raw(), 19_200_000);
}

#[test]
fn hf_experience_table_through_85_and_the_86_sentinel() {
    let r = embedded().rules;
    let x = |l| r.xp_to_level(l).unwrap();
    assert_eq!((x(1), x(2), x(3), x(20)), (0, 68, 363, 835_862));
    assert_eq!(x(21), 1_023_784);
    assert_eq!(x(76), 931_275_828);
    assert_eq!(x(80), 3_075_966_164);
    assert_eq!(x(85), 13_180_481_103);
    assert_eq!(x(86), 16_890_558_728);
    assert!(r.xp_to_level(87).is_err());
    assert_eq!(xp_cap(&r).unwrap(), 16_890_558_727);
    assert_eq!(level_for_xp(&r, 16_890_558_727), 85);
    // F((16890558728 - 13180481103) * 0.01)
    assert_eq!(death_xp_loss(&r, 85).unwrap(), 37_100_776);
}

#[test]
fn hf_death_loss_percentages() {
    let r = embedded().rules;
    let f = |l| r.death_loss_fraction(l).unwrap().raw();
    assert_eq!((f(1), f(2), f(48), f(49), f(75)), (100_000, 98_750, 41_250, 40_000, 40_000));
    assert_eq!((f(76), f(77), f(78), f(79), f(85)), (25_000, 20_000, 15_000, 10_000, 10_000));
    assert_eq!(death_xp_loss(&r, 2).unwrap(), 29);
}

#[test]
fn hf_human_fighter_matches_the_printed_values() {
    let r = embedded().rules;
    let hf = r.fighter_for(Race::Human).unwrap();
    assert_eq!(hf.id, "human_fighter");
    assert_eq!(
        hf.base,
        BaseStats {
            str: 40,
            dex: 30,
            con: 43,
            int: 21,
            wit: 11,
            men: 25
        }
    );
    assert_eq!(hf.p_def_unarmoured.raw(), 80_000_000);
    assert_eq!((hf.fist_crit_rate, hf.fist_attack_speed), (4, 300));
    // Stats doc §2.5 spot values (before CON / MEN).
    for (level, hp, mp) in [
        (1, 80_000_000, 30_000_000),
        (20, 327_000_000, 144_000_000),
        (40, 637_700_000, 287_400_000),
        (60, 1_000_400_000, 454_800_000),
        (76, 1_328_000_000, 606_000_000),
    ] {
        assert_eq!(hf.hp.at(level).unwrap(), hp, "hp at {level}");
        assert_eq!(hf.mp.at(level).unwrap(), mp, "mp at {level}");
    }
    let sheet = StatSheet::for_player(&r, hf, 1, Some(r.starter_weapon())).unwrap();
    assert_eq!(sheet.max_hp(), 126);
    assert_eq!(sheet.p_atk().raw(), 6_480_000);
    assert_eq!(sheet.accuracy().raw(), 34_000_000);
    assert_eq!(sheet.crit_permille(), 88);
    let timing = attack_timing(r.constants(), sheet.attack_speed()).unwrap();
    assert_eq!((timing.impact_ticks, timing.cycle_ticks), (6, 12));
}

#[test]
fn hf_starting_classes_match_stats_doc_table_2_2() {
    // The doc labels this table Interlude; the HF templates carry the same base stats.
    let r = embedded().rules;
    let rows: [(&str, [u32; 6], i64, u32); 9] = [
        ("human_fighter", [40, 30, 43, 21, 11, 25], 80, 115),
        ("human_mystic", [22, 21, 27, 41, 20, 39], 54, 120),
        ("elven_fighter", [36, 35, 36, 23, 14, 26], 80, 125),
        ("elven_mystic", [21, 24, 25, 37, 23, 40], 54, 122),
        ("dark_fighter", [41, 34, 32, 25, 12, 26], 80, 122),
        ("dark_mystic", [23, 23, 24, 44, 19, 37], 54, 122),
        ("orc_fighter", [40, 26, 47, 18, 12, 27], 80, 117),
        ("orc_mystic", [27, 24, 31, 31, 15, 42], 54, 121),
        ("dwarven_fighter", [39, 29, 45, 20, 10, 27], 80, 115),
    ];
    for (id, [str, dex, con, int, wit, men], p_def, _run) in rows {
        let c = r.class(id).unwrap();
        assert_eq!(
            c.base,
            BaseStats {
                str,
                dex,
                con,
                int,
                wit,
                men
            },
            "{id}"
        );
        assert_eq!(c.p_def_unarmoured, Scaled::from_int(p_def).unwrap(), "{id}");
        assert_eq!(c.fist_crit_rate, 4, "{id}: HF base crit is 4 (x10 per mille)");
    }
}

#[test]
fn every_race_has_its_fighter_and_the_domain_race_table_agrees() {
    let r = embedded().rules;
    for race in [
        Race::Human,
        Race::Elf,
        Race::DarkElf,
        Race::Orc,
        Race::Dwarf,
    ] {
        let c = r.fighter_for(race).unwrap();
        assert_eq!(c.base, race.starting_stats(), "{}", race.as_str());
    }
}

#[test]
fn the_starter_weapon_is_squires_sword() {
    let r = embedded().rules;
    let w = r.starter_weapon();
    assert_eq!(w.id, "squires_sword");
    assert_eq!((w.p_atk.raw(), w.crit_rate, w.attack_speed), (6_000_000, 8, 379));
    assert_eq!((w.random_damage, w.accuracy.raw(), w.attack_range_l2), (10, 0, 40));
}

// ---------------------------------------------------------------------------
// Discrepancies: incompatible printed values are identified, not absorbed
// ---------------------------------------------------------------------------

#[test]
fn docs_hf_closed_form_bonuses_are_not_the_hf_table() {
    // Docs: "HF STR 88 gives 1418259" (1.009^(s-49), Ertheia/l2j-mobius). HF STR 88 = 6.55.
    let r = embedded().rules;
    assert_eq!(r.bonus().bonus(StatKind::Str, 88).unwrap().raw(), 6_550_000);
    // Docs §2.1 "CON 43 -> 1.57"; the HF table rounds half-up to 1.58.
    assert_ne!(r.bonus().bonus(StatKind::Con, 43).unwrap().raw(), 1_570_000);
}

#[test]
fn docs_hf_xp_column_is_not_the_hf_table() {
    // Docs' "High Five" column (e.g. L41 8,718,976; L85 5,008,025,097) is the Ertheia table;
    // the docs' "Interlude" column also differs from HF from level 11 (71,201 vs 71,202).
    let r = embedded().rules;
    assert_ne!(r.xp_to_level(41).unwrap(), 8_718_976);
    assert_ne!(r.xp_to_level(85).unwrap(), 5_008_025_097);
    assert_eq!(r.xp_to_level(11).unwrap(), 71_202);
}

// ---------------------------------------------------------------------------
// Rejections (every error is listed)
// ---------------------------------------------------------------------------

#[test]
fn malformed_decimals_are_rejected() {
    let src = edited("tables/stat_bonus.toml", "\"1.2\",", "\"1.2e0\",");
    assert_rejected(&src, "stat_bonus.str[40]: \"1.2e0\" is not a plain decimal");
}

#[test]
fn a_toml_float_is_a_type_error() {
    let src = edited("tables/stat_bonus.toml", "\"1.2\",", "1.2,");
    assert_rejected(&src, "tables/stat_bonus.toml");
}

#[test]
fn a_literal_that_disagrees_with_its_scaled_integer_is_rejected() {
    let src = edited("tables/stat_bonus.toml", "1200000,", "1200001,");
    assert_rejected(&src, "stat_bonus.str[40]: literal 1.2 is 1200000 at Q, stored 1200001");
}

#[test]
fn holes_in_the_tables_are_rejected() {
    let src = edited("tables/experience.toml", "    { level = 40, xp = 15422929 },\n", "");
    assert_rejected(&src, "missing [40]");
    let src = edited(
        "tables/penalties.toml",
        "    { level = 85, percent = \"1.0\", fraction_q = 10000 },\n",
        "",
    );
    assert_rejected(&src, "penalties.death_xp_loss");
    let src = edited("tables/stat_bonus.toml", "\"0.29\", ", "");
    assert_rejected(&src, "stat_bonus.str: 99 literals");
    let src = edited("classes/human_fighter.toml", "\"91.83\", ", "");
    assert_rejected(&src, "classes/human_fighter.toml: hp.levels");
}

#[test]
fn non_monotone_xp_is_rejected() {
    let src = edited("tables/experience.toml", "xp = 15422929", "xp = 13844951");
    assert_rejected(&src, "not strictly increasing at level 40");
}

#[test]
fn bad_coefficients_and_caps_are_rejected() {
    assert_rejected(
        &edited("tables/formulas.toml", "coefficient = 76", "coefficient = 0"),
        "phys_damage.coefficient",
    );
    assert_rejected(
        &edited("tables/formulas.toml", "cap_permille = 500", "cap_permille = 5000"),
        "crit_rate.cap_permille",
    );
    assert_rejected(
        &edited("tables/formulas.toml", "min_permille = 200", "min_permille = 990"),
        "hit_chance.min_permille",
    );
    assert_rejected(
        &edited("tables/formulas.toml", "divisor = 100\n", "divisor = 0\n"),
        "level_mod.divisor",
    );
    assert_rejected(
        &edited("tables/starter_weapon.toml", "p_atk_spd = 379", "p_atk_spd = 0"),
        "attack speed must be > 0",
    );
    assert_rejected(
        &edited(
            "classes/human_fighter.toml",
            "chest = 31\nlegs = 18\nhead = 12\nfeet = 7\ngloves = 8\nunderwear = 3\ncloak = 1",
            "chest = 0\nlegs = 0\nhead = 0\nfeet = 0\ngloves = 0\nunderwear = 0\ncloak = 0",
        ),
        "P.Def must be > 0",
    );
}

#[test]
fn overflowing_data_is_rejected_at_startup() {
    let src = edited(
        "classes/human_fighter.toml",
        "base = \"80\"\nbase_q = 80000000",
        "base = \"9000000000000\"\nbase_q = 9000000000000000000",
    );
    let errs = errors(&src);
    assert!(
        errs.iter()
            .any(|e| e.contains("source row") || e.contains("overflow")),
        "{errs:#?}"
    );
}

#[test]
fn bad_references_are_rejected() {
    assert_rejected(
        &edited("classes/orc_fighter.toml", "race = \"orc\"", "race = \"goblin\""),
        "unknown race goblin",
    );
    assert_rejected(
        &edited("classes/orc_fighter.toml", "race = \"orc\"", "race = \"human\""),
        "race human needs exactly one fighter template, found 2",
    );
    assert_rejected(
        &edited("classes/orc_fighter.toml", "id = \"orc_fighter\"", "id = \"orc\""),
        "does not match the file name",
    );
    assert_rejected(
        &edited("classes/orc_fighter.toml", "archetype = \"fighter\"", "archetype = \"rogue\""),
        "unknown archetype rogue",
    );
    assert_rejected(
        &edited("tables/penalties.toml", "revision = \"3ca488dd", "revision = \"0000"),
        "differs from stat_bonus's",
    );
    assert_rejected(
        &edited("tables/experience.toml", "max_level = 85", "max_level = 84"),
        "experience max_level 84",
    );
    let mut src = RulesSource::embedded();
    src.0.remove("classes/dwarven_fighter.toml");
    assert_rejected(&src, "race dwarf needs exactly one fighter");
    let mut src = RulesSource::embedded();
    src.0.remove("tables/formulas.toml");
    assert_rejected(&src, "tables/formulas.toml: missing");
}

#[test]
fn unknown_and_missing_formula_ids_are_rejected() {
    let src = edited("tables/formulas.toml", "[formulas.p_def]", "[formulas.m_def]");
    let errs = errors(&src);
    assert!(
        errs.iter()
            .any(|e| e.contains("m_def") || e.contains("p_def")),
        "{errs:#?}"
    );
}

#[test]
fn every_error_is_listed_not_just_the_first() {
    let mut src = edited("tables/formulas.toml", "coefficient = 76", "coefficient = 0");
    let text = src.0.get_mut("tables/experience.toml").unwrap();
    *text = text.replacen("xp = 15422929", "xp = 1", 1);
    let errs = errors(&src);
    assert!(errs.iter().any(|e| e.contains("phys_damage.coefficient")), "{errs:#?}");
    assert!(errs.iter().any(|e| e.contains("not strictly increasing")), "{errs:#?}");
    // And parse errors from different files are collected together too.
    let mut src = edited("tables/penalties.toml", "max_level = 85", "max_level = \"x\"");
    let text = src.0.get_mut("classes/human_mystic.toml").unwrap();
    *text = text.replacen("class_id = 10", "class_id = -1", 1);
    let errs = errors(&src);
    assert!(errs.iter().any(|e| e.starts_with("tables/penalties.toml")), "{errs:#?}");
    assert!(
        errs.iter()
            .any(|e| e.starts_with("classes/human_mystic.toml")),
        "{errs:#?}"
    );
    let shown = RulesError(errs).to_string();
    assert!(shown.starts_with("stat rules rejected (2 errors):"), "{shown}");
}

// ---------------------------------------------------------------------------
// Config hash
// ---------------------------------------------------------------------------

#[test]
fn identical_files_yield_identical_rules_and_hash() {
    let a = embedded();
    let b = embedded();
    assert_eq!(a.rules, b.rules);
    assert_eq!(a.config_hash, b.config_hash);
}

#[test]
fn comments_and_layout_do_not_change_the_hash() {
    let a = embedded();
    let b = load_rules(&edited(
        "tables/formulas.toml",
        "q = 1000000\n",
        "# a comment\nq    =   1000000\n\n",
    ))
    .unwrap();
    assert_eq!(a.config_hash, b.config_hash);
}

#[test]
fn one_changed_coefficient_changes_the_hash() {
    let a = embedded();
    let b = load_rules(&edited("tables/formulas.toml", "coefficient = 76", "coefficient = 77"))
        .unwrap();
    assert_eq!(b.rules.constants().damage_coefficient, 77);
    assert_ne!(a.config_hash, b.config_hash);
    let c =
        load_rules(&edited("tables/experience.toml", "xp = 15422929", "xp = 15422930")).unwrap();
    assert_ne!(a.config_hash, c.config_hash);
}

#[test]
fn loading_from_the_data_directory_matches_the_embedded_files() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/data");
    let from_dir = load_rules_dir(&dir).unwrap();
    assert_eq!(from_dir.config_hash, embedded().config_hash);
    assert_eq!(RulesSource::from_dir(&dir).unwrap(), RulesSource::embedded());
}

/// The loader must never parse a value through binary floating point. Built by
/// concatenation so the list does not trip its own scan.
#[test]
fn the_loader_source_uses_no_floats() {
    let forbidden = [
        format!("f{}", 32),
        format!("f{}", 64),
        format!("parse::<{}", "f"),
    ];
    for (file, src) in [
        ("rules_data.rs", include_str!("rules_data.rs")),
        ("rules_data_tests.rs", include_str!("rules_data_tests.rs")),
    ] {
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for token in &forbidden {
                assert!(!code.contains(token.as_str()), "{file}:{} uses `{token}`", n + 1);
            }
        }
    }
}
