//! Boundary and corruption tests for exact catalog admission and reserved subclass rules.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use super::*;
use crate::domain::subclass::{eligible, EligibilityContext, SubclassDenied};

fn load() -> ResolvedClasses {
    load_classes(&ClassSource::embedded()).unwrap()
}

#[test]
fn complete_catalog_has_exact_class_counts_and_source_growth() {
    let loaded = load();
    let registry = &loaded.registry;
    assert_eq!(registry.races().len(), 5);
    for (tier, count) in [(0, 9), (1, 18), (2, 31), (3, 31)] {
        assert_eq!(registry.classes().iter().filter(|c| c.tier == tier).count(), count);
    }
    for class in registry.classes() {
        assert_eq!(registry.growth_table(class.id).unwrap().len(), 85);
        assert!(registry.growth(class.id, 0).is_err());
        assert!(registry.growth(class.id, 86).is_err());
    }
    let row = registry.growth(ClassId(2), 76).unwrap();
    assert_eq!(row.hp, parse_decimal("3061.8").unwrap());
    assert_eq!(row.mp, parse_decimal("1155.6").unwrap());
    assert_eq!(row.cp, parse_decimal("2755.62").unwrap());
    assert_eq!(registry.get(ClassId(34)).unwrap().parent, Some(ClassId(32)));
    assert_eq!(registry.get(ClassId(104)).unwrap().parent, Some(ClassId(28)));
    assert!(registry.get(ClassId(123)).is_none());
    assert_eq!(registry.transfer_options(ClassId(0), 19), Vec::new());
    assert_eq!(
        registry.transfer_options(ClassId(0), 20),
        vec![ClassId(1), ClassId(4), ClassId(7)]
    );
}

#[test]
fn hash_ignores_formatting_but_tracks_metadata_growth_and_provenance() {
    let src = ClassSource::embedded();
    let original = load_classes(&src).unwrap();
    let mut commented = src.clone();
    for text in commented
        .0
        .values_mut()
        .filter(|t| !t.starts_with("level,"))
    {
        text.push_str("\n# insignificant\n");
    }
    assert_eq!(load_classes(&commented).unwrap().config_hash, original.config_hash);
    let mut metadata = src.clone();
    let text = metadata
        .0
        .get_mut("professions/human_fighter.toml")
        .unwrap();
    *text = text.replacen("Human Armsbearer", "Human Shieldbearer", 1);
    assert_ne!(load_classes(&metadata).unwrap().config_hash, original.config_hash);
    let mut bad_provenance = src.clone();
    let text = bad_provenance
        .0
        .get_mut("professions/human_fighter.toml")
        .unwrap();
    *text = text.replacen(
        "3ca488dd2bd0bfaca43e378886a3c2e37968153a",
        "3ca488dd2bd0bfaca43e378886a3c2e37968153b",
        1,
    );
    assert!(load_classes(&bad_provenance).is_err());
    let mut changed = src;
    let text = changed.0.get_mut("growth/human_fighter.csv").unwrap();
    *text = text.replacen("80.0", "80.1", 1);
    assert_ne!(load_classes(&changed).unwrap().config_hash, original.config_hash);
}

#[test]
fn snapshot_round_trip_keeps_exact_catalog_and_rejects_invalid_state() {
    let loaded = load();
    let json = serde_json::to_string(&*loaded.registry).unwrap();
    let restored: ClassRegistry = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, *loaded.registry);
    let mut broken = serde_json::to_value(&restored).unwrap();
    broken["classes"][1]["min_level"] = 19.into();
    assert!(serde_json::from_value::<ClassRegistry>(broken).is_err());
}

#[test]
fn loader_rejects_unknown_fields_bad_decimals_missing_tables_and_cycles() {
    let original = ClassSource::embedded();
    let mut bad = original.clone();
    bad.0
        .get_mut("professions/human_fighter.toml")
        .unwrap()
        .push_str("\nunknown_field = 3\n");
    assert!(load_classes(&bad).is_err());
    let mut bad = original.clone();
    bad.0
        .get_mut("growth/human_fighter.csv")
        .unwrap()
        .push_str("86,1e2,30,32\n");
    assert!(load_classes(&bad).is_err());
    let mut bad = original.clone();
    bad.0.remove("growth/human_fighter.csv");
    assert!(load_classes(&bad).is_err());
    let mut bad = original;
    let text = bad.0.get_mut("professions/human_fighter.toml").unwrap();
    *text = text.replacen("tier = 0", "parent = 1\ntier = 0", 1);
    assert!(load_classes(&bad).is_err());
}

