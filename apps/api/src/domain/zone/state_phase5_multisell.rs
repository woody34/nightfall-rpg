//! V7 multisell (story 3.7, design §7): `MultisellChoose` as pinned `MultiSellChoose`
//! (`GJ/network/clientpackets/MultiSellChoose.java:96-400`) over a prepared entry
//! (`PreparedEntry.java:33-90`): adena aggregation (all non-tax adena held back and appended
//! as one ingredient last, plus the tax), `maintainEnchantment` (exact-enchant inputs, the
//! enchant and the i-th augmentation copied to the products), lowest-enchant inputs on other
//! lists, and `BlacksmithUseRecipes` (every ingredient consumed, including `maintain` ones).
//! Validated whole, then all or nothing: one Exchange operation per choose.

use std::collections::BTreeMap;

use crate::domain::zone::item_inventory::{InventoryEdit, InventoryLimits};
use crate::domain::zone::phase4_projection::ItemOperation;
use crate::domain::zone::phase5::FloodKind;
use crate::domain::zone::phase5_event::{EconomyOutcome, EconomyScope};
use crate::domain::zone::phase5_rules::MultisellList;
use crate::domain::zone::{
    EntityId, ItemInstance, ItemInstanceId, ItemLocation, Phase5Command, RejectReason, Tick,
    ZoneEvent, ZoneState,
};

/// Pinned `BlacksmithUseRecipes = True` (`GC/character.properties:520`): every ingredient is
/// consumed, `maintainIngredient` or not.
pub(super) const BLACKSMITH_USE_RECIPES: bool = true;
/// `MultiSellChoose.java:109`.
const MAX_AMOUNT: u64 = 5000;
/// `PreparedEntry`: `entryId = template.entryId × 100000 (+ enchant on maintained lists)`.
const ENTRY_STRIDE: u32 = 100_000;
const ADENA: u32 = 57;
const JAVA_INT_MAX: u64 = 2_147_483_647;

fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidLoad
}

/// One prepared ingredient: template, count, the enchant it must carry (maintained armor and
/// weapons), and the `maintainIngredient` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Prepared {
    pub(super) template: u32,
    pub(super) count: u64,
    pub(super) enchant: u32,
    pub(super) maintain: bool,
}

/// `Math.round(count × taxRate)` (Java 7+: `⌊v + 1/2⌋` on the double `v = fl(fl(count) ×
/// rate)`; design E32 rounding site 3).
pub(super) fn multisell_tax(count: u64, rate_bits: u64) -> Result<u64, RejectReason> {
    use super::super::binary64::{Binary64, Rational};
    let rate = Binary64::parse_hex(&format!("{rate_bits:016x}"))
        .and_then(super::super::binary64::Binary64::rational)
        .map_err(invalid)?;
    let count = Rational::integer(i64::try_from(count).map_err(invalid)?)
        .fl()
        .and_then(super::super::binary64::Binary64::rational)
        .map_err(invalid)?;
    let v = count
        .mul(&rate)
        .fl()
        .and_then(super::super::binary64::Binary64::rational)
        .map_err(invalid)?;
    let half = Rational::integer(1)
        .div(&Rational::integer(2))
        .map_err(invalid)?;
    v.add(&half).floor_u64().map_err(invalid)
}

/// `PreparedEntry` (design §7 adena-ingredient merge): non-adena ingredients in XML order
/// (armor/weapon ones carry `enchant` on a maintained list), then one adena ingredient for
/// the summed non-tax adena plus the tax (`apply_taxes` only).
pub(super) fn prepare_ingredients(
    list: &MultisellList,
    entry: usize,
    enchant: u32,
    armor_or_weapon: impl Fn(u32) -> bool,
    tax: impl Fn(u64) -> Result<u64, RejectReason>,
    apply_taxes: bool,
) -> Result<Vec<Prepared>, RejectReason> {
    let entry = list.entries.get(entry).ok_or(RejectReason::NotPermitted)?;
    let mut prepared = Vec::with_capacity(entry.ingredients.len());
    let mut adena: u64 = 0;
    let mut taxed: u64 = 0;
    for ing in &entry.ingredients {
        let template = u32::try_from(ing.id).map_err(|_| RejectReason::ItemCapabilityDeferred)?;
        if template == ADENA {
            if ing.is_tax {
                if apply_taxes {
                    taxed = taxed.saturating_add(tax(ing.count)?);
                }
            } else {
                adena = adena.saturating_add(ing.count);
            }
            continue;
        }
        prepared.push(Prepared {
            template,
            count: ing.count,
            enchant: if list.maintain_enchantment && armor_or_weapon(template) {
                enchant
            } else {
                0
            },
            maintain: ing.maintain,
        });
    }
    let adena = adena.saturating_add(taxed);
    if adena > 0 {
        prepared.push(Prepared {
            template: ADENA,
            count: adena,
            enchant: 0,
            maintain: false,
        });
    }
    Ok(prepared)
}

