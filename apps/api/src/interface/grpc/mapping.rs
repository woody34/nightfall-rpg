//! Proto <-> domain conversions. Pure functions, unit-tested.

use super::pb;
use crate::application::use_cases::ClassCatalogue;
use crate::domain::character_progression::{CharacterAppearance, ClassState, FrozenTransferResult};
use crate::domain::subclass::Sex;
use crate::domain::{BaseStats, Character, Position, Race};

pub(super) fn race_to_pb(r: Race) -> pb::Race {
    match r {
        Race::Human => pb::Race::Human,
        Race::Elf => pb::Race::Elf,
        Race::DarkElf => pb::Race::DarkElf,
        Race::Orc => pb::Race::Orc,
        Race::Dwarf => pb::Race::Dwarf,
    }
}

/// `None` for `RACE_UNSPECIFIED` or an unknown enum number.
pub(super) fn race_from_pb(n: i32) -> Option<Race> {
    match pb::Race::try_from(n).ok()? {
        pb::Race::Human => Some(Race::Human),
        pb::Race::Elf => Some(Race::Elf),
        pb::Race::DarkElf => Some(Race::DarkElf),
        pb::Race::Orc => Some(Race::Orc),
        pb::Race::Dwarf => Some(Race::Dwarf),
        pb::Race::Unspecified => None,
    }
}

pub(super) fn character_to_pb(c: &Character) -> pb::Character {
    pb::Character {
        id: c.id.to_string(),
        name: c.name.as_str().to_owned(),
        race: race_to_pb(c.race) as i32,
        level: c.level,
        stats: Some(pb::BaseStats {
            str: c.stats.str,
            dex: c.stats.dex,
            con: c.stats.con,
            int: c.stats.int,
            wit: c.stats.wit,
            men: c.stats.men,
        }),
        position: Some(pb::Position {
            x: c.position.x,
            y: c.position.y,
        }),
        class_id: c.class_state.current_class_id.0,
        base_class_id: c.class_state.base_class_id.0,
        active_class_slot: 0,
        classes: vec![pb::ClassProgress {
            slot: 0,
            class_id: c.class_state.current_class_id.0,
            level: c.level,
            exp: c.xp,
            sp: c.class_state.sp,
        }],
        sex: sex_to_pb(c.appearance.sex) as i32,
        hair_style: c.appearance.hair_style,
        hair_color: c.appearance.hair_color,
        face: c.appearance.face,
    }
}

/// Zero preserves legacy appearance; all unknown numeric values are refused.
pub(super) fn appearance_from_pb(
    sex: i32,
    hair_style: u32,
    hair_color: u32,
    face: u32,
) -> Result<CharacterAppearance, crate::application::AppError> {
    let sex = match pb::Sex::try_from(sex).ok() {
        Some(pb::Sex::Unspecified | pb::Sex::Male) => Sex::Male,
        Some(pb::Sex::Female) => Sex::Female,
        None => return Err(crate::application::AppError::InvalidArgument("invalid sex".into())),
    };
    Ok(CharacterAppearance {
        sex,
        hair_style,
        hair_color,
        face,
    })
}

fn sex_to_pb(sex: Sex) -> pb::Sex {
    match sex {
        Sex::Male => pb::Sex::Male,
        Sex::Female => pb::Sex::Female,
    }
}

fn stats_to_pb(stats: BaseStats) -> pb::BaseStats {
    pb::BaseStats {
        str: stats.str,
        dex: stats.dex,
        con: stats.con,
        int: stats.int,
        wit: stats.wit,
        men: stats.men,
    }
}

pub(super) fn transfer_result_to_pb(result: &FrozenTransferResult) -> pb::ChangeClassResponse {
    let mut class_state = ClassState::new(result.identity.base_class_id);
    class_state.current_class_id = result.current_class_id;
    class_state.sp = result.sp;
    let [x, y] = result.position_millitiles;
    let character = Character {
        id: result.character_id,
        account_id: result.identity.account_id,
        name: result.name.clone(),
        race: result.identity.race,
        level: result.level,
        stats: result.stats,
        position: Position {
            x: x as f32 / 1000.0,
            y: y as f32 / 1000.0,
        },
        appearance: result.identity.appearance,
        class_state,
        xp: result.xp,
    };
    pb::ChangeClassResponse {
        character: Some(character_to_pb(&character)),
        granted_skill_keys: result.granted_skill_keys.clone(),
        token_tier_1_count: result.token_tier_1_count,
        token_tier_2_count: result.token_tier_2_count,
    }
}

