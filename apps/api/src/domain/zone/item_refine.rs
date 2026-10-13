//! A13/D3/D10/D22 blacksmith augmentation.
//!
//! Source (GAME@abfde049): `AbstractRefinePacket.java:202–414` validation and gemstone
//! costs, `RequestRefine.java:50–113` (unequip, consume stone then gemstones, generate,
//! keep the same instance), `RequestRefineCancel.java:45–118` (instance crystal count
//! fee, Adena, `disarmWeapons`, no refund) and the retail-like branch of
//! `AugmentationData.java:447–559,689–737` (D3: the non-retail offset branch is inactive).
//! A8 adaptation: each source `Rnd.get(10000)` / `Rnd.get(100)` category roll is one
//! `next_u32() % 10000` draw (category weights are hashed on 1e4: 5500/3500/700/300 and
//! 5500/3500/900/100), the same modulo mapping as the enchant roll.
use super::item_equipment::PaperdollMutation;
use super::item_inventory::{
    InventoryBoundary, InventoryChange, InventoryEdit, InventoryLimits, InventoryMutation,
};
use super::item_rules::ItemCondition;
use super::phase4_projection::{
    ItemAuditEvidence, ItemEvidenceValue, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemRefineOutcome, ItemResultDetail,
};
use super::{
    EquipSlot, ItemError, ItemInstance, ItemInstanceId, ItemInstanceRange, ItemLocation,
    Phase4Player, Phase4Rules, RejectReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Residue modulus of every retail augmentation draw.
pub const RETAIL_RESIDUES: u32 = 10_000;

/// One debited stack in source order (stone, gemstones) or the cancel fee (Adena).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RefinePayment {
    pub template_id: u32,
    pub count: u64,
}

/// Captured retail draws; raw words retain the A8 modulo mapping for replay evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RefineSelection {
    pub first_raw: u32,
    pub option12: u32,
    pub category_raw: u32,
    pub category: u32,
    pub second_raw: u32,
    pub option34: u32,
}
impl RefineSelection {
    /// `(stat34 << 16) + stat12`, AugmentationData.java:558/735.
    pub fn packed(&self) -> Result<u32, ItemError> {
        pack(self.option12, self.option34)
    }
}

/// Exact option pair packing; both halves are nonzero sixteen-bit option IDs (D19).
pub fn pack(option12: u32, option34: u32) -> Result<u32, ItemError> {
    if option12 == 0 || option34 == 0 || option12 > 0xffff || option34 > 0xffff {
        return Err(ItemError("invalid augmentation option pair"));
    }
    Ok((option34 << 16) | option12)
}

/// Resolution-time audit of one accepted `Refine` or `CancelRefine`, independent of later
/// same-tick operations on the target or the paid stacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct RefineAudit {
    pub operation_ordinal: u32,
    pub receipt: ItemReceiptRef,
    pub outcome: ItemRefineOutcome,
    /// Target before any displacement by this operation.
    pub before: ItemInstance,
    /// Target after this operation.
    pub after: ItemInstance,
    /// Life stone template (Refine only).
    pub stone: Option<u32>,
    /// Debits in source order.
    pub payments: Vec<RefinePayment>,
    pub accessory: bool,
    pub mage: bool,
    /// Draws (`Refine` only); `CancelRefine` has no draw.
    pub selection: Option<RefineSelection>,
}

impl RefineAudit {
    /// Payment cause for one debited stack (D62/D63 `receipt:<operation>:<key>`).
    pub fn payment_cause(&self, count_delta: i64) -> ItemAuditEvidence {
        let family = match self.outcome {
            ItemRefineOutcome::Applied => "refine_payment",
            ItemRefineOutcome::Removed => "cancel_refine_payment",
        };
        ItemAuditEvidence {
            family: family.into(),
            fields: BTreeMap::from([
                ("receipt".into(), ItemEvidenceValue::Text(self.receipt.key.to_string())),
                (
                    "operation_ordinal".into(),
                    ItemEvidenceValue::Unsigned(u64::from(self.operation_ordinal)),
                ),
                ("count_delta".into(), ItemEvidenceValue::Signed(count_delta)),
            ]),
        }
    }

