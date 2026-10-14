//! Enchant preflight and both possible scalar plans precede the single accepted draw.
use crate::domain::zone::item_enchant::{crystal_count, EnchantAttempt};
use crate::domain::zone::item_equipment::PaperdollMutation;
use crate::domain::zone::item_inventory::{
    InventoryBoundary, InventoryChange, InventoryEdit, InventoryLimits, InventoryMutation,
};
use crate::domain::zone::item_rules::{EnchantScrollRule, ItemDefinition};
use crate::domain::zone::phase4_projection::{
    ItemAuditEvidence, ItemEnchantOutcome, ItemEvidenceValue, ItemReceiptRef, ItemRefund,
    ItemResultDetail,
};
use crate::domain::zone::{
    ItemCommandKind, ItemInstance, ItemInstanceId, ItemInstanceRange, Phase4Player, Phase4Rules,
    RejectReason,
};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct EnchantResolution {
    pub mutation: PaperdollMutation,
    pub allocator: ItemInstanceRange,
    pub detail: ItemResultDetail,
    pub attempt: EnchantAttempt,
    pub payments: Vec<ItemInstanceId>,
}
pub(super) struct EnchantCandidates {
    chance: u32,
    success: EnchantResolution,
    failure: EnchantResolution,
}
impl EnchantCandidates {
    pub(super) fn changes_weapon(&self) -> bool {
        self.success
            .mutation
            .changes
            .iter()
            .chain(&self.failure.mutation.changes)
            .any(|change| {
                change
                    .before
                    .iter()
                    .chain(&change.after)
                    .any(|item| item.slot == Some(crate::domain::zone::EquipSlot::Rhand))
            })
    }
    pub(super) fn resolve(self, raw: u32, word_before: u128) -> EnchantResolution {
        let mut result = if raw % 100_000 < self.chance {
            self.success
        } else {
            self.failure
        };
        result
            .attempt
            .cause
            .fields
            .insert("word_before".into(), ItemEvidenceValue::Text(word_before.to_string()));
        result
            .attempt
            .cause
            .fields
            .insert("callsite".into(), ItemEvidenceValue::Text("enchant".into()));
        result.attempt.raw_draw = raw;
        result.attempt.draw = raw % 100_000;
        result
            .attempt
            .cause
            .fields
            .insert("raw_draw".into(), ItemEvidenceValue::Unsigned(u64::from(raw)));
        result
            .attempt
            .cause
            .fields
            .insert("draw".into(), ItemEvidenceValue::Unsigned(u64::from(raw % 100_000)));
        result
    }
}
fn invalid<E>(_: E) -> RejectReason {
    RejectReason::InvalidEnchant
}
fn owned(player: &Phase4Player, id: ItemInstanceId) -> Result<&ItemInstance, RejectReason> {
    player.inventory.get(&id).ok_or(RejectReason::ItemNotOwned)
}
fn admitted(rules: &Phase4Rules, item: &ItemInstance) -> Result<(), RejectReason> {
    if rules
        .parts()
        .capabilities
        .get(&item.template_id)
        .is_some_and(|c| c.admitted)
    {
        Ok(())
    } else {
        Err(RejectReason::ItemCapabilityDeferred)
    }
}
/// Template-level scroll match (grade with S80/S84 as S, weapon/armor kind, allowed items),
/// shared by enchanting and the D113 grant ceiling.
fn scroll_matches(rule: &EnchantScrollRule, definition: &ItemDefinition) -> bool {
    let grade = match definition.grade.as_str() {
        "S80" | "S84" => "S",
        grade => grade,
    };
    rule.target_grade == grade
        && rule.weapon == (definition.kind == "Weapon")
        && (rule.allowed_items.is_empty() || rule.allowed_items.contains(&definition.template_id))
}
fn valid_scroll(
    rule: &EnchantScrollRule,
    target: &ItemInstance,
    definition: &ItemDefinition,
) -> bool {
    scroll_matches(rule, definition)
        && target.template_id == definition.template_id
        && (rule.max_enchant == 0 || target.enchant <= rule.max_enchant)
}

