//! Integer inventory staging. The caller publishes only a complete successful plan.
use super::{
    ItemError, ItemInstance, ItemInstanceId, ItemInstanceRange, ItemLocation, Phase4Rules, Tick,
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct InventoryLimits {
    ordinary: u32,
    quest: u32,
    max_weight: i64,
}
impl InventoryLimits {
    /// Live mutations use retained, authoritatively folded capacities (D39).
    pub fn from_player(player: &super::Phase4Player) -> Self {
        // A negative retained limit still permits removals; any addition fails capacity.
        Self {
            ordinary: player.ordinary_capacity,
            quest: player.quest_capacity,
            max_weight: player.max_load,
        }
    }
    /// Slot ceilings come from the immutable source policy; the effective load getter
    /// supplies the checked weight ceiling after attributes and modifiers have folded.
    pub fn from_rules(
        rules: &Phase4Rules,
        dwarf: bool,
        max_weight: i64,
    ) -> Result<Self, ItemError> {
        if max_weight < 0 {
            return Err(ItemError("negative inventory weight capacity"));
        }
        let capacities = &rules.parts().capacities;
        Ok(Self {
            ordinary: if dwarf {
                capacities.dwarf_inventory
            } else {
                capacities.inventory
            },
            quest: capacities.quest_inventory,
            max_weight,
        })
    }
}

/// `L2PcInstance.getInventoryLimit` adds `trunc(calcStat(INV_LIM, 0))` to the race
/// base; quest inventory has its own configured ceiling and no stat modifier.
pub fn computed_capacities(
    calculator: &super::stat_functions::StatCalculator,
    rules: &Phase4Rules,
    dwarf: bool,
    conditions: super::stat_functions::FunctionConditions<'_>,
) -> Result<(u32, u32), super::StatError> {
    let policy = &rules.parts().capacities;
    let base = if dwarf {
        policy.dwarf_inventory
    } else {
        policy.inventory
    };
    let bonus = calculator.fold("inventoryLimit", 0, conditions)? / super::Q;
    let ordinary = i128::from(base)
        .checked_add(i128::from(bonus))
        .filter(|value| *value >= 0 && *value <= i128::from(i32::MAX))
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(super::StatError::Overflow)?;
    Ok((ordinary, policy.quest_inventory))
}
/// Canonical tick/revision shared by this staged mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct InventoryBoundary {
    pub origin: super::ItemCreationOrigin,
    pub tick: Tick,
    pub revision: u64,
}
/// Authoritative low-level requests produced by validated commands/cast payments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum InventoryEdit {
    Add {
        template: u32,
        count: u64,
    },
    TakeInstance {
        instance: ItemInstanceId,
        count: u64,
    },
    TakeTemplate {
        template: u32,
        count: u64,
        preferred: Option<ItemInstanceId>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct InventoryChange {
    pub before: Option<ItemInstance>,
    pub after: Option<ItemInstance>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct InventoryMutation {
    pub inventory: BTreeMap<ItemInstanceId, ItemInstance>,
    pub allocator: ItemInstanceRange,
    pub changes: Vec<InventoryChange>,
    pub weight: i64,
}
impl InventoryMutation {
    /// Source inventory size includes equipped items, with a separate quest pool
    /// (PcInventory.java:772-776,864-868). Composite exchanges pay inputs before outputs.
    pub fn prepare(
        inventory: &BTreeMap<ItemInstanceId, ItemInstance>,
        allocator: ItemInstanceRange,
        edits: &[InventoryEdit],
        rules: &Phase4Rules,
        limits: InventoryLimits,
        boundary: InventoryBoundary,
    ) -> Result<Self, ItemError> {
        let InventoryBoundary {
            tick,
            revision,
            origin,
        } = boundary;
        if i64::try_from(revision).is_err() || i64::try_from(tick.0).is_err() {
            return Err(ItemError("inventory revision overflow"));
        }
        origin.validate()?;
        let (_, _, weight) = inventory_totals(inventory, rules)?;
        if inventory.values().any(|item| item.revision > revision) {
            return Err(ItemError("inventory revision regressed"));
        }
        let mut plan = Self {
            inventory: inventory.clone(),
            allocator,
            changes: Vec::new(),
            weight,
        };
        let mut added = false;
        for edit in edits {
            match *edit {
                InventoryEdit::Add { template, count } => {
                    plan.add(template, count, rules, limits, boundary)?;
                    added = true;
                },
                InventoryEdit::TakeInstance { instance, count } => {
                    plan.take(instance, count, revision)?;
                },
                InventoryEdit::TakeTemplate {
                    template,
                    count,
                    preferred,
                } => {
                    checked_count(count)?;
                    let mut ids: Vec<_> = plan
                        .inventory
                        .values()
                        .filter(|i| i.template_id == template)
                        .map(|i| i.id)
                        .collect();
                    if let Some(id) = preferred {
                        if let Some(index) = ids.iter().position(|candidate| *candidate == id) {
                            ids.remove(index);
                            ids.insert(0, id);
                        }
                    }
                    let mut remaining = count;
                    for id in ids {
                        let amount = plan
                            .inventory
                            .get(&id)
                            .ok_or(ItemError("missing payment item"))?
                            .count
                            .min(remaining);
                        if amount > 0 {
                            plan.take(id, amount, revision)?;
                            remaining = remaining
                                .checked_sub(amount)
                                .ok_or(ItemError("payment underflow"))?;
                        }
                        if remaining == 0 {
                            break;
                        }
                    }
                    if remaining != 0 {
                        return Err(ItemError("insufficient item payment"));
                    }
                },
            }
        }
        let (ordinary, quest, weight) = inventory_totals(&plan.inventory, rules)?;
        if added
            && (ordinary > u64::from(limits.ordinary)
                || quest > u64::from(limits.quest)
                || weight > limits.max_weight)
        {
            return Err(ItemError("inventory capacity or weight exceeded"));
        }
        plan.weight = weight;
        Ok(plan)
    }
    fn add(
        &mut self,
        template: u32,
        count: u64,
        rules: &Phase4Rules,
        limits: InventoryLimits,
        boundary: InventoryBoundary,
    ) -> Result<(), ItemError> {
        let InventoryBoundary {
            tick,
            revision,
            origin,
        } = boundary;
        checked_count(count)?;
        let definition = rules
            .parts()
            .items
            .get(&template)
            .ok_or(ItemError("unknown item template"))?;
        if !rules
            .parts()
            .capabilities
            .get(&template)
            .is_some_and(|cap| cap.storable)
        {
            return Err(ItemError("item storage is deferred"));
        }
        if definition.stackable {
            if let Some(existing) = self
                .inventory
                .values()
                .find(|item| {
                    item.template_id == template
                        && item.enchant == 0
                        && item.augment_id.is_none()
                        && item.charges == 0
                })
                .cloned()
            {
                let mut after = existing.clone();
                after.count = after
                    .count
                    .checked_add(count)
                    .ok_or(ItemError("item count overflow"))?;
                checked_count(after.count)?;
                if after.count > count_cap(template, true) {
                    return Err(ItemError("item count exceeds pinned cap"));
                }
                after.revision = revision;
                self.inventory.insert(after.id, after.clone());
                self.changes.push(InventoryChange {
                    before: Some(existing),
                    after: Some(after),
                });
                return Ok(());
            }
        }
        let (ordinary, quest, _) = inventory_totals(&self.inventory, rules)?;
        let is_quest = definition
            .flags
            .get("is_questitem")
            .copied()
            .ok_or(ItemError("missing quest classification"))?;
        let (used, limit) = if is_quest {
            (quest, limits.quest)
        } else {
            (ordinary, limits.ordinary)
        };
        if definition.stackable && count > count_cap(template, true) {
            return Err(ItemError("item count exceeds pinned cap"));
        }
        let slots = if definition.stackable { 1 } else { count };
        if used.checked_add(slots).is_none_or(|n| n > u64::from(limit)) {
            return Err(ItemError("inventory capacity exceeded"));
        }
        for _ in 0..slots {
            let id = self.allocator.allocate()?;
            let item = ItemInstance {
                creation_origin: Some(origin),
                id,
                template_id: template,
                count: if definition.stackable { count } else { 1 },
                location: ItemLocation::Inventory,
                slot: None,
                enchant: 0,
                augment_id: None,
                charges: 0,
                created_tick: tick,
                revision,
            };
            item.validate()?;
            if self.inventory.insert(id, item.clone()).is_some() {
                return Err(ItemError("allocated item identity collision"));
            }
            self.changes.push(InventoryChange {
                before: None,
                after: Some(item),
            });
        }
        Ok(())
    }
    fn take(&mut self, id: ItemInstanceId, count: u64, revision: u64) -> Result<(), ItemError> {
        checked_count(count)?;
        let before = self
            .inventory
            .get(&id)
            .ok_or(ItemError("item is not owned"))?
            .clone();
        let remaining = before
            .count
            .checked_sub(count)
            .ok_or(ItemError("insufficient item payment"))?;
        let after = if remaining == 0 {
            self.inventory.remove(&id);
            None
        } else {
            let mut item = before.clone();
            item.count = remaining;
            item.revision = revision;
            self.inventory.insert(id, item.clone());
            Some(item)
        };
        self.changes.push(InventoryChange {
            before: Some(before),
            after,
        });
        Ok(())
    }
}
/// Pinned `MaxAdena` (GAME@abfde049 character.properties:671).
pub const MAX_ADENA: u64 = 99_900_000_000;
/// L2ItemInstance.java:422 caps every other stack at `Integer.MAX_VALUE`.
pub const MAX_STACK: u64 = 2_147_483_647;
const ADENA: u32 = 57;

/// D68: a stack may never exceed its pinned per-template maximum; acceptance refuses the
/// grant instead of reproducing the source's silent clamp (L2ItemInstance.java:418-426).
pub fn count_cap(template: u32, stackable: bool) -> u64 {
    if !stackable {
        1
    } else if template == ADENA {
        MAX_ADENA
    } else {
        MAX_STACK
    }
}

fn checked_count(count: u64) -> Result<(), ItemError> {
    if count == 0 || i64::try_from(count).is_err() {
        Err(ItemError("item count outside positive bigint"))
    } else {
        Ok(())
    }
}
/// Checked weight and slot totals, with nonstackable cardinality verified from source.
pub fn inventory_totals(
    items: &BTreeMap<ItemInstanceId, ItemInstance>,
    rules: &Phase4Rules,
) -> Result<(u64, u64, i64), ItemError> {
    let (mut ordinary, mut quest, mut weight) = (0_u64, 0_u64, 0_i128);
    for (id, item) in items {
        item.validate()?;
        let definition = rules
            .parts()
            .items
            .get(&item.template_id)
            .ok_or(ItemError("unknown owned template"))?;
        if !rules
            .parts()
            .capabilities
            .get(&item.template_id)
            .is_some_and(|cap| cap.storable)
            || *id != item.id
            || (!definition.stackable && item.count != 1)
        {
            return Err(ItemError("invalid owned item scalar"));
        }
        let is_quest = definition
            .flags
            .get("is_questitem")
            .copied()
            .ok_or(ItemError("missing quest classification"))?;
        let slots = if is_quest { &mut quest } else { &mut ordinary };
        *slots = slots
            .checked_add(1)
            .ok_or(ItemError("inventory size overflow"))?;
        weight = weight
            .checked_add(
                i128::from(definition.weight)
                    .checked_mul(i128::from(item.count))
                    .ok_or(ItemError("inventory weight overflow"))?,
            )
            .ok_or(ItemError("inventory weight overflow"))?;
    }
    Ok((
        ordinary,
        quest,
        i64::try_from(weight).map_err(|_| ItemError("inventory weight outside bigint"))?,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infrastructure::item_data::embedded_phase4_bundle;

    fn seed(id: u64, template_id: u32, count: u64) -> ItemInstance {
        ItemInstance {
            creation_origin: None,
            id: ItemInstanceId::new(id).unwrap(),
            template_id,
            count,
            location: ItemLocation::Inventory,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: Tick(0),
            revision: 1,
        }
    }
    fn ordinary(rules: &Phase4Rules) -> InventoryLimits {
        InventoryLimits::from_rules(rules, false, i64::MAX).unwrap()
    }
    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        /// Design §12.2: positive counts, identities never reused, grant/consume conservation
        /// and all-or-nothing refusal over legal-and-illegal edit sequences.
        #[test]
        fn inventory_sequences_conserve_counts_and_never_reuse_identity(
            steps in proptest::collection::vec(
                proptest::collection::vec((0_usize..3, 0_u8..3, 1_u64..6), 1..4),
                1..12,
            )
        ) {
            use super::super::{ItemCreationOrigin, ZoneId};
            let bundle = embedded_phase4_bundle().unwrap();
            let rules = &bundle.items;
            let templates = [57_u32, 3031, 71];
            let mut inventory = BTreeMap::new();
            let mut allocator = ItemInstanceRange::new(100, 100, 100_000).unwrap();
            let mut seen = std::collections::BTreeSet::new();
            let total = |inventory: &BTreeMap<ItemInstanceId, ItemInstance>, template| {
                inventory
                    .values()
                    .filter(|item: &&ItemInstance| item.template_id == template)
                    .map(|item| u128::from(item.count))
                    .sum::<u128>()
            };
            for (revision, step) in (2_u64..).zip(steps) {
                let edits: Vec<InventoryEdit> = step
                    .iter()
                    .map(|&(template, kind, count)| {
                        let template = *templates.get(template).unwrap();
                        match kind {
                            0 => InventoryEdit::Add { template, count },
                            1 => InventoryEdit::TakeTemplate { template, count, preferred: None },
                            _ => inventory
                                .keys()
                                .next()
                                .map_or(InventoryEdit::Add { template, count }, |instance| {
                                    InventoryEdit::TakeInstance { instance: *instance, count }
                                }),
                        }
                    })
                    .collect();
                let before = inventory.clone();
                let Ok(plan) = InventoryMutation::prepare(
                    &inventory,
                    allocator,
                    &edits,
                    rules,
                    ordinary(rules),
                    InventoryBoundary {
                        tick: Tick(revision),
                        revision,
                        origin: ItemCreationOrigin { zone: ZoneId(1), epoch: 1 },
                    },
                ) else {
                    // A refused sequence is pure: nothing to roll back, inputs untouched.
                    proptest::prop_assert_eq!(&inventory, &before);
                    continue;
                };
                for template in templates {
                    let mut expected = i128::try_from(total(&before, template)).unwrap();
                    for edit in &edits {
                        match *edit {
                            InventoryEdit::Add { template: t, count } if t == template => {
                                expected = expected.checked_add(i128::from(count)).unwrap();
                            },
                            InventoryEdit::TakeTemplate { template: t, count, .. } if t == template => {
                                expected = expected.checked_sub(i128::from(count)).unwrap();
                            },
                            InventoryEdit::TakeInstance { instance, count }
                                if before.get(&instance).or(plan.inventory.get(&instance))
                                    .is_some_and(|item| item.template_id == template) => {
                                expected = expected.checked_sub(i128::from(count)).unwrap();
                            },
                            InventoryEdit::Add { .. }
                            | InventoryEdit::TakeTemplate { .. }
                            | InventoryEdit::TakeInstance { .. } => {},
                        }
                    }
                    proptest::prop_assert_eq!(
                        i128::try_from(total(&plan.inventory, template)).unwrap(),
                        expected
                    );
                }
                for item in plan.inventory.values() {
                    proptest::prop_assert!(item.count > 0);
                    if !before.contains_key(&item.id) {
                        // Fresh identities come from the monotone range and were never held.
                        proptest::prop_assert!(seen.insert(item.id));
                    }
                }
                for id in before.keys() {
                    seen.insert(*id);
                }
                inventory = plan.inventory;
                allocator = plan.allocator;
            }
        }
    }

    #[test]
    fn d68_stack_caps_refuse_at_acceptance_without_partial_change() {
        use super::super::{ItemCreationOrigin, ZoneId};
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let items = &rules.parts().items;
        assert!(items.get(&57).unwrap().stackable && items.get(&3031).unwrap().stackable);
        let boundary = InventoryBoundary {
            tick: Tick(1),
            revision: 2,
            origin: ItemCreationOrigin {
                zone: ZoneId(1),
                epoch: 1,
            },
        };
        let range = ItemInstanceRange::new(100, 100, 1000).unwrap();
        for (template, cap) in [(57, MAX_ADENA), (3031, MAX_STACK)] {
            let held =
                BTreeMap::from([(ItemInstanceId::new(3).unwrap(), seed(3, template, cap - 1))]);
            let add = |count| {
                InventoryMutation::prepare(
                    &held,
                    range,
                    &[InventoryEdit::Add { template, count }],
                    rules,
                    ordinary(rules),
                    boundary,
                )
            };
            let plan = add(1).unwrap();
            let merged = plan
                .inventory
                .get(&ItemInstanceId::new(3).unwrap())
                .unwrap();
            assert_eq!(merged.count, cap);
            assert!(add(2).is_err());
            // A fresh stack above the cap is refused before any allocation.
            let refused = InventoryMutation::prepare(
                &BTreeMap::new(),
                range,
                &[InventoryEdit::Add {
                    template,
                    count: cap + 1,
                }],
                rules,
                ordinary(rules),
                boundary,
            );
            assert!(refused.is_err());
        }
        assert_eq!(count_cap(57, true), 99_900_000_000);
        assert_eq!(count_cap(1463, true), 2_147_483_647);
        assert_eq!(count_cap(71, false), 1);
    }

    #[test]
    fn creation_provenance_survives_later_epoch_consumption_and_merge_and_new_ids_use_current_clock(
    ) {
        use super::super::{ItemCreationOrigin, ZoneId};
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let old_origin = ItemCreationOrigin {
            zone: ZoneId(4),
            epoch: 6,
        };
        let current = ItemCreationOrigin {
            zone: ZoneId(9),
            epoch: 2,
        };
        let mut old = seed(3, 57, 20);
        old.creation_origin = Some(old_origin);
        old.created_tick = Tick(900);
        let id = old.id;
        let original = BTreeMap::from([(id, old.clone())]);
        let range = ItemInstanceRange::new(100, 100, 1000).unwrap();
        let plan = InventoryMutation::prepare(
            &original,
            range,
            &[
                InventoryEdit::TakeInstance {
                    instance: id,
                    count: 2,
                },
                InventoryEdit::Add {
                    template: 57,
                    count: 3,
                },
                InventoryEdit::Add {
                    template: 3031,
                    count: 3,
                },
            ],
            rules,
            ordinary(rules),
            InventoryBoundary {
                tick: Tick(1),
                revision: 2,
                origin: current,
            },
        )
        .unwrap();
        let retained = plan.inventory.get(&id).unwrap();
        assert_eq!(retained.count, 21);
        assert_eq!(retained.created_tick, Tick(900));
        assert_eq!(retained.creation_origin, Some(old_origin));
        let fresh = plan
            .inventory
            .get(&ItemInstanceId::new(100).unwrap())
            .unwrap();
        assert_eq!(fresh.created_tick, Tick(1));
        assert_eq!(fresh.creation_origin, Some(current));
        for change in &plan.changes {
            for item in change.before.iter().chain(change.after.iter()) {
                if item.id == id {
                    assert_eq!(
                        (item.created_tick, item.creation_origin),
                        (Tick(900), Some(old_origin))
                    );
                }
            }
        }
        let historical = BTreeMap::from([(
            id,
            ItemInstance {
                creation_origin: None,
                ..old
            },
        )]);
        let plan = InventoryMutation::prepare(
            &historical,
            range,
            &[InventoryEdit::TakeInstance {
                instance: id,
                count: 1,
            }],
            rules,
            ordinary(rules),
            InventoryBoundary {
                tick: Tick(0),
                revision: 2,
                origin: current,
            },
        )
        .unwrap();
        assert_eq!(plan.inventory.get(&id).unwrap().creation_origin, None);
        assert_eq!(plan.inventory.get(&id).unwrap().created_tick, Tick(900));
        for origin in [
            ItemCreationOrigin {
                zone: ZoneId(u32::MAX),
                ..current
            },
            ItemCreationOrigin {
                epoch: u64::MAX,
                ..current
            },
        ] {
            assert!(InventoryMutation::prepare(
                &original,
                range,
                &[InventoryEdit::Add {
                    template: 3031,
                    count: 1
                }],
                rules,
                ordinary(rules),
                InventoryBoundary {
                    tick: Tick(1),
                    revision: 2,
                    origin
                }
            )
            .is_err());
        }
        assert_eq!(original.get(&id).unwrap().count, 20);
    }

    #[test]
    fn retained_capacities_fold_bonus_from_zero_before_adding_race_base() {
        use crate::domain::zone::stat_functions::{
            FunctionConditions, FunctionKey, FunctionOp, StatCalculator, StatFunction,
        };
        use crate::domain::zone::Q;
        let bundle = embedded_phase4_bundle().unwrap();
        let conditions = FunctionConditions {
            weapon_kind: "FIST",
            can_resurrect: true,
        };
        let function = |priority, operation| StatFunction {
            key: FunctionKey {
                stat: "inventoryLimit".into(),
                priority,
                install_sequence: 1,
                handler_ordinal: 0,
                function_ordinal: 0,
            },
            operation,
            conditions: Vec::new(),
        };
        let calculator = StatCalculator::try_new(vec![
            function(1, FunctionOp::Add(3 * Q + Q / 4)),
            function(2, FunctionOp::Mul(2 * Q)),
        ])
        .unwrap();
        assert_eq!(
            computed_capacities(&calculator, &bundle.items, false, conditions).unwrap(),
            (86, 100)
        );
        assert_eq!(
            computed_capacities(&calculator, &bundle.items, true, conditions).unwrap(),
            (106, 100)
        );
        let negative =
            StatCalculator::try_new(vec![function(1, FunctionOp::Set(-81 * Q))]).unwrap();
        assert!(computed_capacities(&negative, &bundle.items, false, conditions).is_err());
        let overflow =
            StatCalculator::try_new(vec![function(1, FunctionOp::Set(i64::MAX))]).unwrap();
        assert!(computed_capacities(&overflow, &bundle.items, false, conditions).is_err());
        let mut player = crate::domain::zone::item_functions::tests::player();
        player.ordinary_capacity = 86;
        player.quest_capacity = 100;
        let limits = InventoryLimits::from_player(&player);
        assert_eq!((limits.ordinary, limits.quest), (86, 100));
    }
    #[test]
    fn merges_lowest_identity_and_pays_preferred_then_id_without_reallocating() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let inventory = [seed(3, 57, 20), seed(7, 57, 30)]
            .into_iter()
            .map(|item| (item.id, item))
            .collect();
        let range = ItemInstanceRange::new(100, 100, 110).unwrap();
        let plan = InventoryMutation::prepare(
            &inventory,
            range,
            &[
                InventoryEdit::Add {
                    template: 57,
                    count: 4,
                },
                InventoryEdit::TakeTemplate {
                    template: 57,
                    count: 35,
                    preferred: Some(ItemInstanceId::new(7).unwrap()),
                },
            ],
            rules,
            ordinary(rules),
            InventoryBoundary {
                origin: super::super::ItemCreationOrigin {
                    zone: super::super::ZoneId(1),
                    epoch: 1,
                },
                tick: Tick(10),
                revision: 2,
            },
        )
        .unwrap();
        assert_eq!(plan.allocator, range);
        assert_eq!(plan.inventory.len(), 1);
        assert_eq!(
            plan.inventory
                .get(&ItemInstanceId::new(3).unwrap())
                .unwrap()
                .count,
            19
        );
        assert_eq!(
            plan.changes
                .iter()
                .map(|c| c.before.as_ref().unwrap().id.get())
                .collect::<Vec<_>>(),
            vec![3, 7, 3]
        );
        assert_eq!(
            inventory
                .get(&ItemInstanceId::new(7).unwrap())
                .unwrap()
                .count,
            30
        );
    }
    #[test]
    fn every_refusal_preserves_input_and_cursor_including_partial_payment_and_allocation() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let inventory: BTreeMap<_, _> = [seed(3, 57, 20)]
            .into_iter()
            .map(|item| (item.id, item))
            .collect();
        let original = inventory.clone();
        let range = ItemInstanceRange::new(100, 100, 101).unwrap();
        for edits in [
            vec![InventoryEdit::TakeTemplate {
                template: 57,
                count: 21,
                preferred: None,
            }],
            vec![
                InventoryEdit::TakeInstance {
                    instance: ItemInstanceId::new(3).unwrap(),
                    count: 1,
                },
                InventoryEdit::Add {
                    template: 2369,
                    count: 2,
                },
            ],
            vec![InventoryEdit::Add {
                template: 57,
                count: i64::MAX as u64,
            }],
            vec![InventoryEdit::Add {
                template: 2369,
                count: u64::MAX,
            }],
            vec![InventoryEdit::Add {
                template: u32::MAX,
                count: 1,
            }],
        ] {
            assert!(InventoryMutation::prepare(
                &inventory,
                range,
                &edits,
                rules,
                ordinary(rules),
                InventoryBoundary {
                    origin: super::super::ItemCreationOrigin {
                        zone: super::super::ZoneId(1),
                        epoch: 1
                    },
                    tick: Tick(10),
                    revision: 2
                }
            )
            .is_err());
            assert_eq!(inventory, original);
            assert_eq!(range.bounds(), (100, 100, 101));
        }
    }
    #[test]
    fn payment_before_exchange_output_frees_capacity_and_destroy_works_while_overweight() {
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let item = seed(3, 2369, 1);
        let inventory = [(item.id, item)].into_iter().collect();
        let range = ItemInstanceRange::new(100, 100, 110).unwrap();
        let one_slot = InventoryLimits {
            ordinary: 1,
            quest: 1,
            max_weight: i64::MAX,
        };
        let edits = [
            InventoryEdit::TakeInstance {
                instance: ItemInstanceId::new(3).unwrap(),
                count: 1,
            },
            InventoryEdit::Add {
                template: 2369,
                count: 1,
            },
        ];
        let plan = InventoryMutation::prepare(
            &inventory,
            range,
            &edits,
            rules,
            one_slot,
            InventoryBoundary {
                origin: super::super::ItemCreationOrigin {
                    zone: super::super::ZoneId(1),
                    epoch: 1,
                },
                tick: Tick(10),
                revision: 2,
            },
        )
        .unwrap();
        assert!(plan
            .inventory
            .contains_key(&ItemInstanceId::new(100).unwrap()));
        assert!(!plan
            .inventory
            .contains_key(&ItemInstanceId::new(3).unwrap()));
        assert!(InventoryMutation::prepare(
            &inventory,
            range,
            &[*edits.get(1).unwrap(), *edits.first().unwrap()],
            rules,
            one_slot,
            InventoryBoundary {
                origin: super::super::ItemCreationOrigin {
                    zone: super::super::ZoneId(1),
                    epoch: 1
                },
                tick: Tick(10),
                revision: 2
            }
        )
        .is_err());
        let zero_weight = InventoryLimits::from_rules(rules, false, 0).unwrap();
        assert!(InventoryMutation::prepare(
            &inventory,
            range,
            edits.get(..1).unwrap(),
            rules,
            zero_weight,
            InventoryBoundary {
                origin: super::super::ItemCreationOrigin {
                    zone: super::super::ZoneId(1),
                    epoch: 1
                },
                tick: Tick(10),
                revision: 2
            }
        )
        .unwrap()
        .inventory
        .is_empty());
        assert!(InventoryMutation::prepare(
            &inventory,
            range,
            &edits,
            rules,
            zero_weight,
            InventoryBoundary {
                origin: super::super::ItemCreationOrigin {
                    zone: super::super::ZoneId(1),
                    epoch: 1
                },
                tick: Tick(10),
                revision: 2
            }
        )
        .is_err());
    }
    #[test]
    fn arbitrary_counts_conserve_stacks_and_never_wrap() {
        use proptest::prelude::*;
        let bundle = embedded_phase4_bundle().unwrap();
        let rules = &bundle.items;
        let mut runner = proptest::test_runner::TestRunner::default();
        runner
            // D68: both sides of the pinned adena cap, plus the old signed-bigint extremes.
            .run(&(prop_oneof![1_u64..=2 * MAX_ADENA, 1_u64..=i64::MAX as u64], prop_oneof![1_u64..=2 * MAX_ADENA, 1_u64..=i64::MAX as u64]), |(initial, addition)| {
                let inventory = [seed(3, 57, initial)]
                    .into_iter()
                    .map(|item| (item.id, item))
                    .collect();
                let range = ItemInstanceRange::new(100, 100, 101).unwrap();
                let result = InventoryMutation::prepare(
                    &inventory,
                    range,
                    &[InventoryEdit::Add {
                        template: 57,
                        count: addition,
                    }],
                    rules,
                    ordinary(rules),
                    InventoryBoundary {
                        origin: super::super::ItemCreationOrigin {
                            zone: super::super::ZoneId(1),
                            epoch: 1,
                        },
                        tick: Tick(10),
                        revision: 2,
                    },
                );
                let sum = u128::from(initial)
                    .checked_add(u128::from(addition))
                    .unwrap();
                if sum > u128::from(MAX_ADENA) {
                    prop_assert!(result.is_err());
                } else {
                    let plan = result.unwrap();
                    prop_assert_eq!(
                        plan.inventory
                            .get(&ItemInstanceId::new(3).unwrap())
                            .unwrap()
                            .count,
                        u64::try_from(sum).unwrap()
                    );
                    prop_assert_eq!(plan.allocator, range);
                    let paid = InventoryMutation::prepare(
                        &plan.inventory,
                        range,
                        &[InventoryEdit::TakeTemplate {
                            template: 57,
                            count: addition,
                            preferred: None,
                        }],
                        rules,
                        ordinary(rules),
                        InventoryBoundary {
                            origin: super::super::ItemCreationOrigin {
                                zone: super::super::ZoneId(1),
                                epoch: 1,
                            },
                            tick: Tick(10),
                            revision: 2,
                        },
                    )
                    .unwrap();
                    prop_assert_eq!(
                        paid.inventory
                            .get(&ItemInstanceId::new(3).unwrap())
                            .unwrap()
                            .count,
                        initial
                    );
                }
                Ok(())
            })
            .unwrap();
    }
}