pub(super) fn catalogue_to_pb(catalogue: &ClassCatalogue) -> pb::ListClassesResponse {
    use crate::domain::zone::Archetype;
    pb::ListClassesResponse {
        data_version: catalogue.data_version.clone(),
        playable_level_cap: Character::MAX_LEVEL,
        max_transfer_tier: 2,
        class_master: Some(pb::ClassMasterInfo {
            name: "Class Master".into(),
            position: Some(pb::Position { x: 126.0, y: 128.0 }),
            interaction_radius: 3.0,
        }),
        races: catalogue
            .registry
            .races()
            .iter()
            .map(|race| pb::RaceInfo {
                race: race_to_pb(race.id) as i32,
                display_name: race.display_name.clone(),
                mystic_path: race.mystic_path,
                walk_speed: race.movement.walk,
                run_speed: race.movement.run,
                base_class_ids: race.base_class_ids.iter().map(|id| id.0).collect(),
                passive_skill_keys: race.passive_skill_keys.clone(),
                hair_style_count: 1,
                hair_color_count: 1,
                face_count: 1,
                passives: race
                    .passive_skill_keys
                    .iter()
                    .map(|key| passive_to_pb(key))
                    .collect(),
            })
            .collect(),
        classes: catalogue
            .registry
            .classes()
            .iter()
            .map(|class| pb::ClassInfo {
                class_id: class.id.0,
                key: class.key.clone(),
                display_name: class.display_name.clone(),
                race: race_to_pb(class.race) as i32,
                tier: u32::from(class.tier),
                parent_class_id: class.parent.map_or(65535, |id| id.0),
                min_level: class.min_level,
                archetype: match class.archetype {
                    Archetype::Fighter => pb::Archetype::Fighter,
                    Archetype::Mystic => pb::Archetype::Mystic,
                } as i32,
                base_stats: Some(stats_to_pb(class.base_stats)),
                subclass_allowed: class.subclass_allowed,
                subclass_equivalents: class.subclass_equivalents.iter().map(|id| id.0).collect(),
                skill_tree: class
                    .skill_tree
                    .iter()
                    .map(|skill| pb::SkillLearnInfo {
                        key: skill.key.clone(),
                        skill_id: skill.skill_id,
                        l2_ref: skill.l2_ref.clone(),
                        learned_by_npc: skill.learned_by_npc,
                        effect_implemented: false,
                        skill_level: skill.skill_level,
                        required_level: skill.required_level,
                        sp_cost: skill.sp_cost,
                        auto_get: skill.auto_get,
                        required_items: skill
                            .required_items
                            .iter()
                            .map(|item| pb::SkillItemRequirement {
                                item: item.item.clone(),
                                count: item.count,
                            })
                            .collect(),
                    })
                    .collect(),
                proficiencies: class
                    .proficiencies
                    .iter()
                    .map(|p| pb::ProficiencyInfo {
                        key: p.key.clone(),
                        min_level: p.min_level,
                        skill_id: p.skill_id,
                        skill_level: p.skill_level,
                        l2_ref: p.l2_ref.clone(),
                        effect_implemented: false,
                    })
                    .collect(),
                walk_speed: class.movement.walk,
                run_speed: class.movement.run,
                swim_speed: class.movement.swim,
                skill_tree_populated: class.skill_tree_status
                    == crate::domain::class::SkillTreeStatus::Populated,
            })
            .collect(),
    }
}

fn passive_to_pb(key: &str) -> pb::PassiveInfo {
    // Available racial mechanics match the Phase 2 zone; other traits remain explicit future hooks.
    let (name, description) = match key {
        "racial.adaptable" => ("Adaptable", "+5% XP gain. SP gain bonus is deferred."),
        "racial.forest_step" => ("Forest Step", "+3 run speed and +3% evasion."),
        "racial.mother_tree_attunement" => (
            "Mother Tree Attunement",
            "Planned: +50% HP/MP regeneration in Elven forest zones.",
        ),
        "racial.shadow_precision" => ("Shadow Precision", "+5% critical damage."),
        "racial.iron_constitution" => {
            ("Iron Constitution", "Planned: +10% HP regeneration and +5 stun resistance.")
        },
        "racial.pack_mule" => ("Pack Mule", "Planned: +20% weight limit."),
        "racial.keen_eye" => ("Keen Eye", "Planned: Spoil/Sweep skill access."),
        "racial.artisan_hands" => ("Artisan Hands", "Planned: crafting skill access."),
        _ => (key, "Reserved racial passive hook."),
    };
    pb::PassiveInfo {
        key: key.into(),
        display_name: name.into(),
        description: description.into(),
        implemented: matches!(
            key,
            "racial.adaptable" | "racial.forest_step" | "racial.shadow_precision"
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{AccountId, CharacterName};

    #[test]
    fn race_round_trips_and_rejects_unspecified() {
        for r in [
            Race::Human,
            Race::Elf,
            Race::DarkElf,
            Race::Orc,
            Race::Dwarf,
        ] {
            assert_eq!(race_from_pb(race_to_pb(r) as i32), Some(r));
        }
        assert_eq!(race_from_pb(0), None);
        assert_eq!(race_from_pb(999), None);
    }

    #[test]
    fn character_maps_every_field() {
        let c = Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Frodo").unwrap(),
            Race::Dwarf,
        );
        let p = character_to_pb(&c);
        assert_eq!(p.id, c.id.to_string());
        assert_eq!(p.name, "Frodo");
        assert_eq!(p.race, pb::Race::Dwarf as i32);
        assert_eq!(p.level, 1);
        assert_eq!(p.stats.unwrap().con, Race::Dwarf.starting_stats().con);
        assert!(p.position.is_some());
    }
}