#[test]
fn growth_can_inherit_a_parent_without_changing_stat_identity() {
    let mut src = ClassSource::embedded();
    let text = src.0.get_mut("professions/duelist.toml").unwrap();
    *text = text
        .lines()
        .filter(|line| !line.starts_with("growth ="))
        .collect::<Vec<_>>()
        .join("\n");
    src.0.remove("growth/duelist.csv");
    let loaded = load_classes(&src).unwrap();
    assert_eq!(
        loaded.registry.growth(ClassId(88), 85).unwrap(),
        loaded.registry.growth(ClassId(2), 85).unwrap()
    );
}

fn context() -> EligibilityContext {
    EligibilityContext {
        race: Race::Human,
        main_class: ClassId(2),
        main_level: 75,
        quest_completed: true,
        noble: false,
        held_classes: vec![],
    }
}

#[test]
fn subclass_denials_cover_level_tier_quests_race_equivalence_and_slots() {
    let loaded = load();
    let registry = &loaded.registry;
    assert_eq!(eligible(registry, &context(), ClassId(55)), Ok(()));
    let mut ctx = context();
    ctx.main_level = 74;
    assert_eq!(eligible(registry, &ctx, ClassId(55)), Err(SubclassDenied::MainLevel));
    ctx.main_level = 75;
    ctx.main_class = ClassId(1);
    assert_eq!(eligible(registry, &ctx, ClassId(55)), Err(SubclassDenied::MainTransfer));
    ctx = context();
    ctx.quest_completed = false;
    assert_eq!(eligible(registry, &ctx, ClassId(55)), Err(SubclassDenied::Quest));
    ctx.noble = true;
    assert_eq!(eligible(registry, &ctx, ClassId(55)), Ok(()));
    ctx = context();
    ctx.main_class = ClassId(20);
    ctx.race = Race::Elf;
    assert_eq!(eligible(registry, &ctx, ClassId(36)), Err(SubclassDenied::Race));
    assert_eq!(eligible(registry, &ctx, ClassId(5)), Err(SubclassDenied::Equivalent));
    ctx.main_class = ClassId(99);
    assert_eq!(eligible(registry, &ctx, ClassId(6)), Err(SubclassDenied::Equivalent));
    ctx = context();
    ctx.held_classes = vec![ClassId(8)];
    assert_eq!(eligible(registry, &ctx, ClassId(23)), Err(SubclassDenied::Equivalent));
    ctx.held_classes = vec![ClassId(8), ClassId(12), ClassId(55)];
    assert_eq!(eligible(registry, &ctx, ClassId(52)), Err(SubclassDenied::SlotsFull));
    for id in [51, 57, 1, 88] {
        assert_eq!(
            eligible(registry, &context(), ClassId(id)),
            Err(SubclassDenied::ForbiddenClass)
        );
    }
    assert_eq!(eligible(registry, &context(), ClassId(123)), Err(SubclassDenied::UnknownClass));
    assert_eq!(eligible(registry, &context(), ClassId(2)), Err(SubclassDenied::Equivalent));
}

#[test]
fn all_five_subclass_equivalence_sets_are_enforced() {
    let registry = load().registry;
    for group in [
        vec![5, 6, 20, 33],
        vec![8, 23, 36],
        vec![9, 24, 37],
        vec![12, 27, 40],
        vec![14, 28, 41],
    ] {
        for main in &group {
            for candidate in &group {
                let mut ctx = context();
                ctx.main_class = ClassId(*main);
                ctx.race = registry.get(ctx.main_class).unwrap().race;
                let target_race = registry.get(ClassId(*candidate)).unwrap().race;
                let expected = if (ctx.race == Race::Elf && target_race == Race::DarkElf)
                    || (ctx.race == Race::DarkElf && target_race == Race::Elf)
                {
                    SubclassDenied::Race
                } else {
                    SubclassDenied::Equivalent
                };
                assert_eq!(eligible(&registry, &ctx, ClassId(*candidate)), Err(expected));
            }
        }
    }
}

#[test]
fn mystic_movement_and_collision_preserve_source_differences() {
    let registry = load().registry;
    let fighter = registry.get(ClassId(0)).unwrap();
    let mystic = registry.get(ClassId(10)).unwrap();
    assert_eq!((fighter.movement.walk, fighter.movement.run), (80, 115));
    assert_eq!((mystic.movement.walk, mystic.movement.run), (78, 120));
    assert_eq!(fighter.collision.radius_male, parse_decimal("9").unwrap());
    assert_eq!(mystic.collision.radius_male, parse_decimal("7.5").unwrap());
    assert_eq!(registry.get(ClassId(49)).unwrap().movement.run, 121);
    assert!(registry
        .races()
        .iter()
        .all(|race| race.start_points == vec![[0, 0]]));
}

