//! V7 recipe book and crafting (story 3.8, design §8): `RegisterRecipe` (pinned `Recipes`
//! item handler), `DeleteRecipe` (`RequestRecipeBookDestroy`), and the synchronous recipe
//! maker (`RecipeController.RecipeItemMaker`, `AlternativeCrafting = False`) used by
//! `CraftItem` and, with a customer and a fee, by `PrivateManufactureOrder` (§9.3). Every
//! check runs before any payment; then MP → fee → materials → success draw (#10) →
//! masterwork draw (#11) → the product, granted with no capacity or weight check (P7b).

use crate::domain::zone::item_inventory::{InventoryEdit, InventoryLimits};
use crate::domain::zone::phase4_projection::ItemOperation;
use crate::domain::zone::phase5::FloodKind;
use crate::domain::zone::phase5_event::{EconomyBody, EconomyEvent, EconomyOutcome, EconomyScope};
use crate::domain::zone::phase5_rules::RecipeRow;
use crate::domain::zone::{
    CombatRole, EntityId, ItemLocation, Phase5Command, RejectReason, Tick, ZoneEvent, ZoneState,
};

/// Create Item (Dwarven recipes) and Create Common Item (Common recipes).
const CREATE_ITEM: &str = "l2.skill.172";
const CREATE_COMMON_ITEM: &str = "l2.skill.1320";
const ADENA: u32 = 57;

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

/// A craft's parties and terms.
#[derive(Debug, Clone, Copy)]
pub(super) struct CraftOrder<'a> {
    pub(super) crafter: EntityId,
    pub(super) customer: EntityId,
    /// The manufacture fee (0 for a self craft), customer → crafter.
    pub(super) fee: u64,
    pub(super) recipe: u32,
    pub(super) receipt: &'a str,
}

impl ZoneState {
    /// The learned rank of a craft skill (0 when not learned).
    fn craft_rank(&self, actor: EntityId, key: &str) -> u32 {
        self.entities
            .get(&actor)
            .and_then(|e| e.combat.as_ref())
            .and_then(|c| match &c.role {
                CombatRole::Player {
                    progression: Some(p),
                    ..
                } => p
                    .class_state
                    .learned_skills
                    .iter()
                    .find(|s| s.key == key)
                    .map(|s| s.level),
                CombatRole::Player { .. } | CombatRole::Npc { .. } => None,
            })
            .unwrap_or(0)
    }