    /// Validate captured scalars and their receipt without consulting later inventory.
    pub fn validate(&self, receipt: &ItemReceiptFact) -> Result<(), ItemError> {
        self.receipt.validate()?;
        self.before.validate()?;
        self.after.validate()?;
        let applied = self.outcome == ItemRefineOutcome::Applied;
        let operation = if applied {
            ItemOperation::Refine
        } else {
            ItemOperation::CancelRefine
        };
        let mut retained = self.before.clone();
        retained.augment_id = self.after.augment_id;
        retained.revision = self.after.revision;
        retained.slot = self.after.slot;
        retained.location = self.after.location;
        let packed = self
            .selection
            .as_ref()
            .map(RefineSelection::packed)
            .transpose()?;
        if self.operation_ordinal == 0
            || self.receipt.operation != operation
            || self.receipt != receipt.reference()
            || retained != self.after
            || self.before.count != 1
            || self.after.revision != receipt.result.inventory_revision
            || self.payments.is_empty()
            || self
                .payments
                .iter()
                .any(|p| p.count == 0 || p.template_id == 0)
            || (applied
                && (self.before.augment_id.is_some()
                    || self.after.augment_id.is_none()
                    || self.after.augment_id != packed
                    || self.after.slot.is_some()
                    || self.stone.is_none()
                    || self.payments.len() != 2))
            || (!applied
                && (self.before.augment_id.is_none()
                    || self.after.augment_id.is_some()
                    || self.selection.is_some()
                    || self.stone.is_some()
                    || self.payments.len() != 1))
        {
            return Err(ItemError("invalid captured refine audit"));
        }
        if let Some(selection) = &self.selection {
            let labels = [
                55,
                35,
                if self.accessory { 9 } else { 7 },
                if self.accessory { 1 } else { 3 },
            ];
            if !labels.contains(&selection.category) {
                return Err(ItemError("invalid captured augmentation category"));
            }
        }
        if !matches!(&receipt.result.detail, ItemResultDetail::Refine { target, augment, outcome }
            if *target == self.after.id && *augment == self.after.augment_id && *outcome == self.outcome)
        {
            return Err(ItemError("refine result differs from captured audit"));
        }
        Ok(())
    }
}

/// One XML `augment` row in its retained order; `hundredths` is the decimal `chance × 100`
/// exactly (Java's float rounding is applied by [`select`], D86).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RetailRow {
    pub option: u32,
    pub hundredths: u32,
}

/// Filtered retail tables for one stone and weapon type, preserving XML order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RetailSelectors {
    pub first: Vec<RetailRow>,
    /// (category probability label, row) for every variation other than 1.
    pub second: Vec<(u32, RetailRow)>,
    pub weights: Vec<u32>,
    pub accessory: bool,
}

/// `chance × 100` as an exact integer. The loader normalizes XML decimals to reduced `n/d`
/// literals; the pinned XML has at most two fractional digits, so a value that is not a whole
/// number of hundredths fails closed instead of being rounded.
fn hundredths(text: &str) -> Result<u32, ItemError> {
    let invalid = ItemError("invalid retail augmentation chance");
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (numerator, denominator): (u64, u64) = if let Some((n, d)) = text.split_once('/') {
        if !digits(n) || !digits(d) {
            return Err(invalid);
        }
        (n.parse().map_err(|_| invalid)?, d.parse().map_err(|_| invalid)?)
    } else {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, "0"));
        if !digits(whole) || !digits(fraction) || fraction.len() > 18 {
            return Err(invalid);
        }
        let scale = 10_u64
            .checked_pow(u32::try_from(fraction.len()).map_err(|_| invalid)?)
            .ok_or(invalid)?;
        let whole: u64 = whole.parse().map_err(|_| invalid)?;
        let fraction: u64 = fraction.parse().map_err(|_| invalid)?;
        (
            whole
                .checked_mul(scale)
                .and_then(|w| w.checked_add(fraction))
                .ok_or(invalid)?,
            scale,
        )
    };
    let scaled = numerator.checked_mul(100).ok_or(invalid)?;
    if scaled.checked_rem(denominator) != Some(0) {
        return Err(invalid);
    }
    u32::try_from(scaled.checked_div(denominator).ok_or(invalid)?).map_err(|_| invalid)
}

fn attribute<'a>(node: &'a ItemCondition, key: &str) -> Result<&'a str, ItemError> {
    node.attributes
        .get(key)
        .map(String::as_str)
        .ok_or(ItemError("retail augmentation node lacks attribute"))
}