#[test]
fn zero_and_negative_class_collision_are_rejected_at_load_and_restore() {
    for raw in [0, -1] {
        let mut source = ClassSource::embedded();
        let text = source.0.get_mut("professions/human_fighter.toml").unwrap();
        *text = text.replacen("radius_male = 9000000", &format!("radius_male = {raw}"), 1);
        assert!(load_classes(&source).is_err());
        let mut value = serde_json::to_value(&*load().registry).unwrap();
        value["classes"][0]["collision"]["radius_male"] = raw.into();
        assert!(serde_json::from_value::<ClassRegistry>(value).is_err());
    }
}

#[test]
fn nested_unknown_fields_are_rejected_in_catalogs_and_snapshots() {
    let mut source = ClassSource::embedded();
    let text = source.0.get_mut("professions/human_fighter.toml").unwrap();
    *text = text.replacen("[base_stats]", "[base_stats]\nbase_stat_typo = 1", 1);
    assert!(load_classes(&source).is_err());
    let original = serde_json::to_value(&*load().registry).unwrap();
    let mut value = original.clone();
    value["classes"][0]["base_stats"]["typo"] = true.into();
    assert!(serde_json::from_value::<ClassRegistry>(value).is_err());
    let mut value = original.clone();
    value["races"][0]["typo"] = true.into();
    assert!(serde_json::from_value::<ClassRegistry>(value).is_err());
    let mut value = original;
    value["growth"]["human_fighter"][0]["typo"] = true.into();
    assert!(serde_json::from_value::<ClassRegistry>(value).is_err());
}

#[test]
fn full_source_learning_trees_are_populated_inherited_and_bounded() {
    use crate::domain::class::{RuntimeSkillStatus, SkillTreeStatus};
    let registry = load().registry;
    let populated: Vec<_> = registry
        .classes()
        .iter()
        .filter(|c| c.skill_tree_status == SkillTreeStatus::Populated)
        .map(|c| c.id.0)
        .collect();
    assert_eq!(
        populated,
        vec![
            0, 1, 2, 4, 5, 7, 8, 9, 10, 11, 12, 15, 16, 17, 18, 19, 22, 25, 26, 27, 29, 31, 32, 33,
            35, 38, 39, 42, 44, 45, 46, 47, 49, 50, 52, 53, 54, 55, 56
        ]
    );
    assert_eq!(
        registry
            .classes()
            .iter()
            .map(|c| c.skill_tree.len())
            .sum::<usize>(),
        6927
    );
    assert_eq!(
        registry
            .classes()
            .iter()
            .map(|c| c.proficiencies.len())
            .sum::<usize>(),
        3521
    );
    assert_eq!(registry.known_skills().len(), 378);
    assert!(registry
        .known_skills()
        .iter()
        .all(|s| s.runtime_status == RuntimeSkillStatus::Deferred));
    let strike = registry
        .get(ClassId(0))
        .unwrap()
        .skill_tree
        .iter()
        .find(|s| s.skill_id == 3 && s.skill_level == 1)
        .unwrap();
    assert_eq!(
        (strike.required_level, strike.sp_cost, strike.auto_get, strike.learned_by_npc),
        (5, 50, false, true)
    );
    assert!(registry
        .skill_tree(ClassId(2))
        .unwrap()
        .iter()
        .any(|s| s.skill_id == 1322 && s.skill_level == 1 && s.auto_get));
    for class in registry.classes().iter().filter(|c| c.tier == 0) {
        let gates: Vec<_> = class
            .proficiencies
            .iter()
            .filter(|p| p.skill_id == 239)
            .map(|p| (p.skill_level, p.min_level))
            .collect();
        assert_eq!(
            gates,
            vec![
                (1, 20),
                (2, 40),
                (3, 52),
                (4, 61),
                (5, 76),
                (6, 80),
                (7, 84)
            ]
        );
    }
    let mut source = ClassSource::embedded();
    let text = source.0.get_mut("professions/human_fighter.toml").unwrap();
    *text = text.replacen("key = \"l2.skill.194\"", "key = \"l2.skill.999999\"", 1);
    assert!(load_classes(&source).is_err());
    let mut source = ClassSource::embedded();
    let text = source.0.get_mut("professions/human_fighter.toml").unwrap();
    *text = text.replacen("skill_level = 1", "skill_level = 999999", 1);
    assert!(load_classes(&source).is_err());
}

#[test]
fn unreferenced_growth_is_rejected_in_external_catalogs() {
    let mut source = ClassSource::embedded();
    source
        .0
        .insert("growth/obsolete.csv".to_owned(), source.0["growth/human_fighter.csv"].clone());
    assert!(load_classes(&source).is_err());
}
