//! Resolution-time enchanting audit, independent of later changes to the target instance.
use super::phase4_projection::{
    ItemAuditEvidence, ItemEnchantOutcome, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemResultDetail,
};
use super::{ItemError, ItemInstance, ItemInstanceId};
use serde::{Deserialize, Serialize};

/// Full instance crystal yield, before enchanting's distinct failure refund reduction.
/// L2Item.java:405–420; grade bonuses are imported from CrystalType.java.
pub fn crystal_count(
    definition: &super::item_rules::ItemDefinition,
    enchant: u32,
    rules: &super::Phase4Rules,
) -> Result<u64, ItemError> {
    let grade = rules
        .parts()
        .grades
        .iter()
        .find(|grade| grade.grade == definition.grade)
        .ok_or(ItemError("missing crystal grade"))?;
    let (bonus, multiplier, offset) = match definition.kind.as_str() {
        "Weapon" => (grade.weapon_bonus, 2_u64, 3_u64),
        "Armor" => (grade.armor_bonus, 3, 6),
        _ => (0, 1, 0),
    };
    let levels = if enchant > 3 {
        u64::from(enchant)
            .checked_mul(multiplier)
            .and_then(|n| n.checked_sub(offset))
    } else {
        Some(u64::from(enchant))
    }
    .ok_or(ItemError("crystal yield overflow"))?;
    u64::from(bonus)
        .checked_mul(levels)
        .and_then(|n| n.checked_add(u64::from(definition.crystal_count)))
        .filter(|n| i64::try_from(*n).is_ok())
        .ok_or(ItemError("crystal yield overflow"))
}

/// D49 captures the attempted scalar before another operation can destroy or exchange it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct EnchantAttempt {
    pub operation_ordinal: u32,
    pub receipt: ItemReceiptRef,
    pub target: ItemInstanceId,
    pub template: u32,
    pub old_enchant: u32,
    pub new_enchant: u32,
    pub outcome: ItemEnchantOutcome,
    pub before: ItemInstance,
    pub after: Option<ItemInstance>,
    pub scroll: ItemInstanceId,
    pub scroll_template: u32,
    pub chance_1e5: u32,
    pub raw_draw: u32,
    pub draw: u32,
    pub cause: ItemAuditEvidence,
}
impl EnchantAttempt {
    /// Check captured identity and the integer adaptation, never reconstruct from final items.
    pub fn validate(&self, receipt: &ItemReceiptFact) -> Result<(), ItemError> {
        self.receipt.validate()?;
        self.before.validate()?;
        let success = matches!(
            self.outcome,
            ItemEnchantOutcome::Success | ItemEnchantOutcome::SuccessNoIncrement
        );
        if self.operation_ordinal == 0
            || self.receipt.operation != ItemOperation::Enchant
            || self.receipt != receipt.reference()
            || self.target != self.before.id
            || self.template != self.before.template_id
            || self.before.count != 1
            || self.old_enchant != self.before.enchant
            || self.scroll == self.target
            || self.scroll_template == 0
            || i32::try_from(self.scroll_template).is_err()
            || self.before.revision > receipt.result.inventory_revision
            || self.chance_1e5 > 100_000
            || self.draw != self.raw_draw % 100_000
            || success != (self.draw < self.chance_1e5)
            || self.cause.family.is_empty()
            || (self.outcome == ItemEnchantOutcome::Destroyed) != self.after.is_none()
        {
            return Err(ItemError("invalid captured enchant attempt"));
        }
        if let Some(after) = &self.after {
            after.validate()?;
            if after.id != self.target
                || after.template_id != self.template
                || after.count != 1
                || after.enchant != self.new_enchant
                || after.revision > receipt.result.inventory_revision
            {
                return Err(ItemError("enchant attempt scalar mismatch"));
            }
        }
        if !matches!(&receipt.result.detail, ItemResultDetail::Enchant {
            target, old_enchant, new_enchant, outcome, ..
        } if *target == self.target && *old_enchant == self.old_enchant && *new_enchant == self.new_enchant && *outcome == self.outcome)
        {
            return Err(ItemError("enchant attempt result mismatch"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    #[test]
    fn source_crystal_yield_preserves_weapon_and_armour_breakpoints() {
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        for (template, expected) in [
            (71, [705, 750, 840, 930, 1110]),
            (58, [260, 271, 293, 326, 392]),
            (59, [180, 191, 213, 246, 312]),
        ] {
            for (enchant, count) in [0, 1, 3, 4, 6].into_iter().zip(expected) {
                assert_eq!(
                    super::crystal_count(
                        &bundle.items.parts().items[&template],
                        enchant,
                        &bundle.items
                    )
                    .unwrap(),
                    count
                );
            }
        }
        // Every imported grade's exact source bonuses; no S80/S84 rank-index shortcut.
        for (grade, expected) in [
            ("D", 451),
            ("C", 226),
            ("B", 336),
            ("A", 726),
            ("S", 1251),
            ("S80", 1251),
            ("S84", 1251),
        ] {
            let mut definition = bundle.items.parts().items[&71].clone();
            definition.grade = grade.into();
            definition.crystal_count = 1;
            assert_eq!(super::crystal_count(&definition, 4, &bundle.items).unwrap(), expected);
        }
    }
}