impl RetailSelectors {
    /// AugmentationData.java:451–461/505–515/693–702: filter by weapon type and exact stone,
    /// variation 1 feeds option12 and every other variation feeds option34.
    pub fn new(
        rules: &Phase4Rules,
        accessory: bool,
        mage: bool,
        stone: u32,
    ) -> Result<Self, ItemError> {
        let kind = if accessory { "accessory" } else { "weapon" };
        // The accessory routine only reads warrior rows (AugmentationData.java:696).
        let weapon_type = if mage && !accessory {
            "mage"
        } else {
            "warrior"
        };
        let mut first = Vec::new();
        let mut second = Vec::new();
        for row in rules
            .parts()
            .augment_chances
            .iter()
            .filter(|r| r.kind == kind)
        {
            if row.rules.tag != "weapon" || attribute(&row.rules, "type")? != weapon_type {
                continue;
            }
            for stone_node in &row.rules.children {
                if stone_node.tag != "stone" {
                    return Err(ItemError("unexpected retail augmentation node"));
                }
                if attribute(stone_node, "id")? != stone.to_string() {
                    continue;
                }
                for variation in &stone_node.children {
                    let variation_id: u32 = attribute(variation, "id")?
                        .parse()
                        .map_err(|_| ItemError("invalid augmentation variation"))?;
                    for category in &variation.children {
                        let label: u32 = attribute(category, "probability")?
                            .parse()
                            .map_err(|_| ItemError("invalid augmentation category"))?;
                        for augment in &category.children {
                            let entry = RetailRow {
                                option: attribute(augment, "id")?
                                    .parse()
                                    .map_err(|_| ItemError("invalid augmentation option"))?,
                                hundredths: hundredths(attribute(augment, "chance")?)?,
                            };
                            if variation_id == 1 {
                                first.push(entry);
                            } else {
                                second.push((label, entry));
                            }
                        }
                    }
                }
            }
        }
        let weights = if accessory {
            rules.parts().accessory_categories.clone()
        } else {
            rules.parts().weapon_categories.clone()
        };
        if first.is_empty()
            || second.is_empty()
            || weights.len() != 4
            || weights.iter().any(|w| *w == 0 || w % 100 != 0)
            || weights.iter().sum::<u32>() != RETAIL_RESIDUES
            || rules.parts().augment_selector != "xml_order_descending_tail"
        {
            return Err(ItemError("incomplete retail augmentation selector"));
        }
        Ok(Self {
            first,
            second,
            weights,
            accessory,
        })
    }

    /// Rows of option34 for one category label, in XML order.
    #[must_use]
    pub fn category_rows(&self, label: u32) -> Vec<RetailRow> {
        self.second
            .iter()
            .filter(|(category, _)| *category == label)
            .map(|(_, row)| *row)
            .collect()
    }

    /// Category label for one residue (`weight / 100` on the pinned 1e4 weights).
    pub fn category(&self, residue: u32) -> Result<u32, ItemError> {
        let mut end = 0_u32;
        for weight in &self.weights {
            end = end
                .checked_add(*weight)
                .ok_or(ItemError("category weight overflow"))?;
            if residue < end {
                return Ok(weight / 100);
            }
        }
        Err(ItemError("augmentation category residue outside weights"))
    }

    /// Three draws in source order; `draw` returns one raw u32 per call.
    pub fn select(&self, mut draw: impl FnMut() -> u32) -> Result<RefineSelection, ItemError> {
        let first_raw = draw();
        let option12 = select(&self.first, first_raw % RETAIL_RESIDUES)?;
        let category_raw = draw();
        let category = self.category(category_raw % RETAIL_RESIDUES)?;
        let second_raw = draw();
        let option34 = select(&self.category_rows(category), second_raw % RETAIL_RESIDUES)?;
        Ok(RefineSelection {
            first_raw,
            option12,
            category_raw,
            category,
            second_raw,
            option34,
        })
    }
}

/// Fixed-point scale of the binary32 emulation: values are exact integers times 2^-40, so every
/// float the selector reaches (chances ≥ 0.01, accumulators ≤ 10000) is represented exactly.
const BINARY32_SCALE: u32 = 40;

/// Round the positive rational `numerator / denominator` (in 2^-40 units) to the nearest
/// binary32, ties to even, IEEE 754 roundTiesToEven as Java float arithmetic does. Values
/// below the scale's 24-bit window must already be exact; anything else fails closed.
fn round_binary32(numerator: u128, denominator: u128) -> Result<u128, ItemError> {
    let invalid = ItemError("retail binary32 emulation out of range");
    let whole = numerator.checked_div(denominator).ok_or(invalid)?;
    let bits = 128_u32.checked_sub(whole.leading_zeros()).ok_or(invalid)?;
    let Some(shift) = bits.checked_sub(24) else {
        // Below 2^24 units the float grid is no coarser than one unit: exact or reject.
        return (numerator.checked_rem(denominator) == Some(0))
            .then_some(whole)
            .ok_or(invalid);
    };
    let unit = denominator.checked_shl(shift).ok_or(invalid)?;
    let mut mantissa = numerator.checked_div(unit).ok_or(invalid)?;
    let twice = numerator
        .checked_rem(unit)
        .and_then(|r| r.checked_mul(2))
        .ok_or(invalid)?;
    if twice > unit || (twice == unit && mantissa % 2 == 1) {
        mantissa = mantissa.checked_add(1).ok_or(invalid)?;
    }
    mantissa.checked_shl(shift).ok_or(invalid)
}

