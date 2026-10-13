//! D52 resolution-time SA exchange audit and source multisell planning.
use super::phase4_projection::{
    ItemDeltaPlan, ItemOperation, ItemReceiptFact, ItemReceiptRef, ItemResultDetail,
};
use super::{ItemError, ItemInstance, ItemLocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Logical rows retain their own receipt even when physical Consume rows aggregate a tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ExchangeAudit {
    pub operation_ordinal: u32,
    pub receipt: ItemReceiptRef,
    pub recipe: u64,
    pub maintain_enchantment: bool,
    pub before: ItemInstance,
    pub after: ItemInstance,
    /// Consume selected base, consume ingredients in XML order, grant fresh product.
    pub rows: Vec<ItemDeltaPlan>,
}

impl ExchangeAudit {
    /// Validate captured scalars and receipt correlation without consulting later inventory.
    pub fn validate(&self, receipt: &ItemReceiptFact) -> Result<(), ItemError> {
        self.receipt.validate()?;
        self.before.validate()?;
        self.after.validate()?;
        if self.operation_ordinal == 0
            || self.receipt.operation != ItemOperation::Exchange
            || self.receipt != receipt.reference()
            || self.recipe == 0
            || i64::try_from(self.recipe).is_err()
            || self.before.id == self.after.id
            || self.before.count != 1
            || self.after.count != 1
            || self.before.slot.is_some()
            || self.after.slot.is_some()
            || self.before.location != ItemLocation::Inventory
            || self.after.location != ItemLocation::Inventory
            || self.after.charges != 0
            || self.after.created_tick != receipt.result.tick
            || self.after.revision != receipt.result.inventory_revision
            || self.after.enchant
                != if self.maintain_enchantment {
                    self.before.enchant
                } else {
                    0
                }
            || self.after.augment_id
                != if self.maintain_enchantment {
                    self.before.augment_id
                } else {
                    None
                }
            || self.rows.len() < 2
        {
            return Err(ItemError("invalid captured SA exchange"));
        }
        let mut affected = BTreeSet::new();
        for (index, row) in self.rows.iter().enumerate() {
            row.validate(receipt.result.inventory_revision)?;
            let last = index.checked_add(1) == Some(self.rows.len());
            let before_count = row.before.as_ref().map_or(0, |item| item.count);
            let after_count = row.after.as_ref().map_or(0, |item| item.count);
            if !last {
                use super::phase4_projection::ItemEvidenceValue as V;
                let [cause] = row.causes.as_slice() else {
                    return Err(ItemError("SA payment needs one cause"));
                };
                if cause.family != "exchange_payment"
                    || cause.fields.get("receipt") != Some(&V::Text(self.receipt.key.to_string()))
                    || cause.fields.get("operation") != Some(&V::Text("exchange".into()))
                    || cause.fields.get("recipe") != Some(&V::Unsigned(self.recipe))
                    || cause.fields.get("operation_ordinal")
                        != Some(&V::Unsigned(u64::from(self.operation_ordinal)))
                    || cause.fields.get("count_delta") != Some(&V::Signed(row.count_delta))
                {
                    return Err(ItemError("SA payment cause differs from its logical row"));
                }
                if let Some(after) = &row.after {
                    let mut expected = row
                        .before
                        .clone()
                        .ok_or(ItemError("SA payment lacks before"))?;
                    expected.count = after.count;
                    expected.revision = after.revision;
                    if &expected != after {
                        return Err(ItemError("SA payment changes non-count scalar"));
                    }
                }
            }
            if row.receipt != Some(self.receipt)
                || !affected.insert(row.instance)
                || i128::from(after_count).checked_sub(i128::from(before_count))
                    != Some(i128::from(row.count_delta))
                || (last
                    && (row.operation != ItemOperation::Grant
                        || row.before.is_some()
                        || row.after.as_ref() != Some(&self.after)))
                || (!last && (row.operation != ItemOperation::Consume || row.count_delta >= 0))
                || (index == 0
                    && (row.before.as_ref() != Some(&self.before) || row.after.is_some()))
            {
                return Err(ItemError("invalid SA exchange audit row"));
            }
        }
        if !matches!(&receipt.result.detail, ItemResultDetail::Operation { operation: ItemOperation::Exchange, affected: ids, recipe: Some(recipe) }
            if *recipe == self.recipe && ids.iter().copied().collect::<BTreeSet<_>>() == affected)
        {
            return Err(ItemError("SA exchange result differs from captured audit"));
        }
        Ok(())
    }
}

pub(super) struct ExchangePlan {
    pub mutation: super::item_equipment::PaperdollMutation,
    pub allocator: super::ItemInstanceRange,
    pub audit: ExchangeAudit,
}

/// MultiSellChoose.java:247–278,342–363: inputs first, fresh unequipped product,
/// source enchant/augmentation transfer only under maintainEnchantment.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn prepare(
    player: &super::Phase4Player,
    rules: &super::Phase4Rules,
    allocator: super::ItemInstanceRange,
    target: super::ItemInstanceId,
    recipe_id: u64,
    boundary: super::item_inventory::InventoryBoundary,
    receipt: ItemReceiptRef,
    operation_ordinal: u32,
) -> Result<ExchangePlan, super::RejectReason> {
    use super::item_equipment::PaperdollMutation;
    use super::item_inventory::{InventoryEdit, InventoryLimits, InventoryMutation};
    use super::RejectReason as R;
    let recipe = rules
        .parts()
        .sa_recipes
        .get(&recipe_id)
        .ok_or(R::InvalidRecipe)?;
    if !rules
        .parts()
        .recipe_capabilities
        .get(&recipe_id)
        .is_some_and(|cap| cap.admitted)
    {
        return Err(R::ItemCapabilityDeferred);
    }
    let base = player.inventory.get(&target).ok_or(R::ItemNotOwned)?;
    let [product] = recipe.products.as_slice() else {
        return Err(R::InvalidRecipe);
    };
    if base.template_id != recipe.base_template
        || base.count != 1
        || product.count != 1
        || !rules
            .parts()
            .items
            .get(&product.template_id)
            .is_some_and(|def| def.kind == "Weapon" && !def.stackable)
        || recipe
            .ingredients
            .iter()
            .find(|row| row.template_id == recipe.base_template)
            .is_none_or(|row| row.count != 1)
    {
        return Err(R::InvalidRecipe);
    }
    let mut mutation = if let Some(slot) = base.slot {
        PaperdollMutation::unequip(
            &player.inventory,
            &player.paperdoll,
            slot,
            rules,
            boundary.revision,
        )
        .map_err(|_| R::InvalidRecipe)?
    } else {
        PaperdollMutation {
            inventory: player.inventory.clone(),
            paperdoll: player.paperdoll.clone(),
            changes: Vec::new(),
        }
    };
    let before = mutation
        .inventory
        .get(&target)
        .ok_or(R::InvalidRecipe)?
        .clone();
    let mut edits = vec![InventoryEdit::TakeInstance {
        instance: target,
        count: 1,
    }];
    for ingredient in &recipe.ingredients {
        if ingredient.template_id == recipe.base_template {
            continue;
        }
        let available = mutation
            .inventory
            .values()
            .filter(|item| item.template_id == ingredient.template_id)
            .try_fold(0_u64, |sum, item| sum.checked_add(item.count))
            .ok_or(R::InvalidRecipe)?;
        if available < ingredient.count {
            return Err(R::InsufficientItems);
        }
        edits.push(InventoryEdit::TakeTemplate {
            template: ingredient.template_id,
            count: ingredient.count,
            preferred: None,
        });
    }
    edits.push(InventoryEdit::Add {
        template: product.template_id,
        count: 1,
    });
    let mut plan = InventoryMutation::prepare(
        &mutation.inventory,
        allocator,
        &edits,
        rules,
        InventoryLimits::from_player(player),
        boundary,
    )
    .map_err(|_| R::InventoryFull)?;
    let change = plan.changes.last_mut().ok_or(R::InvalidRecipe)?;
    let after = change.after.as_mut().ok_or(R::InvalidRecipe)?;
    if change.before.is_some() {
        return Err(R::InvalidRecipe);
    }
    if recipe.maintain_enchantment {
        after.enchant = before.enchant;
        after.augment_id = before.augment_id;
    }
    let after = after.clone();
    plan.inventory.insert(after.id, after.clone());
    let rows = plan
        .changes
        .iter()
        .map(|change| {
            use super::phase4_projection::{ItemAuditEvidence, ItemEvidenceValue as V};
            let item = change
                .before
                .as_ref()
                .or(change.after.as_ref())
                .ok_or(R::InvalidRecipe)?;
            let count_delta = i64::try_from(
                i128::from(change.after.as_ref().map_or(0, |i| i.count))
                    .checked_sub(i128::from(change.before.as_ref().map_or(0, |i| i.count)))
                    .ok_or(R::InvalidRecipe)?,
            )
            .map_err(|_| R::InvalidRecipe)?;
            let consume = count_delta < 0;
            Ok(ItemDeltaPlan {
                receipt_refs: Vec::new(),
                operation: if consume {
                    ItemOperation::Consume
                } else {
                    ItemOperation::Grant
                },
                operation_ordinal: if consume { 0 } else { operation_ordinal },
                instance: item.id,
                before: change.before.clone(),
                after: change.after.clone(),
                count_delta,
                receipt: Some(receipt),
                causes: if consume {
                    vec![ItemAuditEvidence {
                        family: "exchange_payment".into(),
                        fields: std::collections::BTreeMap::from([
                            ("receipt".into(), V::Text(receipt.key.to_string())),
                            ("operation".into(), V::Text("exchange".into())),
                            ("operation_ordinal".into(), V::Unsigned(u64::from(operation_ordinal))),
                            ("recipe".into(), V::Unsigned(recipe_id)),
                            ("count_delta".into(), V::Signed(count_delta)),
                        ]),
                    }]
                } else {
                    Vec::new()
                },
            })
        })
        .collect::<Result<Vec<_>, R>>()?;
    mutation.inventory = plan.inventory;
    mutation.changes.extend(plan.changes);
    Ok(ExchangePlan {
        mutation,
        allocator: plan.allocator,
        audit: ExchangeAudit {
            operation_ordinal,
            receipt,
            recipe: recipe_id,
            maintain_enchantment: recipe.maintain_enchantment,
            before,
            after,
            rows,
        },
    })
}