/// D113: the highest enchant level a template can reach in play with some matching scroll.
/// Per scroll: one past its `max_enchant` (an absent maximum is `MAX_GRANT_ENCHANT`), and no
/// further than the first level whose chance-group range has a 0% base chance (an attempt
/// from that level can never succeed). The template must be an admitted, storable,
/// non-stackable, enchantable weapon or armor outside the blacklist; otherwise zero.
pub(in crate::domain::zone) fn grant_enchant_limit(rules: &Phase4Rules, template: u32) -> u32 {
    use crate::domain::zone::item_grant::MAX_GRANT_ENCHANT;
    let parts = rules.parts();
    let Some(definition) = parts.items.get(&template) else {
        return 0;
    };
    if definition.stackable
        || !matches!(definition.kind.as_str(), "Weapon" | "Armor")
        || definition.enchant_enabled == 0
        || parts.enchant_blacklist.contains(&template)
        || !parts
            .capabilities
            .get(&template)
            .is_some_and(|c| c.admitted && c.storable)
    {
        return 0;
    }
    parts
        .enchant_scrolls
        .values()
        .filter(|rule| scroll_matches(rule, definition))
        .filter_map(|rule| {
            let scroll_cap = if rule.max_enchant == 0 {
                MAX_GRANT_ENCHANT
            } else {
                rule.max_enchant.saturating_add(1).min(MAX_GRANT_ENCHANT)
            };
            let group = chance_group(rules, definition, rule).ok()?;
            let rate_cap = group
                .ranges
                .iter()
                .filter(|range| range.chance_1e5 == 0)
                .map(|range| range.min)
                .min()
                .unwrap_or(MAX_GRANT_ENCHANT);
            Some(scroll_cap.min(rate_cap))
        })
        .max()
        .unwrap_or(0)
}
fn bonus_units(rule: &EnchantScrollRule) -> Result<u32, RejectReason> {
    let value = rule
        .bonus_rate
        .value()
        .checked_mul(crate::domain::zone::skill_math::ExactRatio::whole(1000))
        .map_err(invalid)?;
    let (n, d) = value.parts();
    if n < 0 || d <= 0 || n.checked_rem(d) != Some(0) {
        return Err(RejectReason::InvalidEnchant);
    }
    u32::try_from(n.checked_div(d).ok_or(RejectReason::InvalidEnchant)?).map_err(invalid)
}
// EnchantItemGroupsData accumulates slots and retains the last id/magic restriction in each XML group.
fn chance_group<'a>(
    rules: &'a Phase4Rules,
    definition: &ItemDefinition,
    scroll: &EnchantScrollRule,
) -> Result<&'a crate::domain::zone::item_rules::EnchantRates, RejectReason> {
    let binding = rules
        .parts()
        .enchant_bindings
        .iter()
        .find(|b| b.id == scroll.group_id)
        .ok_or(RejectReason::InvalidEnchant)?;
    for group in &binding.rules.children {
        if group.tag != "enchantRate" {
            return Err(RejectReason::InvalidEnchant);
        }
        let mut mask = 0;
        let mut item_id = 0;
        let mut magic = None;
        for item in &group.children {
            if item.tag != "item" || !item.children.is_empty() {
                return Err(RejectReason::InvalidEnchant);
            }
            for (key, value) in &item.attributes {
                match key.as_str() {
                    "slot" => {
                        let slot = match value.as_str() {
                            "fullarmor" => "onepiece",
                            "shirt" => "underwear",
                            "belt" => "waist",
                            slot => slot,
                        };
                        let source_mask = rules
                            .parts()
                            .items
                            .values()
                            .find(|item| item.bodypart == slot)
                            .ok_or(RejectReason::InvalidEnchant)?
                            .bodypart_mask;
                        mask |= source_mask;
                    },
                    "id" => item_id = value.parse::<u32>().map_err(invalid)?,
                    "magicWeapon" => magic = Some(value.parse::<bool>().map_err(invalid)?),
                    _ => return Err(RejectReason::InvalidEnchant),
                }
            }
        }
        if (item_id == 0 || item_id == definition.template_id)
            && (mask == 0 || mask & definition.bodypart_mask != 0)
            && magic.is_none_or(|magic| definition.flags.get("is_magic_weapon") == Some(&magic))
        {
            let name = group
                .attributes
                .get("group")
                .ok_or(RejectReason::InvalidEnchant)?;
            return rules
                .parts()
                .enchant_rates
                .iter()
                .find(|rates| &rates.name == name)
                .ok_or(RejectReason::InvalidEnchant);
        }
    }
    Err(RejectReason::InvalidEnchant)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Immutable operation inputs; no actor or random generator is available here.