/// `(float) value` for a signed exact value in 2^-40 units.
fn round_binary32_signed(value: i128) -> Result<i128, ItemError> {
    let invalid = ItemError("retail binary32 emulation out of range");
    let magnitude =
        i128::try_from(round_binary32(value.unsigned_abs(), 1)?).map_err(|_| invalid)?;
    Ok(if value < 0 {
        magnitude.checked_neg().ok_or(invalid)?
    } else {
        magnitude
    })
}

/// `getAugmentChance() * 100` in float: Float.parseFloat of the exact decimal (`hundredths /
/// 100`), then the float product with the int literal 100, each rounded to binary32.
fn float_decrement(hundredths: u32) -> Result<i128, ItemError> {
    let invalid = ItemError("retail binary32 emulation out of range");
    let chance = round_binary32(
        u128::from(hundredths)
            .checked_shl(BINARY32_SCALE)
            .ok_or(invalid)?,
        100,
    )?;
    let product = round_binary32(chance.checked_mul(100).ok_or(invalid)?, 1)?;
    i128::try_from(product).map_err(|_| invalid)
}

/// Descending-tail selection, AugmentationData.java:464–468 (and 497–503, 518–522, 550–555,
/// 705–709, 729–734): while `float s` exceeds the residue, `s -= chance * 100` and select the
/// visited row. D86: the accumulator emulates Java binary32 exactly in integers; the residue
/// widens to float exactly (< 2^24). No reachable residue may leave option 0.
pub fn select(rows: &[RetailRow], residue: u32) -> Result<u32, ItemError> {
    let invalid = ItemError("retail threshold overflow");
    let mut threshold = i128::from(RETAIL_RESIDUES)
        .checked_shl(BINARY32_SCALE)
        .ok_or(invalid)?;
    let residue = i128::from(residue)
        .checked_shl(BINARY32_SCALE)
        .ok_or(invalid)?;
    let mut selected = 0;
    for row in rows {
        if threshold > residue {
            let difference = threshold
                .checked_sub(float_decrement(row.hundredths)?)
                .ok_or(invalid)?;
            threshold = round_binary32_signed(difference)?;
            selected = row.option;
        }
    }
    if selected == 0 {
        return Err(ItemError("retail augmentation table leaves option zero"));
    }
    Ok(selected)
}

/// Validated pre-draw plan. Refine payments and displacement are already applied to the
/// staged inventory; the target is not yet augmented.
pub(super) struct RefinePrepared {
    pub mutation: PaperdollMutation,
    pub allocator: ItemInstanceRange,
    pub audit: RefineAudit,
    pub selectors: Option<RetailSelectors>,
    /// Debited instances, routed into the tick's Consume aggregates with `cause`.
    pub payments: Vec<ItemInstanceId>,
}

impl RefinePrepared {
    /// Weapon displacement must pass the cast guard before any draw.
    pub(super) fn changes_weapon(&self) -> bool {
        self.mutation.changes.iter().any(|change| {
            change
                .before
                .iter()
                .chain(&change.after)
                .any(|item| item.slot == Some(EquipSlot::Rhand))
        })
    }

    /// Apply the (possibly absent) selection to the same target identity.
    pub(super) fn finish(
        mut self,
        selection: Option<RefineSelection>,
        revision: u64,
    ) -> Result<
        (PaperdollMutation, ItemInstanceRange, RefineAudit, Vec<ItemInstanceId>),
        RejectReason,
    > {
        let invalid = |_| RejectReason::InvalidRefinement;
        let target = self.audit.before.id;
        let before = self
            .mutation
            .inventory
            .get(&target)
            .ok_or(RejectReason::InvalidRefinement)?
            .clone();
        let mut after = before.clone();
        after.augment_id = match (self.audit.outcome, &selection) {
            (ItemRefineOutcome::Applied, Some(selection)) => {
                Some(selection.packed().map_err(invalid)?)
            },
            (ItemRefineOutcome::Removed, None) => None,
            _ => return Err(RejectReason::InvalidRefinement),
        };
        after.revision = revision;
        self.mutation.inventory.insert(target, after.clone());
        self.mutation.changes.push(InventoryChange {
            before: Some(before),
            after: Some(after.clone()),
        });
        self.audit.after = after;
        self.audit.selection = selection;
        Ok((self.mutation, self.allocator, self.audit, self.payments))
    }
}

fn definition<'a>(
    rules: &'a Phase4Rules,
    item: &ItemInstance,
) -> Result<&'a super::item_rules::ItemDefinition, RejectReason> {
    rules
        .parts()
        .items
        .get(&item.template_id)
        .ok_or(RejectReason::InvalidLoad)
}