/// D46 inputs come from the actual inventories around this operation, before aggregation.
pub(super) fn evidence(
    audit: &ExchangeAudit,
    before: &std::collections::BTreeMap<super::ItemInstanceId, ItemInstance>,
    after: &std::collections::BTreeMap<super::ItemInstanceId, ItemInstance>,
) -> Result<serde_json::Value, super::RejectReason> {
    use serde_json::json;
    let templates: BTreeSet<_> = audit
        .rows
        .iter()
        .filter_map(|row| row.before.as_ref())
        .filter(|item| item.id != audit.before.id)
        .map(|item| item.template_id)
        .collect();
    let counts = |inventory: &std::collections::BTreeMap<super::ItemInstanceId, ItemInstance>| {
        templates
            .iter()
            .map(|template| {
                let count = inventory
                    .values()
                    .filter(|item| item.template_id == *template)
                    .try_fold(0_u64, |sum, item| sum.checked_add(item.count))
                    .ok_or(super::RejectReason::InvalidLoad)?;
                Ok((template.to_string(), count))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>, super::RejectReason>>()
    };
    let scalar = |item: &ItemInstance| json!({"id":item.id,"template_id":item.template_id,"enchant":item.enchant,"augment_id":item.augment_id});
    Ok(json!({"formula":"sa_exchange", "inputs":{
        "recipe_id":audit.recipe,"before_instance":scalar(&audit.before),"after_instance":scalar(&audit.after),
        "ingredients_before":counts(before)?,"ingredients_after":counts(after)?,"tax_rate":"0"},
        "actual":{"product_template":audit.after.template_id,"new_instance_id":audit.after.id},
        "resolution_id":format!("exchange:{}",audit.receipt.key),"baseline":false}))
}