pub(super) fn prepare(
    player: &Phase4Player,
    rules: &Phase4Rules,
    allocator: ItemInstanceRange,
    kind: &ItemCommandKind,
    boundary: InventoryBoundary,
    receipt: ItemReceiptRef,
    ordinal: u32,
) -> Result<EnchantCandidates, RejectReason> {
    let ItemCommandKind::Enchant {
        scroll,
        target,
        support,
    } = *kind
    else {
        return Err(RejectReason::InvalidEnchant);
    };
    if scroll == target || support.is_some_and(|id| id == scroll || id == target) {
        return Err(RejectReason::InvalidEnchant);
    }
    let target = owned(player, target)?;
    let scroll_item = owned(player, scroll)?;
    admitted(rules, target)?;
    admitted(rules, scroll_item)?;
    let definition = rules
        .parts()
        .items
        .get(&target.template_id)
        .ok_or(RejectReason::InvalidEnchant)?;
    let scroll = rules
        .parts()
        .enchant_scrolls
        .get(&scroll_item.template_id)
        .ok_or(RejectReason::InvalidEnchant)?;
    if target.count != 1
        || !matches!(definition.kind.as_str(), "Weapon" | "Armor")
        || definition.enchant_enabled == 0
        || rules
            .parts()
            .enchant_blacklist
            .contains(&target.template_id)
        || !valid_scroll(scroll, target, definition)
    {
        return Err(RejectReason::InvalidEnchant);
    }
    let mut payments = vec![scroll_item.id];
    let support_bonus = if let Some(id) = support {
        let item = owned(player, id)?;
        admitted(rules, item)?;
        let support = rules
            .parts()
            .enchant_supports
            .get(&item.template_id)
            .ok_or(RejectReason::InvalidEnchant)?;
        if scroll.blessed || !valid_scroll(support, target, definition) {
            return Err(RejectReason::InvalidEnchant);
        }
        payments.push(id);
        bonus_units(support)?
    } else {
        0
    };
    let group = chance_group(rules, definition, scroll)?;
    let base = group
        .ranges
        .iter()
        .find(|r| r.min <= target.enchant && target.enchant <= r.max)
        .or_else(|| group.ranges.last())
        .ok_or(RejectReason::InvalidEnchant)?
        .chance_1e5;
    let bonus = bonus_units(scroll)?;
    let chance = base
        .checked_add(bonus)
        .and_then(|c| c.checked_add(support_bonus))
        .ok_or(RejectReason::InvalidEnchant)?
        .min(100_000);
    let fields = BTreeMap::from([
        (
            "scroll_before".into(),
            ItemEvidenceValue::Text(serde_json::to_string(scroll_item).map_err(invalid)?),
        ),
        (
            "support_before".into(),
            ItemEvidenceValue::Text(
                serde_json::to_string(&support.map(|id| owned(player, id)).transpose()?)
                    .map_err(invalid)?,
            ),
        ),
        (
            "resolution_id".into(),
            ItemEvidenceValue::Text(format!("enchant:{}", receipt.key)),
        ),
        ("group".into(), ItemEvidenceValue::Text(group.name.clone())),
        ("base_chance_1e5".into(), ItemEvidenceValue::Unsigned(u64::from(base))),
        ("scroll_bonus_1e5".into(), ItemEvidenceValue::Unsigned(u64::from(bonus))),
        (
            "support_bonus_1e5".into(),
            ItemEvidenceValue::Unsigned(u64::from(support_bonus)),
        ),
        ("chance_1e5".into(), ItemEvidenceValue::Unsigned(u64::from(chance))),
        ("bound".into(), ItemEvidenceValue::Unsigned(100_000)),
    ]);
    let attempt = EnchantAttempt {
        operation_ordinal: ordinal,
        receipt,
        target: target.id,
        template: target.template_id,
        old_enchant: target.enchant,
        new_enchant: target.enchant,
        outcome: ItemEnchantOutcome::SafeFailed,
        before: target.clone(),
        after: Some(target.clone()),
        scroll: scroll_item.id,
        scroll_template: scroll_item.template_id,
        chance_1e5: chance,
        raw_draw: 0,
        draw: 0,
        cause: ItemAuditEvidence {
            family: "enchant".into(),
            fields,
        },
    };
    let success = if base == 0 {
        ItemEnchantOutcome::SuccessNoIncrement
    } else {
        ItemEnchantOutcome::Success
    };
    let failure = if scroll.safe {
        ItemEnchantOutcome::SafeFailed
    } else if scroll.blessed {
        ItemEnchantOutcome::BlessedReset
    } else {
        ItemEnchantOutcome::Destroyed
    };
    // Capacity is required only for reachable branches; the draw still occurs at 0/100%.
    let (success, failure) = if chance == 100_000 {
        let success = branch(player, rules, allocator, boundary, &payments, attempt, success)?;
        (success.clone(), success)
    } else if chance == 0 {
        let failure = branch(player, rules, allocator, boundary, &payments, attempt, failure)?;
        (failure.clone(), failure)
    } else {
        (
            branch(player, rules, allocator, boundary, &payments, attempt.clone(), success)?,
            branch(player, rules, allocator, boundary, &payments, attempt, failure)?,
        )
    };
    Ok(EnchantCandidates {
        chance,
        success,
        failure,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // Both branches preflight refund allocation/capacity before the one draw.
fn branch(
    player: &Phase4Player,
    rules: &Phase4Rules,
    allocator: ItemInstanceRange,
    boundary: InventoryBoundary,
    payments: &[ItemInstanceId],
    mut attempt: EnchantAttempt,
    outcome: ItemEnchantOutcome,
) -> Result<EnchantResolution, RejectReason> {
    let edits: Vec<_> = payments
        .iter()
        .map(|id| InventoryEdit::TakeInstance {
            instance: *id,
            count: 1,
        })
        .collect();
    let paid = InventoryMutation::prepare(
        &player.inventory,
        allocator,
        &edits,
        rules,
        InventoryLimits::from_player(player),
        boundary,
    )
    .map_err(invalid)?;
    let mut mutation = PaperdollMutation {
        inventory: paid.inventory,
        paperdoll: player.paperdoll.clone(),
        changes: paid.changes,
    };
    if matches!(outcome, ItemEnchantOutcome::BlessedReset | ItemEnchantOutcome::Destroyed) {
        if let Some(slot) = attempt.before.slot {
            let removed = PaperdollMutation::unequip(
                &mutation.inventory,
                &mutation.paperdoll,
                slot,
                rules,
                boundary.revision,
            )
            .map_err(invalid)?;
            mutation.inventory = removed.inventory;
            mutation.paperdoll = removed.paperdoll;
            mutation.changes.extend(removed.changes);
        }
    }
    let before = mutation
        .inventory
        .get(&attempt.target)
        .ok_or(RejectReason::InvalidEnchant)?
        .clone();
    let mut refunds = Vec::new();
    let mut allocator = paid.allocator;
    if outcome == ItemEnchantOutcome::Destroyed {
        let definition = rules
            .parts()
            .items
            .get(&before.template_id)
            .ok_or(RejectReason::InvalidEnchant)?;
        let grade = rules
            .parts()
            .grades
            .iter()
            .find(|g| g.grade == definition.grade)
            .ok_or(RejectReason::InvalidEnchant)?;
        let mut edits = vec![InventoryEdit::TakeInstance {
            instance: before.id,
            count: 1,
        }];
        if grade.crystal_id != 0 && definition.flags.get("is_crystallizable") == Some(&true) {
            let base_half = u64::from(definition.crystal_count)
                .checked_add(1)
                .ok_or(RejectReason::InvalidEnchant)?
                / 2;
            let count = crystal_count(definition, before.enchant, rules)
                .map_err(invalid)?
                .checked_sub(base_half)
                .ok_or(RejectReason::InvalidEnchant)?
                .max(1);
            edits.push(InventoryEdit::Add {
                template: grade.crystal_id,
                count,
            });
        }
        let destroyed = InventoryMutation::prepare(
            &mutation.inventory,
            allocator,
            &edits,
            rules,
            InventoryLimits::from_player(player),
            boundary,
        )
        .map_err(|_| RejectReason::InventoryFull)?;
        for change in &destroyed.changes {
            if let Some(after) = &change.after {
                let count = after
                    .count
                    .checked_sub(change.before.as_ref().map_or(0, |item| item.count))
                    .ok_or(RejectReason::InvalidEnchant)?;
                refunds.push(ItemRefund {
                    instance: after.id,
                    template: after.template_id,
                    count,
                });
            }
        }
        allocator = destroyed.allocator;
        mutation.inventory = destroyed.inventory;
        mutation.changes.extend(destroyed.changes);
        attempt.new_enchant = 0;
        attempt.after = None;
    } else {
        let mut after = before.clone();
        after.enchant = match outcome {
            ItemEnchantOutcome::Success => before
                .enchant
                .checked_add(1)
                .ok_or(RejectReason::InvalidEnchant)?,
            ItemEnchantOutcome::BlessedReset => 0,
            ItemEnchantOutcome::SafeFailed | ItemEnchantOutcome::SuccessNoIncrement => {
                before.enchant
            },
            ItemEnchantOutcome::Destroyed => return Err(RejectReason::InvalidEnchant),
        };
        if after != before {
            after.revision = boundary.revision;
            mutation.inventory.insert(after.id, after.clone());
            mutation.changes.push(InventoryChange {
                before: Some(before),
                after: Some(after.clone()),
            });
        }
        attempt.new_enchant = after.enchant;
        attempt.after = Some(after);
    }
    attempt.outcome = outcome;
    let definition = rules
        .parts()
        .items
        .get(&attempt.template)
        .ok_or(RejectReason::InvalidEnchant)?;
    let announcement_tier = if outcome == ItemEnchantOutcome::Success
        && ((definition.kind == "Weapon" && matches!(attempt.new_enchant, 7 | 15))
            || (definition.kind == "Armor" && attempt.new_enchant == 6))
    {
        attempt.new_enchant
    } else {
        0
    };
    let detail = ItemResultDetail::Enchant {
        target: attempt.target,
        old_enchant: attempt.old_enchant,
        new_enchant: attempt.new_enchant,
        outcome,
        refunds,
        announcement_tier,
    };
    Ok(EnchantResolution {
        mutation,
        allocator,
        detail,
        attempt,
        payments: payments.to_vec(),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;

    fn candidates(
        template: u32,
        scroll: u32,
        enchant: u32,
        support: Option<u32>,
    ) -> Result<EnchantCandidates, RejectReason> {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        let mut player = crate::domain::zone::item_functions::tests::player();
        player.max_load = i64::MAX;
        for (id, template, enchant) in [(1, template, enchant), (2, scroll, 0)]
            .into_iter()
            .chain(support.map(|id| (3, id, 0)))
        {
            let id = ItemInstanceId::new(id).unwrap();
            player.inventory.insert(
                id,
                ItemInstance {
                    creation_origin: None,
                    id,
                    template_id: template,
                    count: 1,
                    location: crate::domain::zone::ItemLocation::Inventory,
                    slot: None,
                    enchant,
                    augment_id: None,
                    charges: 0,
                    created_tick: crate::domain::zone::Tick(0),
                    revision: 1,
                },
            );
        }
        prepare(
            &player,
            &bundle.items,
            ItemInstanceRange::new(1, 1000, 10000).unwrap(),
            &ItemCommandKind::Enchant {
                target: ItemInstanceId::new(1).unwrap(),
                scroll: ItemInstanceId::new(2).unwrap(),
                support: support.map(|_| ItemInstanceId::new(3).unwrap()),
            },
            InventoryBoundary {
                origin: crate::domain::zone::ItemCreationOrigin {
                    zone: crate::domain::zone::ZoneId(1),
                    epoch: 1,
                },
                tick: crate::domain::zone::Tick(7),
                revision: 2,
            },
            ItemReceiptRef {
                operation: crate::domain::zone::phase4_projection::ItemOperation::Enchant,
                key: uuid::Uuid::from_u128(7),
            },
            1,
        )
    }

    #[test]
    fn v6_enchant_source_groups_modulo_edges_support_and_inclusive_maximum() {
        for (template, scroll, enchant, expected) in [
            (58, 956, 3, 66_670),
            (58, 956, 4, 33_340),
            (58, 956, 19, 5_560),
            (60, 952, 3, 100_000),
            (60, 952, 4, 66_670),
            (60, 952, 19, 5_890),
            (71, 951, 14, 70_000),
            (71, 951, 15, 35_000),
            (84, 951, 14, 40_000),
            (84, 951, 15, 20_000),
        ] {
            let plan = candidates(template, scroll, enchant, None).unwrap();
            assert_eq!(plan.chance, expected);
            assert_eq!(plan.resolve(expected - 1, 7).attempt.outcome, ItemEnchantOutcome::Success);
            if expected < 100_000 {
                let resolution = candidates(template, scroll, enchant, None)
                    .unwrap()
                    .resolve(expected, 7);
                assert_eq!(resolution.attempt.outcome, ItemEnchantOutcome::Destroyed);
            }
        }
        let plan = candidates(71, 951, 4, Some(12363)).unwrap();
        assert_eq!(plan.chance, 88_000);
        let resolved = plan.resolve(u32::MAX, 29);
        assert_eq!(resolved.attempt.draw, 67_295);
        assert_eq!(resolved.attempt.outcome, ItemEnchantOutcome::Success);
        assert_eq!(resolved.payments.len(), 2);
        assert!(!resolved
            .mutation
            .inventory
            .contains_key(&ItemInstanceId::new(3).unwrap()));
        assert_eq!(
            resolved.attempt.cause.fields["word_before"],
            ItemEvidenceValue::Text("29".into())
        );
        assert!(candidates(71, 6573, 4, Some(12363)).is_err());
        assert!(candidates(71, 951, 10, Some(12363)).is_err());
        assert!(candidates(71, 22227, 14, None).is_ok());
        assert!(candidates(71, 22227, 15, None).is_err());
    }
}