fn grade_id(rules: &Phase4Rules, grade: &str) -> Result<u8, RejectReason> {
    rules
        .parts()
        .grades
        .iter()
        .find(|g| g.grade == grade)
        .map(|g| g.id)
        .ok_or(RejectReason::InvalidLoad)
}

/// AbstractRefinePacket.java:269–337 target checks, shared by the refine path.
fn valid_target(rules: &Phase4Rules, item: &ItemInstance) -> Result<(bool, bool), RejectReason> {
    let definition = definition(rules, item)?;
    if !rules
        .parts()
        .capabilities
        .get(&item.template_id)
        .is_some_and(|cap| cap.admitted)
    {
        return Err(RejectReason::ItemCapabilityDeferred);
    }
    let flag = |name: &str| definition.flags.get(name) == Some(&true);
    let pvp_allowed = rules
        .parts()
        .capacities
        .policy
        .get("allow_augment_pvp_items")
        .is_some_and(|value| value == "true");
    let accessory = match definition.kind.as_str() {
        "Weapon" => {
            if matches!(definition.weapon_type.as_str(), "NONE" | "FISHINGROD") {
                return Err(RejectReason::InvalidRefinement);
            }
            false
        },
        "Armor" => {
            if !matches!(definition.bodypart.as_str(), "rfinger;lfinger" | "rear;lear" | "neck") {
                return Err(RejectReason::InvalidRefinement);
            }
            true
        },
        _ => return Err(RejectReason::InvalidRefinement),
    };
    if item.augment_id.is_some()
        || flag("is_hero")
        || flag("is_common")
        || (flag("is_pvp") && !pvp_allowed)
        || grade_id(rules, &definition.grade)? < grade_id(rules, "C")?
        || !matches!(item.location, ItemLocation::Inventory | ItemLocation::Paperdoll)
        || rules.parts().augment_blacklist.contains(&item.template_id)
    {
        return Err(RejectReason::InvalidRefinement);
    }
    Ok((accessory, flag("is_magic_weapon")))
}

fn total(player: &Phase4Player, template: u32) -> Result<u64, RejectReason> {
    player
        .inventory
        .values()
        .filter(|item| item.template_id == template && item.location == ItemLocation::Inventory)
        .try_fold(0_u64, |sum, item| sum.checked_add(item.count))
        .ok_or(RejectReason::InvalidLoad)
}

/// RequestRefine.java:50–106 and AbstractRefinePacket.java:202–261: validate the target,
/// stone and server-selected gemstones together, then unequip and pay before generation.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_refine(
    player: &Phase4Player,
    rules: &Phase4Rules,
    allocator: ItemInstanceRange,
    level: u32,
    target: ItemInstanceId,
    stone: ItemInstanceId,
    boundary: InventoryBoundary,
    receipt: ItemReceiptRef,
    operation_ordinal: u32,
) -> Result<RefinePrepared, RejectReason> {
    let item = player
        .inventory
        .get(&target)
        .ok_or(RejectReason::ItemNotOwned)?;
    let stone_item = player
        .inventory
        .get(&stone)
        .ok_or(RejectReason::ItemNotOwned)?;
    let (accessory, mage) = valid_target(rules, item)?;
    if stone == target || stone_item.location != ItemLocation::Inventory {
        return Err(RejectReason::InvalidRefinement);
    }
    let life = rules
        .parts()
        .life_stones
        .get(&stone_item.template_id)
        .ok_or(RejectReason::InvalidRefinement)?;
    if (life.kind == "accessory") != accessory || level < life.level {
        return Err(RejectReason::InvalidRefinement);
    }
    let grade = &definition(rules, item)?.grade;
    let cost = rules
        .parts()
        .augment_costs
        .iter()
        .find(|cost| &cost.grade == grade)
        .ok_or(RejectReason::InvalidRefinement)?;
    let gem_count = u64::from(if accessory {
        cost.accessory_count
    } else {
        cost.weapon_count
    });
    let kind = if accessory {
        "accessory:warrior"
    } else if mage {
        "weapon:mage"
    } else {
        "weapon:warrior"
    };
    // D5/D32: a stone whose reachable options are not all executable is a named blocker.
    if !rules
        .parts()
        .augment_capabilities
        .iter()
        .any(|cap| cap.kind == kind && cap.stone_id == stone_item.template_id && cap.admitted)
    {
        return Err(RejectReason::ItemCapabilityDeferred);
    }
    if gem_count == 0 || total(player, cost.gemstone_id)? < gem_count {
        return Err(RejectReason::InsufficientItems);
    }
    let selectors = RetailSelectors::new(rules, accessory, mage, stone_item.template_id)
        .map_err(|_| RejectReason::ItemCapabilityDeferred)?;
    let before = item.clone();
    let mut mutation = displaced(player, rules, item.slot, boundary.revision)?;
    let edits = [
        InventoryEdit::TakeInstance {
            instance: stone,
            count: 1,
        },
        InventoryEdit::TakeTemplate {
            template: cost.gemstone_id,
            count: gem_count,
            preferred: None,
        },
    ];
    let paid = InventoryMutation::prepare(
        &mutation.inventory,
        allocator,
        &edits,
        rules,
        InventoryLimits::from_player(player),
        boundary,
    )
    .map_err(|_| RejectReason::InsufficientItems)?;
    let payments = paid
        .changes
        .iter()
        .filter_map(|change| change.before.as_ref().map(|item| item.id))
        .collect();
    mutation.inventory = paid.inventory;
    mutation.changes.extend(paid.changes);
    Ok(RefinePrepared {
        mutation,
        allocator: paid.allocator,
        audit: RefineAudit {
            operation_ordinal,
            receipt,
            outcome: ItemRefineOutcome::Applied,
            after: before.clone(),
            before,
            stone: Some(stone_item.template_id),
            payments: vec![
                RefinePayment {
                    template_id: stone_item.template_id,
                    count: 1,
                },
                RefinePayment {
                    template_id: cost.gemstone_id,
                    count: gem_count,
                },
            ],
            accessory,
            mage,
            selection: None,
        },
        selectors: Some(selectors),
        payments,
    })
}

