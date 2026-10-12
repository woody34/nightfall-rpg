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
        inventory_revision: c
            .item_metadata
            .map_or(0, |metadata| metadata.inventory_revision),
        item_rules_hash: c
            .item_metadata
            .map_or_else(Vec::new, |metadata| metadata.rules_hash.to_vec()),
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
        item_metadata: None,
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

/// Maps every frozen owner-only field without consulting mutable progression.
pub(super) fn learn_result_to_pb(
    out: &crate::application::ports::FrozenLearnResult,
) -> pb::LearnSkillResponse {
    pb::LearnSkillResponse {
        character_id: out.character_id.to_string(),
        skill_id: out.skill.id.0,
        skill_level: out.skill.rank,
        sp_spent: out.sp_spent,
        sp_remaining: out.sp_remaining,
        tick: out.tick.0,
        hp: out.hp,
        max_hp: out.max_hp,
        mp: out.mp,
        max_mp: out.max_mp,
        cp: out.cp,
        max_cp: out.max_cp,
        incarnation: out.incarnation,
        effective_stats: Some(pb::EffectiveStats {
            p_atk_q: out.effective_stats.p_atk_q,
            p_def_q: out.effective_stats.p_def_q,
            m_atk_q: out.effective_stats.m_atk_q,
            m_def_q: out.effective_stats.m_def_q,
            attack_speed: out.effective_stats.attack_speed,
            cast_speed: out.effective_stats.cast_speed,
            speed_q: out.effective_stats.speed_q,
            accuracy_q: out.effective_stats.accuracy_q,
            evasion_q: out.effective_stats.evasion_q,
            crit_permille: out.effective_stats.crit_permille,
            ..Default::default()
        }),
    }
}

/// Preserves actor-owned eligibility, costs and prerequisite explanations.
pub(super) fn learnable_skills_to_pb(
    out: &crate::application::ports::LearnableSkillsState,
) -> pb::ListLearnableSkillsResponse {
    pb::ListLearnableSkillsResponse {
        current_class_id: out.current_class_id.0,
        sp: out.sp,
        skills: out
            .skills
            .iter()
            .map(|entry| {
                let skill = &entry.skill;
                pb::LearnableSkill {
                    learned: entry.learned,
                    affordable: entry.affordable,
                    trainer_required: entry.trainer_required,
                    eligible: entry.eligible,
                    unmet: entry.unmet.clone(),
                    skill: Some(pb::SkillLearnInfo {
                        key: skill.key.clone(),
                        skill_id: skill.skill_id,
                        l2_ref: skill.l2_ref.clone(),
                        learned_by_npc: skill.learned_by_npc,
                        effect_implemented: entry.effect_implemented,
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
                    }),
                }
            })
            .collect(),
    }
}

/// Deferred blockers remain public metadata; private learned ranks/resources are absent.
pub(super) fn skill_catalogue_to_pb(
    out: &crate::application::ports::SkillCatalogue,
) -> pb::GetSkillCatalogResponse {
    pb::GetSkillCatalogResponse {
        data_version: out.data_version.clone(),
        skills: out
            .skills
            .iter()
            .map(|skill| pb::SkillCatalogEntry {
                skill_id: skill.skill_id.0,
                key: skill.key.clone(),
                display_name: skill.display_name.clone(),
                max_level: skill.max_level,
                effect_implemented: skill.effect_implemented,
                deferred_reasons: skill.deferred_reasons.clone(),
                l2_ref: skill.l2_ref.clone(),
                mp_consume_start: skill.mp_consume_start.clone(),
                mp_consume_end: skill.mp_consume_end.clone(),
                hp_consume: skill.hp_consume.clone(),
                reuse_ms: skill.reuse_ms.clone(),
                hit_time_ms: skill.hit_time_ms.clone(),
                operate_type: match skill.operate_type {
                    crate::application::ports::SkillOperateType::ActiveInstant => {
                        pb::SkillOperateType::ActiveInstant
                    },
                    crate::application::ports::SkillOperateType::ActiveInstantContinuous => {
                        pb::SkillOperateType::ActiveInstantContinuous
                    },
                    crate::application::ports::SkillOperateType::ActiveContinuous => {
                        pb::SkillOperateType::ActiveContinuous
                    },
                    crate::application::ports::SkillOperateType::Passive => {
                        pb::SkillOperateType::Passive
                    },
                    crate::application::ports::SkillOperateType::Toggle => {
                        pb::SkillOperateType::Toggle
                    },
                    crate::application::ports::SkillOperateType::Other => {
                        pb::SkillOperateType::Other
                    },
                } as i32,
                target_type: skill.target_type.clone(),
                affect_scope: skill.affect_scope.clone(),
            })
            .collect(),
    }
}

