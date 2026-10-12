//! Resolution-time enchanting audit, independent of later changes to the target instance.
use super::phase4_projection::{
    ItemAuditEvidence, ItemEnchantOutcome, ItemOperation, ItemReceiptFact, ItemReceiptRef,
    ItemResultDetail,
};
use super::{ItemError, ItemInstance, ItemInstanceId};
use serde::{Deserialize, Serialize};

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