/// RequestRefineCancel.java:66–110: fee from the instance crystal count including enchant,
/// Adena first, then `disarmWeapons` (the RHAND weapon, whichever item the target is) when
/// the target is equipped, then removal without refund.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_cancel(
    player: &Phase4Player,
    rules: &Phase4Rules,
    allocator: ItemInstanceRange,
    target: ItemInstanceId,
    boundary: InventoryBoundary,
    receipt: ItemReceiptRef,
    operation_ordinal: u32,
) -> Result<RefinePrepared, RejectReason> {
    let item = player
        .inventory
        .get(&target)
        .ok_or(RejectReason::ItemNotOwned)?;
    if item.augment_id.is_none() {
        return Err(RejectReason::InvalidRefinement);
    }
    let definition = definition(rules, item)?;
    let fee = cancel_fee(rules, definition, item.enchant)?;
    if total(player, ADENA)? < fee {
        return Err(RejectReason::InsufficientItems);
    }
    let before = item.clone();
    let paid = InventoryMutation::prepare(
        &player.inventory,
        allocator,
        &[InventoryEdit::TakeTemplate {
            template: ADENA,
            count: fee,
            preferred: None,
        }],
        rules,
        InventoryLimits::from_player(player),
        boundary,
    )
    .map_err(|_| RejectReason::InsufficientItems)?;
    let payments = paid
        .changes
        .iter()
        .filter_map(|change| change.before.as_ref().map(|item| item.id))
        .collect();
    let mut mutation = PaperdollMutation {
        inventory: paid.inventory,
        paperdoll: player.paperdoll.clone(),
        changes: paid.changes,
    };
    if item.slot.is_some() && player.paperdoll.contains_key(&EquipSlot::Rhand) {
        let removed = PaperdollMutation::unequip(
            &mutation.inventory,
            &mutation.paperdoll,
            EquipSlot::Rhand,
            rules,
            boundary.revision,
        )
        .map_err(|_| RejectReason::InvalidRefinement)?;
        mutation.inventory = removed.inventory;
        mutation.paperdoll = removed.paperdoll;
        mutation.changes.extend(removed.changes);
    }
    let (accessory, mage) = (
        definition.kind == "Armor",
        definition.flags.get("is_magic_weapon") == Some(&true),
    );
    Ok(RefinePrepared {
        mutation,
        allocator: paid.allocator,
        audit: RefineAudit {
            operation_ordinal,
            receipt,
            outcome: ItemRefineOutcome::Removed,
            after: before.clone(),
            before,
            stone: None,
            payments: vec![RefinePayment {
                template_id: ADENA,
                count: fee,
            }],
            accessory,
            mage,
            selection: None,
        },
        selectors: None,
        payments,
    })
}

/// Adena, Inventory.java:51.
pub const ADENA: u32 = 57;

/// RequestRefineCancel.java:73–106 via the hashed `cancel_fees` rows (0 = otherwise).
pub fn cancel_fee(
    rules: &Phase4Rules,
    definition: &super::item_rules::ItemDefinition,
    enchant: u32,
) -> Result<u64, RejectReason> {
    let crystals = super::item_enchant::crystal_count(definition, enchant, rules)
        .map_err(|_| RejectReason::InvalidLoad)?;
    rules
        .parts()
        .cancel_fees
        .iter()
        .filter(|fee| fee.grade == definition.grade)
        .find(|fee| fee.crystal_count_below == 0 || crystals < fee.crystal_count_below)
        .map(|fee| fee.adena)
        .ok_or(RejectReason::InvalidRefinement)
}