    fn recipe_row(&self, id: u32) -> Option<RecipeRow> {
        self.phase5
            .as_ref()?
            .rules
            .parts()
            .recipes
            .recipe
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// The owner's `RecipeBookSync` delta (design §13.2), with the book after the change.
    fn recipe_sync(&self, tick: Tick, actor: EntityId, events: &mut Vec<ZoneEvent>) {
        let Some(phase5) = self.phase5.as_ref() else {
            return;
        };
        let Some(player) = phase5.canonical.players.get(&actor) else {
            return;
        };
        let config = &phase5.rules.parts().recipes.config;
        let incarnation = self
            .entities
            .get(&actor)
            .and_then(|e| e.combat.as_ref())
            .map_or(1, |c| c.incarnation);
        events.push(ZoneEvent::Economy(Box::new(EconomyEvent {
            tick,
            entity: actor,
            body: EconomyBody::RecipeBookSync {
                dwarven: player.recipes.dwarven.iter().copied().collect(),
                common: player.recipes.common.iter().copied().collect(),
                dwarven_limit: config.dwarf_recipe_limit,
                common_limit: config.common_recipe_limit,
                baseline: false,
                incarnation,
            },
            audience: Vec::new(),
        })));
    }

    /// `RegisterRecipe` (design §8.1, `Recipes.java:36-94`).
    pub(super) fn register_recipe(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::RegisterRecipe {
            recipe_instance, ..
        } = command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        let item = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&actor))
            .and_then(|p| p.inventory.get(recipe_instance))
            .filter(|i| i.location == ItemLocation::Inventory)
            .cloned()
            .ok_or(RejectReason::ItemNotOwned)?;
        // `getRecipeByItemId`.
        let recipe = self
            .phase5
            .as_ref()
            .ok_or(RejectReason::NotYetImplemented)?
            .rules
            .parts()
            .recipes
            .recipe
            .iter()
            .find(|r| r.recipe_id == item.template_id)
            .cloned()
            .ok_or(RejectReason::InvalidRecipe)?;
        let dwarven = recipe.r#type == "dwarven";
        let phase5 = self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?;
        let book = &phase5
            .canonical
            .players
            .get(&actor)
            .ok_or(RejectReason::InvalidLoad)?
            .recipes;
        if book.dwarven.contains(&recipe.id) || book.common.contains(&recipe.id) {
            return Err(RejectReason::RecipeAlreadyRegistered);
        }
        let rank = self.craft_rank(
            actor,
            if dwarven {
                CREATE_ITEM
            } else {
                CREATE_COMMON_ITEM
            },
        );
        if rank == 0 {
            return Err(RejectReason::CraftSkillMissing);
        }
        if recipe.craft_level > rank {
            return Err(RejectReason::CraftLevelTooLow);
        }
        let config = &phase5.rules.parts().recipes.config;
        let (held, limit) = if dwarven {
            (book.dwarven.len(), config.dwarf_recipe_limit)
        } else {
            (book.common.len(), config.common_recipe_limit)
        };
        if held >= usize::try_from(limit).map_err(invalid)? {
            return Err(RejectReason::RecipeBookFull);
        }
        // Commit: register, then consume the recipe item (count 1).
        let receipt = format!("receipt:register_recipe:{}", command.request_key());
        let player = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&actor))
            .ok_or(RejectReason::InvalidLoad)?;
        let limits = InventoryLimits::from_player(player);
        let changes = self.economy_commit(
            tick,
            actor,
            &[InventoryEdit::TakeInstance {
                instance: *recipe_instance,
                count: 1,
            }],
            limits,
            events,
        )?;
        let book = &mut self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.players.get_mut(&actor))
            .ok_or(RejectReason::InvalidLoad)?
            .recipes;
        if dwarven {
            book.dwarven.insert(recipe.id);
        } else {
            book.common.insert(recipe.id);
        }
        for change in changes.into_iter().flatten() {
            self.push_economy_consume(tick, actor, change, &receipt, events)?;
        }
        self.recipe_sync(tick, actor, events);
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::RegisterRecipe,
            vec![*recipe_instance],
            EconomyOutcome::Recipe {
                recipe_id: recipe.id,
                dwarven,
            },
            events,
        );
        Ok(())
    }

    /// `DeleteRecipe` (`RequestRecipeBookDestroy.java:46-63`): the Transaction gate, then
    /// the recipe leaves the book with no refund.
    pub(super) fn delete_recipe(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::DeleteRecipe { recipe_id, .. } = command else {
            return Err(RejectReason::InvalidLoad);
        };
        self.flood_gate_v7(tick, actor, FloodKind::Transaction)?;
        let book = &mut self
            .phase5
            .as_mut()
            .and_then(|p| p.canonical.players.get_mut(&actor))
            .ok_or(RejectReason::InvalidLoad)?
            .recipes;
        let dwarven = if book.dwarven.remove(recipe_id) {
            true
        } else if book.common.remove(recipe_id) {
            false
        } else {
            return Err(RejectReason::InvalidRecipe);
        };
        self.recipe_sync(tick, actor, events);
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::DeleteRecipe,
            Vec::new(),
            EconomyOutcome::Recipe {
                recipe_id: *recipe_id,
                dwarven,
            },
            events,
        );
        Ok(())
    }

    /// `CraftItem` (design §8.2): the Manufacture gate, not in combat, a registered recipe,
    /// then the maker for the crafter as its own customer.
    pub(super) fn craft_item(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::CraftItem { recipe_id, .. } = command else {
            return Err(RejectReason::InvalidLoad);
        };
        self.flood_gate_v7(tick, actor, FloodKind::Manufacture)?;
        if self.engaged_for_learning(actor) {
            return Err(RejectReason::EconomyInCombat);
        }
        let book = &self
            .phase5
            .as_ref()
            .and_then(|p| p.canonical.players.get(&actor))
            .ok_or(RejectReason::InvalidLoad)?
            .recipes;
        if !book.dwarven.contains(recipe_id) && !book.common.contains(recipe_id) {
            return Err(RejectReason::InvalidRecipe);
        }
        let receipt = format!("receipt:craft_item:{}", command.request_key());
        let outcome = self.make_item(
            tick,
            CraftOrder {
                crafter: actor,
                customer: actor,
                fee: 0,
                recipe: *recipe_id,
                receipt: &receipt,
            },
            events,
        )?;
        let affected = outcome.1;
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::CraftItem,
            affected,
            outcome.0,
            events,
        );
        Ok(())
    }

    /// The recipe maker (`RecipeItemMaker`): every check, then MP → fee → materials →
    /// success (#10) → masterwork (#11) → product. Returns the craft outcome and the
    /// customer's affected instances.
    #[allow(clippy::too_many_lines)] // The pinned order stays in one place.
    pub(super) fn make_item(
        &mut self,
        tick: Tick,
        order: CraftOrder<'_>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(EconomyOutcome, Vec<crate::domain::zone::ItemInstanceId>), RejectReason> {
        let recipe = self
            .recipe_row(order.recipe)
            .ok_or(RejectReason::InvalidRecipe)?;
        let alive = |id: EntityId| self.entities.get(&id).is_some_and(|e| !e.targeting.dead);
        if !alive(order.crafter) || !alive(order.customer) {
            return Err(RejectReason::DeadActor);
        }
        let busy = |id: EntityId| {
            self.phase5.as_ref().is_some_and(|p| {
                p.canonical
                    .players
                    .get(&id)
                    .is_some_and(|pl| pl.trade.is_some())
                    || p.canonical
                        .requests
                        .values()
                        .any(|r| r.requester == id || r.target == id)
            })
        };
        if busy(order.crafter) || (order.customer != order.crafter && busy(order.customer)) {
            return Err(RejectReason::TargetBusy);
        }
        if recipe.ingredients.is_empty() {
            return Err(RejectReason::InvalidRecipe);
        }
        let dwarven = recipe.r#type == "dwarven";
        let rank = self.craft_rank(
            order.crafter,
            if dwarven {
                CREATE_ITEM
            } else {
                CREATE_COMMON_ITEM
            },
        );
        if recipe.craft_level > rank {
            return Err(RejectReason::CraftLevelTooLow);
        }
        if order.fee > 0 && self.adena_of_v7(order.customer) < order.fee {
            return Err(RejectReason::InsufficientAdena);
        }
        // `listItems(false)`: every material held by the customer (inventory, not equipped).
        let customer_items = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&order.customer))
            .ok_or(RejectReason::InvalidLoad)?
            .inventory
            .clone();
        let mut takes = Vec::new();
        for ingredient in &recipe.ingredients {
            let template = u32::try_from(ingredient.id).map_err(|_| RejectReason::InvalidRecipe)?;
            let mut remaining = ingredient.count;
            for item in customer_items
                .values()
                .filter(|i| i.template_id == template && i.location == ItemLocation::Inventory)
            {
                if remaining == 0 {
                    break;
                }
                let take = item.count.min(remaining);
                remaining = remaining.saturating_sub(take);
                takes.push(InventoryEdit::TakeInstance {
                    instance: item.id,
                    count: take,
                });
            }
            if remaining > 0 {
                return Err(RejectReason::MissingMaterials);
            }
        }
        // `calculateStatUse(false, false)`: HP needs `hp > value`, MP needs `mp ≥ value`.
        let (mut mp_use, mut hp_use) = (0_u64, 0_u64);
        for stat in &recipe.stat_use {
            match stat.name.as_str() {
                "MP" => mp_use = mp_use.saturating_add(stat.value),
                "HP" => hp_use = hp_use.saturating_add(stat.value),
                _ => return Err(RejectReason::InvalidRecipe),
            }
        }
        let crafter = self
            .entities
            .get(&order.crafter)
            .and_then(|e| e.combat.as_ref())
            .ok_or(RejectReason::InvalidLoad)?;
        if hp_use > 0 && u64::from(crafter.hp) <= hp_use {
            return Err(RejectReason::InsufficientHp);
        }
        if u64::from(crafter.mp) < mp_use {
            return Err(RejectReason::InsufficientMp);
        }
        let product = u32::try_from(recipe.production.id).map_err(invalid)?;
        let rare = recipe.production_rare.clone();
        // Allocator room for a fresh product (merge needs none) before anything is paid.
        let available = self.phase4.as_ref().map_or(0, |items| {
            let (_, next, end) = items.canonical.allocator.bounds();
            end.saturating_sub(next)
        });
        if available == 0 {
            return Err(RejectReason::InventoryFull);
        }
        // 1. MP/HP paid by the crafter (`calculateStatUse(false, true)`).
        if let Some(c) = self
            .entities
            .get_mut(&order.crafter)
            .and_then(|e| e.combat.as_mut())
        {
            c.mp = c.mp.saturating_sub(u32::try_from(mp_use).map_err(invalid)?);
            c.hp = c.hp.saturating_sub(u32::try_from(hp_use).map_err(invalid)?);
        }
        if let Some(phase3) = self.phase3.as_mut() {
            phase3.vitals_dirty.insert(order.crafter);
        }
        // 2. The fee, customer → crafter, before the materials and the roll; never refunded.
        if order.fee > 0 {
            self.move_adena_v7(
                tick,
                order.customer,
                order.crafter,
                order.fee,
                order.receipt,
                events,
            )?;
        }
        // 3. Materials from the customer.
        let player = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&order.customer))
            .ok_or(RejectReason::InvalidLoad)?;
        let limits = InventoryLimits::from_player(player);
        let paid = self.economy_commit(tick, order.customer, &takes, limits, events)?;
        for change in paid.into_iter().flatten() {
            self.push_economy_consume(tick, order.customer, change, order.receipt, events)?;
        }
        // 4. Success draw #10.
        let success = self
            .chance_bound(u64::from(recipe.success_rate), 100)
            .map_err(invalid)?;
        if !success {
            return Ok((
                EconomyOutcome::Craft {
                    success: false,
                    masterwork: false,
                    product_template: product,
                    count: 0,
                    fee: order.fee,
                },
                Vec::new(),
            ));
        }
        // 5. `rewardPlayer`: the rare gate (always open for pinned data), draw #11.
        let config = &self
            .phase5
            .as_ref()
            .ok_or(RejectReason::InvalidLoad)?
            .rules
            .parts()
            .recipes
            .config;
        let rare_id = rare.as_ref().map_or(0, |r| r.id);
        let (mut template, mut count) = (product, recipe.production.count);
        let mut masterwork = false;
        if rare_id == product || config.craft_masterwork {
            let rarity = rare.as_ref().map_or(0, |r| r.rarity);
            if self.chance_bound(u64::from(rarity), 100).map_err(invalid)? {
                if let Some(rare) = &rare {
                    template = rare.id;
                    count = rare.count;
                    masterwork = true;
                }
            }
        }
        let granted = self.economy_commit(
            tick,
            order.customer,
            &[InventoryEdit::Add { template, count }],
            InventoryLimits::unchecked(),
            events,
        )?;
        let mut affected = Vec::new();
        for change in granted.into_iter().flatten() {
            affected.push(self.push_economy_grant(
                tick,
                order.customer,
                change,
                ItemOperation::Craft,
                order.receipt,
                events,
            )?);
        }
        Ok((
            EconomyOutcome::Craft {
                success: true,
                masterwork,
                product_template: template,
                count,
                fee: order.fee,
            },
            affected,
        ))
    }

    fn adena_of_v7(&self, owner: EntityId) -> u64 {
        self.phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&owner))
            .map_or(0, |p| {
                p.inventory
                    .values()
                    .filter(|i| i.template_id == ADENA)
                    .map(|i| i.count)
                    .fold(0, u64::saturating_add)
            })
    }

    /// The manufacture fee as an adena transfer, customer → crafter: one Consume on the
    /// customer and a `transfer_in` (fresh or merge) on the crafter, both with the receipt.
    #[allow(clippy::too_many_arguments)] // The transfer's explicit inputs.
    fn move_adena_v7(
        &mut self,
        tick: Tick,
        from: EntityId,
        to: EntityId,
        amount: u64,
        receipt: &str,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let player = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&from))
            .ok_or(RejectReason::InvalidLoad)?;
        let limits = InventoryLimits::from_player(player);
        let taken = self.economy_commit(
            tick,
            from,
            &[InventoryEdit::TakeTemplate {
                template: ADENA,
                count: amount,
                preferred: None,
            }],
            limits,
            events,
        )?;
        for change in taken.into_iter().flatten() {
            self.push_economy_consume(tick, from, change, receipt, events)?;
        }
        let added = self.economy_commit(
            tick,
            to,
            &[InventoryEdit::Add {
                template: ADENA,
                count: amount,
            }],
            InventoryLimits::unchecked(),
            events,
        )?;
        for change in added.into_iter().flatten() {
            self.push_economy_grant(tick, to, change, ItemOperation::TransferIn, receipt, events)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::arithmetic_side_effects,
    clippy::panic
)]
mod tests {
    use super::*;
    use crate::domain::subclass::LearnedSkill;
    use crate::domain::zone::item_inventory::inventory_totals;
    use crate::domain::zone::phase5_rules::Phase5Rules;
    use crate::domain::zone::{
        ItemCreationOrigin, ItemInstance, ItemInstanceId, SessionGeneration, ZoneCommand, ZoneInput,
    };