impl ZoneState {
    /// `MultisellChoose` (design §7).
    #[allow(clippy::too_many_lines)] // The pinned check order and the commit stay in one place.
    pub(super) fn multisell_choose(
        &mut self,
        tick: Tick,
        actor: EntityId,
        command: &Phase5Command,
        seq: Option<u32>,
        events: &mut Vec<ZoneEvent>,
    ) -> Result<(), RejectReason> {
        let Phase5Command::MultisellChoose {
            npc,
            list_id,
            entry_id,
            amount,
            ..
        } = command
        else {
            return Err(RejectReason::InvalidLoad);
        };
        // `:104` flood gate, then `:109` the amount.
        self.flood_gate_v7(tick, actor, FloodKind::MultiSell)?;
        if !(1..=MAX_AMOUNT).contains(amount) {
            return Err(RejectReason::InvalidQuantity);
        }
        let rules =
            std::sync::Arc::clone(&self.phase5.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        let list = rules
            .multisell_list(*list_id)
            .ok_or(RejectReason::NotPermitted)?
            .clone();
        // `:115-123` isNpcAllowed (an NPC-only list needs a listed NPC), then `:127-130` range.
        let bound = self.merchant_binding_v7(*npc)?;
        if !list.npcs.is_empty() && !list.npcs.contains(&bound) {
            return Err(RejectReason::InvalidMerchant);
        }
        self.merchant_range_v7(actor, *npc)?;
        // The prepared entry id: template entry (1-based) × 100000 + enchant.
        let (index, enchant) = (entry_id / ENTRY_STRIDE, entry_id % ENTRY_STRIDE);
        if index == 0 || (!list.maintain_enchantment && enchant != 0) {
            return Err(RejectReason::NotPermitted);
        }
        let index = usize::try_from(index.saturating_sub(1)).map_err(invalid)?;
        let template = list
            .entries
            .get(index)
            .ok_or(RejectReason::NotPermitted)?
            .clone();
        let gear = |t: i64| {
            u32::try_from(t)
                .ok()
                .and_then(|t| {
                    self.phase4
                        .as_ref()?
                        .rules
                        .parts()
                        .items
                        .get(&t)
                        .map(|d| d.kind == "Armor" || d.kind == "Weapon")
                })
                .unwrap_or(false)
        };
        // z-review-7 P3-5: pinned prepares an enchanted entry only for an armor/weapon input.
        if enchant != 0 && !template.ingredients.iter().any(|i| gear(i.id)) {
            return Err(RejectReason::NotPermitted);
        }
        if template.ingredients.iter().any(|i| i.id < 0)
            || template.products.iter().any(|p| p.id < 0)
        {
            // Clan reputation / fame currencies arrive with clans (Phase 7).
            return Err(RejectReason::ItemCapabilityDeferred);
        }
        let item_rules =
            std::sync::Arc::clone(&self.phase4.as_ref().ok_or(RejectReason::InvalidLoad)?.rules);
        let definition = |template: u32| item_rules.parts().items.get(&template);
        let armor_or_weapon =
            |t: u32| definition(t).is_some_and(|d| d.kind == "Armor" || d.kind == "Weapon");
        // Taxes apply only for an NPC in town of a castle with an owner; no castle has one in
        // Phase 5, so the tax is 0 (the formula is kept and tested).
        let ingredients = prepare_ingredients(
            &list,
            index,
            enchant,
            armor_or_weapon,
            |count| multisell_tax(count, 0),
            false,
        )?;
        let mut products = Vec::with_capacity(template.products.len());
        for product in &template.products {
            let id = u32::try_from(product.id).map_err(|_| RejectReason::ItemCapabilityDeferred)?;
            let def = definition(id).ok_or(RejectReason::InvalidLoad)?;
            let total = product
                .count
                .checked_mul(*amount)
                .filter(|n| *n <= JAVA_INT_MAX)
                .ok_or(RejectReason::InvalidQuantity)?;
            products.push((id, total, def.stackable, def.kind == "Armor" || def.kind == "Weapon"));
        }
        // `:135` a non-stackable entry takes amount 1.
        if *amount > 1 && products.iter().any(|(_, _, stackable, _)| !stackable) {
            return Err(RejectReason::InvalidQuantity);
        }
        // `:168-212` the merged possession check, on items not equipped.
        let player = self
            .phase4
            .as_ref()
            .and_then(|items| items.canonical.players.get(&actor))
            .ok_or(RejectReason::InvalidLoad)?;
        let mut merged: Vec<Prepared> = Vec::new();
        for ing in &ingredients {
            if let Some(m) = merged
                .iter_mut()
                .find(|m| m.template == ing.template && m.enchant == ing.enchant)
            {
                m.count = m
                    .count
                    .checked_add(ing.count)
                    .filter(|n| *n <= JAVA_INT_MAX)
                    .ok_or(RejectReason::InvalidQuantity)?;
            } else {
                merged.push(*ing);
            }
        }
        for ing in &merged {
            let total = ing
                .count
                .checked_mul(*amount)
                .filter(|n| *n <= JAVA_INT_MAX)
                .ok_or(RejectReason::InvalidQuantity)?;
            let required = if BLACKSMITH_USE_RECIPES || !ing.maintain {
                total
            } else {
                ing.count
            };
            let owned: u64 = player
                .inventory
                .values()
                .filter(|i| {
                    i.template_id == ing.template
                        && i.location == ItemLocation::Inventory
                        && (!list.maintain_enchantment || i.enchant == ing.enchant)
                })
                .map(|i| i.count)
                .fold(0, u64::saturating_add);
            if owned < required {
                return Err(if ing.template == ADENA {
                    RejectReason::InsufficientAdena
                } else {
                    RejectReason::InsufficientItems
                });
            }
        }
        // `:226-330` the instances each prepared ingredient takes, in prepared order.
        let mut scratch = player.inventory.clone();
        let mut takes: Vec<InventoryEdit> = Vec::new();
        let mut augments: Vec<u32> = Vec::new();
        for ing in &ingredients {
            if !BLACKSMITH_USE_RECIPES && ing.maintain {
                continue;
            }
            let total = ing.count.saturating_mul(*amount);
            let stackable = definition(ing.template)
                .ok_or(RejectReason::InvalidLoad)?
                .stackable;
            if stackable {
                take_stackable(&mut scratch, ing.template, total)?;
                takes.push(InventoryEdit::TakeTemplate {
                    template: ing.template,
                    count: total,
                    preferred: None,
                });
                continue;
            }
            for _ in 0..total {
                let pick =
                    pick_unit(&scratch, ing.template, list.maintain_enchantment, ing.enchant)
                        .ok_or(RejectReason::InsufficientItems)?;
                if list.maintain_enchantment {
                    if let Some(augment) = pick.augment_id {
                        augments.push(augment);
                    }
                }
                scratch.remove(&pick.id);
                takes.push(InventoryEdit::TakeInstance {
                    instance: pick.id,
                    count: 1,
                });
            }
        }
        // `:151-155` weight and slots for the products (after the inputs, design §7).
        let additions: Vec<(u32, u64)> = products.iter().map(|(t, c, _, _)| (*t, *c)).collect();
        self.room_in_v7(actor, &scratch, &additions)?;
        self.caps_for_v7(actor, &additions)?;
        let fresh: u64 = products
            .iter()
            .map(|(_, count, stackable, _)| if *stackable { 1 } else { *count })
            .fold(0, u64::saturating_add);
        let available = self.phase4.as_ref().map_or(0, |items| {
            let (_, next, end) = items.canonical.allocator.bounds();
            end.saturating_sub(next)
        });
        if fresh > available {
            return Err(RejectReason::InventoryFull);
        }
        // Commit: inputs in prepared order, then the products (one Add per non-stackable unit).
        let limits = InventoryLimits::from_player(player);
        let mut edits = takes;
        let consumed = edits.len();
        for (template, count, stackable, _) in &products {
            if *stackable {
                edits.push(InventoryEdit::Add {
                    template: *template,
                    count: *count,
                });
            } else {
                for _ in 0..*count {
                    edits.push(InventoryEdit::Add {
                        template: *template,
                        count: 1,
                    });
                }
            }
        }
        let receipt = format!("receipt:multisell_choose:{}", command.request_key());
        let mut per_edit = self
            .economy_commit(tick, actor, &edits, limits, events)?
            .into_iter();
        for changes in per_edit.by_ref().take(consumed) {
            for change in changes {
                self.push_economy_consume(tick, actor, change, &receipt, events)?;
            }
        }
        // Maintained lists copy the enchant to armor/weapon products and the i-th augment to
        // the i-th non-stackable product unit (`:352-363`); elementals are a no-op (B10).
        let mut unit: usize;
        let mut affected = Vec::new();
        let mut produced: BTreeMap<ItemInstanceId, ItemInstance> = BTreeMap::new();
        let mut rows = Vec::new();
        for (_, count, stackable, gear) in &products {
            // z-review-7 P3-4: pinned indexes the augments per product line.
            unit = 0;
            let edits_for = if *stackable { 1 } else { *count };
            for _ in 0..edits_for {
                let changes = per_edit.next().ok_or(RejectReason::InvalidLoad)?;
                for mut change in changes {
                    if list.maintain_enchantment && !*stackable && change.before.is_none() {
                        if let Some(after) = change.after.as_mut() {
                            if *gear {
                                after.enchant = enchant;
                            }
                            if let Some(augment) = augments.get(unit) {
                                after.augment_id = Some(*augment);
                            }
                            after.validate().map_err(invalid)?;
                            produced.insert(after.id, after.clone());
                        }
                        unit = unit.saturating_add(1);
                    }
                    rows.push(change);
                }
            }
        }
        if !produced.is_empty() {
            let player = self
                .phase4
                .as_mut()
                .and_then(|items| items.canonical.players.get_mut(&actor))
                .ok_or(RejectReason::InvalidLoad)?;
            for (id, item) in produced {
                player.inventory.insert(id, item);
            }
            self.rebuild_v5(actor).map_err(invalid)?;
        }
        for change in rows {
            affected.push(self.push_economy_grant(
                tick,
                actor,
                change,
                ItemOperation::Exchange,
                &receipt,
                events,
            )?);
        }
        self.economy_receipt(
            tick,
            actor,
            command,
            seq,
            EconomyScope::MultisellChoose,
            affected,
            EconomyOutcome::Multisell {
                list_id: *list_id,
                entry_id: *entry_id,
                amount: *amount,
            },
            events,
        );
        Ok(())
    }
}

/// The non-stackable unit a choose takes (`:262-300`): on a maintained list the first
/// instance (ascending id) with exactly the enchant; otherwise the first instance with the
/// lowest enchant. Equipped items are never taken.
pub(super) fn pick_unit(
    inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
    template: u32,
    maintained: bool,
    enchant: u32,
) -> Option<ItemInstance> {
    inventory
        .values()
        .filter(|i| i.template_id == template && i.location == ItemLocation::Inventory)
        .filter(|i| !maintained || i.enchant == enchant)
        .min_by_key(|i| (if maintained { 0 } else { i.enchant }, i.id))
        .cloned()
}

/// Take `count` of a stackable template from the scratch inventory (ascending ids).
fn take_stackable(
    scratch: &mut BTreeMap<ItemInstanceId, ItemInstance>,
    template: u32,
    count: u64,
) -> Result<(), RejectReason> {
    let mut remaining = count;
    let ids: Vec<ItemInstanceId> = scratch
        .values()
        .filter(|i| i.template_id == template)
        .map(|i| i.id)
        .collect();
    for id in ids {
        let item = scratch.get_mut(&id).ok_or(RejectReason::InvalidLoad)?;
        let taken = item.count.min(remaining);
        item.count = item.count.saturating_sub(taken);
        remaining = remaining.saturating_sub(taken);
        if item.count == 0 {
            scratch.remove(&id);
        }
        if remaining == 0 {
            return Ok(());
        }
    }
    Err(RejectReason::InsufficientItems)
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
    use crate::domain::zone::item_inventory::inventory_totals;
    use crate::domain::zone::phase5_rules::Phase5Rules;
    use crate::domain::zone::{
        EntityKind, ItemCreationOrigin, SessionGeneration, Speed, Vec2Fixed, ZoneCommand, ZoneInput,
    };

    use super::super::item_materialize::tests::{fresh_zone_and_kit_load, spawn_kit};
    use super::super::Spawned;

    const P1: u128 = 71;

    fn id(n: u128) -> EntityId {
        EntityId::from_uuid(uuid::Uuid::from_u128(n))
    }

    fn rules() -> std::sync::Arc<Phase5Rules> {
        crate::infrastructure::economy_data::embedded_phase5_rules().unwrap()
    }

    fn vectors() -> serde_json::Value {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/economy-oracle-vendor.json"
        ))
        .unwrap();
        fixture["m_vectors"].clone()
    }

    /// Family M: the prepared ingredient order (adena held back, appended last), the tax
    /// half-up rounding, and `BlacksmithUseRecipes` consuming the `maintain` ingredients.
    #[test]
    fn prepared_entries_tax_and_maintain_ingredient_match_family_m() {
        let rules = rules();
        let m = vectors();
        for example in m["adena_aggregation"]["examples"].as_array().unwrap() {
            let list = rules
                .multisell_list(u32::try_from(example["list"].as_u64().unwrap()).unwrap())
                .unwrap();
            let entry = usize::try_from(example["entry"].as_u64().unwrap()).unwrap() - 1;
            let prepared =
                prepare_ingredients(list, entry, 0, |_| false, |_| Ok(0), false).unwrap();
            let expected: Vec<(u32, u64)> = example["prepared"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| {
                    (
                        u32::try_from(i["id"].as_u64().unwrap()).unwrap(),
                        i["count"].as_u64().unwrap(),
                    )
                })
                .collect();
            let got: Vec<(u32, u64)> = prepared.iter().map(|p| (p.template, p.count)).collect();
            assert_eq!(got, expected, "{example}");
        }
        for v in m["tax_half_up"].as_array().unwrap() {
            let bits = u64::from_str_radix(v["rate_bits"].as_str().unwrap(), 16).unwrap();
            assert_eq!(
                multisell_tax(v["count"].as_u64().unwrap(), bits).unwrap(),
                v["tax"].as_u64().unwrap(),
                "{v}"
            );
        }
        for v in m["maintain_ingredient_consumed"].as_array().unwrap() {
            let list = rules
                .multisell_list(u32::try_from(v["list"].as_u64().unwrap()).unwrap())
                .unwrap();
            let id = v["id"].as_i64().unwrap();
            assert!(list
                .entries
                .iter()
                .any(|e| e.ingredients.iter().any(|i| i.id == id && i.maintain)));
            let (count, amount) = (v["count"].as_u64().unwrap(), v["amount"].as_u64().unwrap());
            let required = if BLACKSMITH_USE_RECIPES {
                count * amount
            } else {
                count
            };
            assert_eq!(required, v["required"].as_u64().unwrap(), "{v}");
        }
        assert_eq!(
            rules
                .parts()
                .multisell
                .list
                .iter()
                .filter(|l| l.maintain_enchantment)
                .count(),
            usize::try_from(m["maintain_enchantment_lists"].as_u64().unwrap()).unwrap()
        );
    }

    /// Family M `maintain_enchantment`: a maintained list takes the exact-enchant instance
    /// (the first by id) and copies its enchant and augment; another list takes the
    /// lowest-enchant instance.
    #[test]
    fn maintained_lists_take_the_exact_enchant_and_others_the_lowest() {
        let m = vectors();
        let v = &m["maintain_enchantment"];
        let mut inventory = BTreeMap::new();
        for row in v["inventory"].as_array().unwrap() {
            let id = ItemInstanceId::new(row["id"].as_u64().unwrap()).unwrap();
            inventory.insert(
                id,
                ItemInstance {
                    creation_origin: None,
                    id,
                    template_id: u32::try_from(row["template"].as_u64().unwrap()).unwrap(),
                    count: 1,
                    location: ItemLocation::Inventory,
                    slot: None,
                    enchant: u32::try_from(row["enchant"].as_u64().unwrap()).unwrap(),
                    augment_id: row["augment"].as_u64().map(|a| u32::try_from(a).unwrap()),
                    charges: 0,
                    created_tick: Tick(0),
                    revision: 0,
                },
            );
        }
        let lowest = pick_unit(&inventory, 9417, false, 0).unwrap();
        assert_eq!(lowest.id.get(), v["taken_lowest"][0]["id"].as_u64().unwrap());
        let maintained = pick_unit(&inventory, 9417, true, 3).unwrap();
        assert_eq!(maintained.id.get(), v["taken_maintained"][0]["id"].as_u64().unwrap());
        assert_eq!(maintained.augment_id.map(u64::from), v["products"][0]["augment"].as_u64());
    }

    /// A zone with the kit player and a fixture multisell NPC bound to `NPC` beside it.
    fn zone() -> (ZoneState, EntityId) {
        let (mut zone, mut load, _) = fresh_zone_and_kit_load(true);
        load.phase5 = Some(crate::domain::zone::Phase5Checkpoint {
            schema_version: 1,
            rules_hash: zone.phase5.as_ref().unwrap().rules.hash_hex(),
            recipes: crate::domain::zone::RecipeBook::default(),
        });
        let tick = zone
            .run_tick(zone.draft(vec![spawn_kit(id(P1), load)]))
            .unwrap();
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        // The bound P5-D29 fixture: the player stands on its tile.
        let tiles = (101, 98);
        let here = Vec2Fixed::from_tiles(tiles.0, tiles.1);
        let from = zone.entities[&id(P1)].pos;
        zone.entities.get_mut(&id(P1)).unwrap().pos = here;
        zone.aoi.relocate(id(P1), from, here);
        let npc = id(6_000);
        zone.spawn(
            Tick(zone.next_tick.0 - 1),
            Spawned {
                id: npc,
                kind: EntityKind::Npc,
                name: "exchanger",
                pos: Vec2Fixed::from_tiles(tiles.0, tiles.1),
                speed: Speed::from_milli_tiles_per_tick(0),
                generation: SessionGeneration::default(),
                combat: None,
            },
        )
        .unwrap();
        (zone, npc)
    }

    fn give(zone: &mut ZoneState, template: u32, count: u64, enchant: u32) -> ItemInstanceId {
        let items = zone.phase4.as_mut().unwrap();
        let new = items.canonical.allocator.allocate().unwrap();
        let rules = std::sync::Arc::clone(&items.rules);
        let player = items.canonical.players.get_mut(&id(P1)).unwrap();
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
                enchant,
                augment_id: None,
                charges: 0,
                created_tick: Tick(0),
                revision: player.inventory_revision,
            },
        );
        player.weight = inventory_totals(&player.inventory, &rules).unwrap().2;
        zone.rebuild_v5(id(P1)).unwrap();
        new
    }

    fn held(zone: &ZoneState, template: u32) -> Vec<(u64, u32)> {
        zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .values()
            .filter(|i| i.template_id == template)
            .map(|i| (i.count, i.enchant))
            .collect()
    }

    fn choose(
        zone: &mut ZoneState,
        key: u128,
        npc: EntityId,
        list: u32,
        entry: u32,
        amount: u64,
    ) -> crate::domain::zone::AppliedTick {
        let command: Phase5Command = serde_json::from_value(serde_json::json!({
            "kind": "multisell_choose",
            "entity": id(P1),
            "request_key": uuid::Uuid::from_u128(key),
            "npc": npc,
            "list_id": list,
            "entry_id": entry,
            "amount": amount,
        }))
        .unwrap();
        zone.run_tick(zone.draft(vec![ZoneInput::session(
            id(P1),
            SessionGeneration(1),
            u32::try_from(key).unwrap(),
            ZoneCommand::Phase5(Box::new(command)),
        )]))
        .unwrap()
    }

    fn reasons(tick: &crate::domain::zone::AppliedTick) -> Vec<RejectReason> {
        tick.dispositions.iter().map(|d| d.reason).collect()
    }

    /// §7 end to end: list 625 entry 2 (259 × 9599 + 12,000,000 adena → 10559): the amount
    /// bounds, the possession check, then one choose pays both (adena last) and grants the
    /// product as an `exchange` row.
    #[test]
    fn multisell_choose_pays_the_prepared_ingredients_and_grants_the_product() {
        let (mut zone, npc) = zone();
        let m = vectors();
        for (key, (amount, expected)) in (1..).zip(m["amount_bounds"].as_object().unwrap()) {
            let tick = choose(&mut zone, key, npc, 625, 200_000, amount.parse().unwrap());
            let got = reasons(&tick);
            if expected.is_null() {
                assert_ne!(got, vec![RejectReason::InvalidQuantity], "{amount}");
            } else {
                assert_eq!(got, vec![RejectReason::InvalidQuantity], "{amount}");
            }
            zone.run_tick(zone.draft(Vec::new())).unwrap();
        }
        let tick = choose(&mut zone, 20, npc, 625, 200_000, 1);
        assert_eq!(reasons(&tick), vec![RejectReason::InsufficientItems]);
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        give(&mut zone, 9599, 300, 0);
        let adena_before: u64 = held(&zone, 57).iter().map(|(c, _)| c).sum();
        give(&mut zone, 57, 12_000_000, 0);
        let tick = choose(&mut zone, 21, npc, 625, 200_000, 1);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(held(&zone, 9599), vec![(41, 0)]);
        let adena_after: u64 = held(&zone, 57).iter().map(|(c, _)| c).sum();
        assert_eq!(adena_after, adena_before);
        assert_eq!(held(&zone, 10559), vec![(1, 0)]);
        let rows: Vec<_> = tick
            .events
            .iter()
            .filter_map(|e| match e {
                ZoneEvent::ItemCheckpoint(p) if p.entity == id(P1) => Some(p.ledger.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(
            rows.iter()
                .filter(|r| r.operation == ItemOperation::Consume)
                .count()
                >= 2
        );
        assert!(rows
            .iter()
            .any(|r| r.operation == ItemOperation::Exchange && r.before.is_none()));
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(
            reasons(&choose(&mut zone, 23, npc, 999_999, 100_000, 1)),
            vec![RejectReason::NotPermitted]
        );
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(
            reasons(&choose(&mut zone, 24, npc, 625, 200_003, 1)),
            vec![RejectReason::NotPermitted]
        );
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(
            reasons(&choose(&mut zone, 25, id(P1), 625, 200_000, 1)),
            vec![RejectReason::InvalidMerchant]
        );
        // The 1-tick MultiSell gate: two chooses in one tick, the second refuses.
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        let first: Phase5Command = serde_json::from_value(serde_json::json!({
            "kind": "multisell_choose", "entity": id(P1), "request_key": uuid::Uuid::from_u128(26),
            "npc": npc, "list_id": 625, "entry_id": 200_000, "amount": 1,
        }))
        .unwrap();
        let mut second = first.clone();
        if let Phase5Command::MultisellChoose { request_key, .. } = &mut second {
            *request_key = uuid::Uuid::from_u128(27);
        }
        let tick = zone
            .run_tick(zone.draft(vec![
                ZoneInput::session(
                    id(P1),
                    SessionGeneration(1),
                    26,
                    ZoneCommand::Phase5(Box::new(first)),
                ),
                ZoneInput::session(
                    id(P1),
                    SessionGeneration(1),
                    27,
                    ZoneCommand::Phase5(Box::new(second)),
                ),
            ]))
            .unwrap();
        assert_eq!(
            reasons(&tick),
            vec![RejectReason::InsufficientItems, RejectReason::RateLimited]
        );
    }

    /// §7 maintainEnchantment end to end (list 616 entry 1, 9417 → 9416): the prepared entry
    /// `100000 + 3` takes the +3 input and yields a +3 product; the +0 input stays.
    #[test]
    fn maintained_choose_keeps_the_enchant() {
        let (mut zone, npc) = zone();
        give(&mut zone, 9417, 1, 0);
        let plus3 = give(&mut zone, 9417, 1, 3);
        let tick = choose(&mut zone, 30, npc, 616, 100_003, 1);
        assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
        assert_eq!(held(&zone, 9417), vec![(1, 0)]);
        assert_eq!(held(&zone, 9416), vec![(1, 3)]);
        assert!(!zone.phase4.as_ref().unwrap().canonical.players[&id(P1)]
            .inventory
            .contains_key(&plus3));
        zone.run_tick(zone.draft(Vec::new())).unwrap();
        assert_eq!(
            reasons(&choose(&mut zone, 31, npc, 616, 100_005, 1)),
            vec![RejectReason::InsufficientItems]
        );
    }
}