fn displaced(
    player: &Phase4Player,
    rules: &Phase4Rules,
    slot: Option<EquipSlot>,
    revision: u64,
) -> Result<PaperdollMutation, RejectReason> {
    if let Some(slot) = slot {
        PaperdollMutation::unequip(&player.inventory, &player.paperdoll, slot, rules, revision)
            .map_err(|_| RejectReason::InvalidRefinement)
    } else {
        Ok(PaperdollMutation {
            inventory: player.inventory.clone(),
            paperdoll: player.paperdoll.clone(),
            changes: Vec::new(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::infrastructure::item_data::embedded_phase4_bundle;

    fn row(option: u32, hundredths: u32) -> RetailRow {
        RetailRow { option, hundredths }
    }

    #[test]
    fn descending_tail_selection_matches_source_boundaries() {
        // 10000 → 9890 → 9780: residue ≥ 9890 keeps the first row only.
        let rows = [row(1, 110), row(2, 110), row(3, 9780)];
        assert_eq!(select(&rows, 9999).unwrap(), 1);
        assert_eq!(select(&rows, 9890).unwrap(), 1);
        assert_eq!(select(&rows, 9889).unwrap(), 2);
        assert_eq!(select(&rows, 9780).unwrap(), 2);
        assert_eq!(select(&rows, 9779).unwrap(), 3);
        assert_eq!(select(&rows, 0).unwrap(), 3);
        assert!(select(&[], 0).is_err());
        assert_eq!(hundredths("1.1").unwrap(), 110);
        assert_eq!(hundredths("0.25").unwrap(), 25);
        assert_eq!(hundredths("12").unwrap(), 1200);
        assert_eq!(hundredths("11/10").unwrap(), 110);
        assert_eq!(hundredths("1/4").unwrap(), 25);
        for bad in ["1.125", "", ".5", "1,1", "-1", "1/3", "1/0", "/2"] {
            assert!(hundredths(bad).is_err(), "{bad}");
        }
        assert_eq!(pack(0xffff, 0xffff).unwrap(), u32::MAX);
        assert!(pack(0, 1).is_err() && pack(1, 0x1_0000).is_err());
    }

    #[test]
    fn retail_selection_matches_literal_rows_read_from_the_pinned_xml() {
        // Expected ids read by hand from retailchances*.xml (DP 3ca488dd), not the selector:
        // 8723 warrior variation 1 = ids 1..91 (1.1 each, last 1.0); mage = 3641..3731;
        // 8723 warrior variation 2 category 55 = 7281..7293 (7.69 each, last 7.72);
        // 8753 variation 1 = 95 rows ending 16343, 16344 at 1.05 (D86 float tail at 105);
        // accessory 12754 variation 1 = 16687..16704.
        let rules = embedded_phase4_bundle().unwrap().items;
        let warrior = RetailSelectors::new(&rules, false, false, 8723).unwrap();
        let mage = RetailSelectors::new(&rules, false, true, 8723).unwrap();
        let tail = RetailSelectors::new(&rules, false, false, 8753).unwrap();
        let accessory = RetailSelectors::new(&rules, true, false, 12754).unwrap();
        let cat55 = warrior.category_rows(55);
        for (rows, residue, option) in [
            (&warrior.first, 9999, 1),
            (&warrior.first, 9890, 1),
            (&warrior.first, 9889, 2),
            (&warrior.first, 0, 91),
            (&mage.first, 9999, 3641),
            (&mage.first, 0, 3731),
            (&cat55, 9999, 7281),
            (&cat55, 9231, 7281),
            (&cat55, 9230, 7282),
            (&cat55, 0, 7293),
            (&tail.first, 9999, 2731),
            (&tail.first, 105, 16344),
            (&tail.first, 0, 16344),
            (&accessory.first, 9999, 16687),
            (&accessory.first, 0, 16704),
        ] {
            assert_eq!(select(rows, residue).unwrap(), option, "{residue}");
        }
    }

    #[test]
    fn binary32_selection_matches_independent_oracle_on_every_divergent_table() {
        // D86: packages/data/scripts/refine_binary32_oracle.py parses the pinned XML and emulates
        // Java float with struct round trips; each entry is [residue, java, exact decimal].
        let rules = embedded_phase4_bundle().unwrap().items;
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../packages/data/fixtures/refine-binary32-oracle.json"
        ))
        .unwrap();
        let tables = oracle.get("tables").and_then(|t| t.as_array()).unwrap();
        let mut checked = 0;
        for table in tables {
            let text = |key: &str| table.get(key).and_then(|v| v.as_str()).unwrap();
            let accessory = text("kind") == "accessory";
            let mage = text("weapon_type") == "mage";
            if accessory && mage {
                continue; // generateRandomAccessoryAugmentation reads warrior rows only.
            }
            let stone = u32::try_from(
                table
                    .get("stone")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap(),
            )
            .unwrap();
            let selectors = RetailSelectors::new(&rules, accessory, mage, stone).unwrap();
            let rows = match text("selector") {
                "option12" => selectors.first.clone(),
                label => selectors.category_rows(label.parse().unwrap()),
            };
            for entry in table.get("residues").and_then(|v| v.as_array()).unwrap() {
                let value = |index: usize| {
                    u32::try_from(
                        entry
                            .get(index)
                            .and_then(serde_json::Value::as_u64)
                            .unwrap(),
                    )
                    .unwrap()
                };
                assert_ne!(value(1), value(2), "{table}");
                assert_eq!(select(&rows, value(0)).unwrap(), value(1), "{stone} {entry}");
            }
            checked += 1;
        }
        assert_eq!(checked, 30);
        // The 30 are the 95-row option12 tables (chance 1.05) of 15 stones × warrior/mage: the
        // float accumulator still exceeds residue 105 before the last row. Rounding helpers:
        assert!(round_binary32(3, 2).is_err());
        assert_eq!(round_binary32(3 << BINARY32_SCALE, 2).unwrap(), 3 << (BINARY32_SCALE - 1));
        assert_eq!(float_decrement(110).unwrap(), 110 << BINARY32_SCALE);
    }

    #[test]
    fn every_admitted_stone_table_selects_a_nonzero_option_for_every_residue() {
        let rules = embedded_phase4_bundle().unwrap().items;
        let mut checked = 0;
        for cap in &rules.parts().augment_capabilities {
            let (accessory, mage) = match cap.kind.as_str() {
                "accessory:warrior" => (true, false),
                "weapon:mage" => (false, true),
                _ => (false, false),
            };
            let selectors = RetailSelectors::new(&rules, accessory, mage, cap.stone_id).unwrap();
            for residue in 0..RETAIL_RESIDUES {
                select(&selectors.first, residue).unwrap();
                let label = selectors.category(residue).unwrap();
                select(&selectors.category_rows(label), residue).unwrap();
            }
            checked += 1;
        }
        assert_eq!(checked, rules.parts().augment_capabilities.len());
        let accessory = RetailSelectors::new(&rules, true, false, 12754).unwrap();
        let weapon = RetailSelectors::new(&rules, false, false, 8723).unwrap();
        for (selectors, labels) in [(&accessory, [55, 35, 9, 1]), (&weapon, [55, 35, 7, 3])] {
            let mut seen = Vec::new();
            for residue in [0, 5499, 5500, 8999, 9000, 9699, 9700, 9899, 9900, 9999] {
                seen.push(selectors.category(residue).unwrap());
            }
            assert!(labels.iter().all(|l| seen.contains(l)));
        }
        assert_eq!(accessory.category(9899).unwrap(), 9);
        assert_eq!(accessory.category(9900).unwrap(), 1);
        assert_eq!(weapon.category(9699).unwrap(), 7);
        assert_eq!(weapon.category(9700).unwrap(), 3);
        assert!(weapon.category(RETAIL_RESIDUES).is_err());
    }

    #[test]
    fn cancel_fee_uses_instance_crystal_count_boundaries() {
        let rules = embedded_phase4_bundle().unwrap().items;
        let mut definition = rules
            .parts()
            .items
            .values()
            .find(|d| d.kind == "Weapon" && d.grade == "C")
            .unwrap()
            .clone();
        for (crystals, fee) in [
            (1719, 95_000),
            (1720, 150_000),
            (2451, 150_000),
            (2452, 210_000),
        ] {
            definition.crystal_count = crystals;
            assert_eq!(cancel_fee(&rules, &definition, 0).unwrap(), fee);
        }
        // Enchant raises the crystal count (L2Item.java:405–420) across a fee boundary.
        definition.crystal_count = 1719;
        assert_eq!(cancel_fee(&rules, &definition, 1).unwrap(), 150_000);
        for (grade, crystals, fee) in [
            ("B", 1745, 240_000),
            ("B", 1746, 270_000),
            ("A", 2159, 330_000),
            ("A", 2160, 390_000),
            ("A", 2824, 420_000),
            ("S", 0, 480_000),
            ("S80", 0, 920_000),
            ("S84", 0, 920_000),
        ] {
            definition.grade = grade.into();
            definition.crystal_count = crystals;
            assert_eq!(cancel_fee(&rules, &definition, 0).unwrap(), fee, "{grade}");
        }
        definition.grade = "D".into();
        assert!(cancel_fee(&rules, &definition, 0).is_err());
    }
}