    use super::super::item_materialize::tests::{kit_load_for, spawn_kit};

    const ME: u128 = 71;
    const CUSTOMER: u128 = 81;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    /// A V7 zone with a level-10 dwarven fighter crafter (Create Item rank 1 when `skill`)
    /// and a second dwarf as a customer.
    fn zone(skill: bool) -> ZoneState {
        let (mut zone, mut load, me) = kit_load_for(true, Some(53));
        let stats = zone.rules.clone().unwrap();
        load.level = 10;
        load.xp = stats.xp_table()[9];
        let base = vec![
            LearnedSkill {
                key: "l2.skill.254".into(),
                level: 1,
            },
            LearnedSkill {
                key: "l2.skill.42".into(),
                level: 1,
            },
        ];
        let mut customer = load.clone();
        if let Some(progression) = load.progression.as_mut() {
            progression.class_state.learned_skills = base.clone();
            progression.class_state.sp = 10_000;
        }
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        customer.phase5 = load.phase5.clone();
        if let Some(progression) = customer.progression.as_mut() {
            progression.identity.account_id =
                crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(82));
            progression.class_state.learned_skills = base;
        }
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(me, load)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "crafter {:?}", tick.dispositions);
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(CUSTOMER), customer)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "customer {:?}", tick.dispositions);
        if skill {
            // Create Item rank 1 is a passive: learned in the zone (SP 310, level 5).
            let master = crate::domain::zone::Vec2Fixed::from_tiles(126, 127);
            let from = zone.entities[&me].pos;
            zone.entities.get_mut(&me).unwrap().pos = master;
            zone.aoi.relocate(me, from, master);
            let tick = zone
                .run_tick(zone.draft(vec![ZoneInput::session(
                    me,
                    SessionGeneration(1),
                    900,
                    ZoneCommand::LearnSkill {
                        entity: me,
                        account: crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(72)),
                        request_key: uuid::Uuid::from_u128(901),
                        skill_id: crate::domain::zone::skill::SkillId(172),
                        skill_level: 1,
                    },
                )]))
                .unwrap();
            assert!(tick.dispositions.is_empty(), "learn {:?}", tick.dispositions);
            assert_eq!(zone.craft_rank(me, CREATE_ITEM), 1);
        }
        zone
    }

    fn give(zone: &mut ZoneState, who: u128, template: u32, count: u64) -> ItemInstanceId {
        let items = zone.phase4.as_mut().unwrap();
        let new = items.canonical.allocator.allocate().unwrap();
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&id(who)).unwrap();
        player.inventory_revision += 1;
        player.inventory.insert(
            new,
            ItemInstance {
                creation_origin: Some(ItemCreationOrigin {
                    zone: zone.seed.zone,
                    epoch: zone.seed.epoch,
                }),
                id: new,
                template_id: template,
                count,
                location: ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: player.inventory_revision,
            },
        );
        player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
        zone.rebuild_v5(id(who)).unwrap();
        new
    }

    fn held(zone: &ZoneState, who: u128, template: u32) -> u64 {
        zone.phase4.as_ref().unwrap().canonical.players[&id(who)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| i.count)
            .sum()
    }

    fn mp(zone: &ZoneState, who: u128) -> u32 {
        zone.entities[&id(who)].combat.as_ref().unwrap().mp
    }

    fn run(
        zone: &mut ZoneState,
        key: u128,
        body: serde_json::Value,
    ) -> crate::domain::zone::AppliedTick {
        let mut value = body;
        value["entity"] = serde_json::json!(id(ME));
        value["request_key"] = serde_json::json!(uuid::Uuid::from_u128(key));
        let command: Phase5Command = serde_json::from_value(value).unwrap();
        zone.run_tick(zone.draft(vec![ZoneInput::session(
            id(ME),
            SessionGeneration(1),
            u32::try_from(key).unwrap(),
            ZoneCommand::Phase5(Box::new(command)),
        )]))
        .unwrap()
    }

    fn reasons(tick: &crate::domain::zone::AppliedTick) -> Vec<RejectReason> {
        tick.dispositions.iter().map(|d| d.reason).collect()
    }

    fn book(zone: &ZoneState) -> crate::domain::zone::RecipeBook {
        zone.phase5.as_ref().unwrap().canonical.players[&id(ME)]
            .recipes
            .clone()
    }

    fn wait(zone: &mut ZoneState, ticks: u64) {
        for _ in 0..ticks {
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
    }

    /// §8.1: the ability check, register + consume the recipe item, already registered,
    /// then delete (Transaction gate) with no refund.
    #[test]
    fn register_and_delete_recipes() {
        let mut none = zone(false);
        let item = give(&mut none, ME, 1666, 1);
        let tick = run(
            &mut none,
            1,
            serde_json::json!({"kind": "register_recipe", "recipe_instance": item}),
        );
        assert_eq!(reasons(&tick), vec![RejectReason::CraftSkillMissing]);
        let mut zone = zone(true);
        let item = give(&mut zone, ME, 1666, 2);
        let tick = run(
            &mut zone,
            1,
            serde_json::json!({"kind": "register_recipe", "recipe_instance": item}),
        );
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(book(&zone).dwarven.contains(&1));
        assert_eq!(held(&zone, ME, 1666), 1, "one recipe item consumed");
        assert!(tick.events.iter().any(|e| matches!(
            e,
            ZoneEvent::Economy(ev) if matches!(ev.body, EconomyBody::RecipeBookSync { .. })
        )));
        let tick = run(
            &mut zone,
            2,
            serde_json::json!({"kind": "register_recipe", "recipe_instance": item}),
        );
        assert_eq!(reasons(&tick), vec![RejectReason::RecipeAlreadyRegistered]);
        assert_eq!(held(&zone, ME, 1666), 1);
        // A level-2 recipe needs rank 2.
        let level2 = zone
            .phase5
            .as_ref()
            .unwrap()
            .rules
            .parts()
            .recipes
            .recipe
            .iter()
            .find(|r| r.craft_level == 2 && r.r#type == "dwarven")
            .unwrap()
            .recipe_id;
        let item2 = give(&mut zone, ME, level2, 1);
        let tick = run(
            &mut zone,
            3,
            serde_json::json!({"kind": "register_recipe", "recipe_instance": item2}),
        );
        assert_eq!(reasons(&tick), vec![RejectReason::CraftLevelTooLow]);
        let tick = run(&mut zone, 4, serde_json::json!({"kind": "delete_recipe", "recipe_id": 1}));
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(book(&zone).dwarven.is_empty());
        wait(&mut zone, 10);
        let tick = run(&mut zone, 5, serde_json::json!({"kind": "delete_recipe", "recipe_id": 1}));
        assert_eq!(reasons(&tick), vec![RejectReason::InvalidRecipe]);
    }

    /// §8.2 (family C): a missing material refuses with nothing spent (no MP, no item, no
    /// draw); a craft pays MP 30, consumes the materials, draws #10 and #11 and grants the
    /// product as a `craft` row.
    #[test]
    fn craft_checks_everything_before_paying_then_pays_in_order() {
        let mut zone = zone(true);
        let recipe_item = give(&mut zone, ME, 1666, 1);
        assert!(run(
            &mut zone,
            1,
            serde_json::json!({"kind": "register_recipe", "recipe_instance": recipe_item})
        )
        .dispositions
        .is_empty());
        give(&mut zone, ME, 1864, 4);
        // 1869 × 2 is missing.
        let (mp_before, words) = (mp(&zone, ME), zone.rng.get_word_pos());
        let tick = run(&mut zone, 2, serde_json::json!({"kind": "craft_item", "recipe_id": 1}));
        assert_eq!(reasons(&tick), vec![RejectReason::MissingMaterials]);
        assert_eq!((mp(&zone, ME), held(&zone, ME, 1864)), (mp_before, 4), "nothing spent");
        assert_eq!(zone.rng.get_word_pos(), words, "no draw");
        wait(&mut zone, 3);
        give(&mut zone, ME, 1869, 2);
        let mp_before = mp(&zone, ME);
        let words = zone.rng.get_word_pos();
        let tick = run(&mut zone, 3, serde_json::json!({"kind": "craft_item", "recipe_id": 1}));
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert!(mp(&zone, ME) <= mp_before - 30 + 1, "MP 30 paid (regen aside)");
        assert_eq!((held(&zone, ME, 1864), held(&zone, ME, 1869)), (0, 0));
        assert_eq!(held(&zone, ME, 17), 500);
        assert!(zone.rng.get_word_pos() >= words + 4, "draws #10 and #11 (two words each)");
        let rows: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == id(ME) => Some(p.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(rows.iter().any(|r| r.operation == ItemOperation::Craft));
        assert_eq!(
            rows.iter()
                .filter(|r| r.operation == ItemOperation::Consume)
                .count(),
            2
        );
        // The Manufacture gate (3 ticks): an immediate second craft refuses.
        let tick = run(&mut zone, 4, serde_json::json!({"kind": "craft_item", "recipe_id": 1}));
        assert_eq!(reasons(&tick), vec![RejectReason::RateLimited]);
    }

    /// §8.2 with a customer (the `PrivateManufactureOrder` maker): the fee moves to the
    /// crafter before the materials and the roll and is kept on failure; the materials are
    /// the customer's and are lost; the product would go to the customer.
    #[test]
    fn a_failed_manufacture_keeps_the_fee_and_loses_the_materials() {
        let mut zone = zone(true);
        // The same rules with recipe 1 at success 1 % (0 is outside the hashed bounds); the
        // deterministic draw fails.
        let mut parts = zone.phase5.as_ref().unwrap().rules.parts().clone();
        parts
            .recipes
            .recipe
            .iter_mut()
            .find(|r| r.id == 1)
            .unwrap()
            .success_rate = 1;
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let mut rules = Phase5Rules::try_new(parts).unwrap();
        rules.bind_items(&bundle.items).unwrap();
        rules.bind_skills(&bundle.skills).unwrap();
        let phase5 = zone.phase5.as_mut().unwrap();
        phase5.canonical.rules_hash = rules.hash();
        phase5.rules = std::sync::Arc::new(rules);
        give(&mut zone, CUSTOMER, 1864, 4);
        give(&mut zone, CUSTOMER, 1869, 2);
        give(&mut zone, CUSTOMER, ADENA, 1000);
        let crafter_adena = held(&zone, ME, ADENA);
        let customer_adena = held(&zone, CUSTOMER, ADENA);
        let mut events = Vec::new();
        let (outcome, affected) = zone
            .make_item(
                zone.next_tick,
                CraftOrder {
                    crafter: id(ME),
                    customer: id(CUSTOMER),
                    fee: 500,
                    recipe: 1,
                    receipt: "receipt:private_manufacture:00000000-0000-0000-0000-000000000009",
                },
                &mut events,
            )
            .unwrap();
        assert_eq!(
            outcome,
            EconomyOutcome::Craft {
                success: false,
                masterwork: false,
                product_template: 17,
                count: 0,
                fee: 500
            }
        );
        assert!(affected.is_empty());
        assert_eq!(held(&zone, ME, ADENA), crafter_adena + 500, "the fee is kept");
        assert_eq!(held(&zone, CUSTOMER, ADENA), customer_adena - 500);
        assert_eq!((held(&zone, CUSTOMER, 1864), held(&zone, CUSTOMER, 1869)), (0, 0), "lost");
        assert_eq!(held(&zone, CUSTOMER, 17), 0);
        // The customer's materials missing: refused before the fee.
        let mut events = Vec::new();
        let before = held(&zone, CUSTOMER, ADENA);
        assert_eq!(
            zone.make_item(
                zone.next_tick,
                CraftOrder {
                    crafter: id(ME),
                    customer: id(CUSTOMER),
                    fee: 1,
                    recipe: 1,
                    receipt: "receipt:private_manufacture:00000000-0000-0000-0000-00000000000a",
                },
                &mut events,
            )
            .unwrap_err(),
            RejectReason::MissingMaterials
        );
        assert_eq!(held(&zone, CUSTOMER, ADENA), before);
        assert!(events.is_empty());
    }

    /// Family C thresholds: `chance_bound(k, 100)` succeeds below `⌈k · 2^64 / 100⌉`
    /// (success rates and masterwork rarities on the oracle vectors).
    #[test]
    fn craft_thresholds_match_family_c() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-vendor.json"
        ))
        .unwrap();
        let c = &fixture["c_vectors"]["thresholds"];
        for family in ["success", "masterwork"] {
            for (k, threshold) in c[family].as_object().unwrap() {
                let k: u64 = k.parse().unwrap();
                let t: u128 = threshold.as_str().unwrap().parse().unwrap();
                if t == 0 {
                    assert!(!crate::domain::zone::wide::chance_bound(k, 100, 0).unwrap());
                } else if t >= 1 << 64 {
                    assert!(crate::domain::zone::wide::chance_bound(k, 100, u64::MAX).unwrap());
                } else {
                    let t = u64::try_from(t).unwrap();
                    assert!(
                        crate::domain::zone::wide::chance_bound(k, 100, t - 1).unwrap(),
                        "{family} {k}"
                    );
                    assert!(
                        !crate::domain::zone::wide::chance_bound(k, 100, t).unwrap(),
                        "{family} {k}"
                    );
                }
            }
        }
    }
}