/// Full immutable metadata projection; integers/rationals are checked at the wire boundary.
pub(super) fn item_catalogue_to_pb(
    rules: &crate::domain::zone::Phase4Rules,
) -> Result<pb::GetItemCatalogResponse, crate::application::AppError> {
    let parts = rules.parts();
    let items = parts
        .items
        .values()
        .map(|item| {
            let cap = parts
                .capabilities
                .get(&item.template_id)
                .ok_or_else(|| anyhow::anyhow!("missing item capability"))?;
            let mut blockers = std::collections::BTreeSet::new();
            blockers.extend(cap.blockers.iter().cloned());
            blockers.extend(cap.use_blockers.iter().cloned());
            let skills = item
                .skills
                .iter()
                .map(|skill| pb::ItemSkillReference {
                    id: skill.id,
                    rank: skill.level,
                    source: pb::ItemSkillSource::Item.into(),
                })
                .chain(
                    item.enchant4_skills
                        .iter()
                        .map(|skill| pb::ItemSkillReference {
                            id: skill.id,
                            rank: skill.level,
                            source: pb::ItemSkillSource::EnchantFour.into(),
                        }),
                )
                .collect();
            Ok(pb::ItemDefinition {
                template_id: item.template_id,
                name: item.name.clone(),
                kind: item_kind_to_pb(&item.kind)?.into(),
                grade: item_grade_to_pb(&item.grade)?.into(),
                bodypart_mask: item.bodypart_mask,
                weight: item.weight,
                stackable: item.stackable,
                usable: cap.admitted && blockers.is_empty(),
                blockers: blockers.into_iter().collect(),
                icon_key: item.icon.clone(),
                stats: item
                    .functions
                    .iter()
                    .map(|function| {
                        let (numerator, denominator) = function.value.value().parts();
                        Ok(pb::ItemStatDescription {
                            stat: function.stat.clone(),
                            operation: function.operation.clone(),
                            numerator: i64::try_from(numerator)?,
                            denominator: i64::try_from(denominator)?,
                            order: u32::try_from(function.order)?,
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?,
                skills,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let recipes = parts
        .sa_recipes
        .values()
        .filter(|recipe| {
            parts
                .recipe_capabilities
                .get(&recipe.recipe_id)
                .is_some_and(|cap| cap.admitted)
        })
        .map(|recipe| {
            anyhow::ensure!(
                recipe.products.len() == 1,
                "catalogue SA recipe has multiple products"
            );
            let product = recipe
                .products
                .first()
                .ok_or_else(|| anyhow::anyhow!("catalogue SA product missing"))?;
            anyhow::ensure!(
                product.count == 1,
                "catalogue SA product is not a single equipment instance"
            );
            Ok(pb::SaRecipe {
                recipe_id: recipe.recipe_id,
                base_template: recipe.base_template,
                ingredients: recipe
                    .ingredients
                    .iter()
                    .map(|ingredient| pb::ItemIngredient {
                        template: ingredient.template_id,
                        count: ingredient.count,
                    })
                    .collect(),
                product_template: product.template_id,
                maintain_enchantment: recipe.maintain_enchantment,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let capabilities = items
        .iter()
        .map(|item| pb::ItemCapability {
            template_id: item.template_id,
            blockers: item.blockers.clone(),
        })
        .collect();
    Ok(pb::GetItemCatalogResponse {
        data_version: 2,
        rules_hash: rules.hash().to_vec(),
        items,
        recipes,
        capabilities,
    })
}

fn item_kind_to_pb(kind: &str) -> anyhow::Result<pb::ItemKind> {
    match kind {
        "Weapon" => Ok(pb::ItemKind::Weapon),
        "Armor" => Ok(pb::ItemKind::Armor),
        "EtcItem" => Ok(pb::ItemKind::Etc),
        _ => anyhow::bail!("unsupported catalogue item kind"),
    }
}
fn item_grade_to_pb(grade: &str) -> anyhow::Result<pb::ItemGrade> {
    match grade {
        "NONE" => Ok(pb::ItemGrade::Ng),
        "D" => Ok(pb::ItemGrade::D),
        "C" => Ok(pb::ItemGrade::C),
        "B" => Ok(pb::ItemGrade::B),
        "A" => Ok(pb::ItemGrade::A),
        "S" => Ok(pb::ItemGrade::S),
        "S80" => Ok(pb::ItemGrade::S80),
        "S84" => Ok(pb::ItemGrade::S84),
        _ => anyhow::bail!("unsupported catalogue item grade"),
    }
}

/// Detached zero-origin projection. A live WS baseline always has a positive incarnation.
pub(super) fn detached_inventory_to_pb(
    output: &crate::application::use_cases::GetInventoryOutput,
) -> Result<pb::InventorySync, crate::application::AppError> {
    use crate::domain::zone::{phase4_checkpoint::ItemReuseKey, ItemLocation};
    let cp = &output.checkpoint;
    let weight = crate::domain::zone::item_views::ItemWeightView::new(
        cp.weight,
        cp.max_load,
        cp.weight_penalty_bonus,
    )
    .map_err(anyhow::Error::from)?;
    Ok(pb::InventorySync {
        entity_id: output.character_id.as_uuid().as_bytes().to_vec(),
        incarnation: 0,
        tick: 0,
        inventory_revision: cp.inventory_revision,
        baseline_tick: 0,
        baseline: true,
        items: cp
            .instances
            .iter()
            .map(|item| pb::OwnedItem {
                instance_id: item.id.get(),
                template_id: item.template_id,
                count: item.count,
                enchant: item.enchant,
                augment_id: item.augment_id,
                location: match item.location {
                    ItemLocation::Inventory => pb::ItemLocation::Inventory,
                    ItemLocation::Paperdoll => pb::ItemLocation::Paperdoll,
                }
                .into(),
                slot: item
                    .slot
                    .map_or(0, |slot| i32::from(slot as u8).saturating_add(1)),
                charge_bits: u32::from(item.charges),
                revision: item.revision,
            })
            .collect(),
        paperdoll: cp
            .paperdoll
            .iter()
            .map(|(slot, id)| pb::PaperdollEntry {
                slot: i32::from(*slot as u8).saturating_add(1),
                instance_id: id.get(),
            })
            .collect(),
        auto_shot_templates: cp.auto_shots.iter().copied().collect(),
        weight: Some(pb::ItemWeightState {
            weight: cp.weight,
            max_load: cp.max_load,
            penalty_bonus: cp.weight_penalty_bonus,
            per_mille: weight.per_mille,
        }),
        penalties: Some(pb::ItemPenaltyState {
            expertise: u32::from(cp.expertise),
            weapon_level: u32::from(cp.weapon_penalty),
            armor_level: u32::from(cp.armor_penalty),
            overload_level: u32::from(cp.overload_level),
            overloaded: cp.overloaded,
        }),
        reuse: cp
            .item_reuse_remainders
            .iter()
            .map(|reuse| {
                let (kind, key) = match reuse.key {
                    ItemReuseKey::Template(key) => (pb::ItemReuseKind::Template, key),
                    ItemReuseKey::SharedGroup(key) => (pb::ItemReuseKind::SharedGroup, key),
                };
                pb::ItemReuseState {
                    kind: kind.into(),
                    key,
                    until_tick: reuse.remaining_ticks,
                }
            })
            .collect(),
        rules_hash: cp.rules_hash.to_vec(),
        ordinary_capacity: output.ordinary_capacity,
        quest_capacity: output.quest_capacity,
    })
}

/// Freeze the original Grant response without reading current DB inventory or capacities.
pub(super) fn grant_receipt_to_pb(
    receipt: &crate::application::ports::ItemMutationReceipt,
) -> Result<pb::GrantItemResponse, crate::application::AppError> {
    receipt.validate().map_err(anyhow::Error::from)?;
    let pb::world_event::Event::ItemOperationResult(result) =
        crate::interface::zone_mapping::frozen_item_result_to_pb(&receipt.result)?
    else {
        return Err(anyhow::anyhow!("grant receipt does not contain an operation result").into());
    };
    if receipt.operation != crate::application::ports::ItemMutationOperation::GrantItem {
        return Err(anyhow::anyhow!("grant receipt operation is invalid").into());
    }
    let frozen = receipt
        .inventory
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("grant receipt has no frozen inventory"))?;
    let mut inventory =
        detached_inventory_to_pb(&crate::application::use_cases::GetInventoryOutput {
            character_id: receipt.character_id,
            character_revision: frozen.character_revision,
            checkpoint: frozen.checkpoint.clone(),
            ordinary_capacity: frozen.ordinary_capacity,
            quest_capacity: frozen.quest_capacity,
        })?;
    if result.incarnation != 0 {
        inventory.incarnation = result.incarnation;
        inventory.tick = result.tick;
        inventory.baseline_tick = result.tick;
        for reuse in &mut inventory.reuse {
            reuse.until_tick = reuse
                .until_tick
                .checked_add(result.tick)
                .ok_or_else(|| anyhow::anyhow!("grant baseline reuse deadline overflow"))?;
        }
    }
    Ok(pb::GrantItemResponse {
        result: Some(result),
        inventory: Some(inventory),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{AccountId, CharacterName};

    #[test]
    fn detached_inventory_nonpositive_load_matches_live_display() {
        for maximum in [0, -1, i64::MIN, 1000] {
            let mut checkpoint =
                crate::domain::zone::phase4_projection::tests::projection().checkpoint;
            checkpoint.weight = 100;
            checkpoint.max_load = maximum;
            checkpoint.overload_level = 4;
            checkpoint.overloaded = true;
            let expected = crate::domain::zone::item_views::ItemWeightView::new(
                checkpoint.weight,
                maximum,
                checkpoint.weight_penalty_bonus,
            )
            .unwrap();
            let wire =
                detached_inventory_to_pb(&crate::application::use_cases::GetInventoryOutput {
                    character_id: crate::domain::CharacterId::from_uuid(Uuid::from_u128(1)),
                    character_revision: 2,
                    ordinary_capacity: checkpoint.ordinary_capacity,
                    quest_capacity: checkpoint.quest_capacity,
                    checkpoint,
                })
                .unwrap();
            let weight = wire.weight.unwrap();
            assert_eq!(weight.max_load, maximum);
            assert_eq!(weight.per_mille, expected.per_mille);
            let penalties = wire.penalties.unwrap();
            assert_eq!(penalties.overload_level, 4);
            assert!(penalties.overloaded);
            assert_eq!((wire.tick, wire.baseline_tick, wire.incarnation), (0, 0, 0));
        }
    }

    #[test]
    fn detached_nonpositive_maximum_displays_zero_without_changing_penalty_history() {
        use crate::application::use_cases::GetInventoryOutput;
        let mut checkpoint = crate::domain::zone::phase4_projection::tests::projection().checkpoint;
        checkpoint.weight = 500;
        checkpoint.overload_level = 3;
        checkpoint.overloaded = true;
        for (maximum, per_mille) in [(-5, 0), (0, 0), (250, 2000)] {
            checkpoint.max_load = maximum;
            let output = GetInventoryOutput {
                character_id: crate::domain::CharacterId::from_uuid(Uuid::from_u128(1)),
                character_revision: 9,
                ordinary_capacity: checkpoint.ordinary_capacity,
                quest_capacity: checkpoint.quest_capacity,
                checkpoint: checkpoint.clone(),
            };
            let wire = detached_inventory_to_pb(&output).unwrap();
            let weight = wire.weight.unwrap();
            assert_eq!(
                (weight.weight, weight.max_load, weight.per_mille),
                (500, maximum, per_mille)
            );
            let penalties = wire.penalties.unwrap();
            assert_eq!((penalties.overload_level, penalties.overloaded), (3, true));
            assert_eq!(output.checkpoint, checkpoint);
        }
    }

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
        assert_eq!(p.inventory_revision, 0);
        assert!(p.item_rules_hash.is_empty());
        let mut retained = c.clone();
        retained.item_metadata = Some(crate::domain::CharacterItemMetadata {
            inventory_revision: i64::MAX as u64,
            rules_hash: [7; 32],
        });
        let wire = character_to_pb(&retained);
        assert_eq!(wire.inventory_revision, i64::MAX as u64);
        assert_eq!(wire.item_rules_hash, [7; 32]);
        assert!(serde_json::to_value(&c)
            .unwrap()
            .get("item_metadata")
            .is_none());
        assert_eq!(
            serde_json::from_value::<Character>(serde_json::to_value(&c).unwrap()).unwrap(),
            c
        );
    }
}
