//! Private durable item facts. These are settlement/audit inputs, never client authority.
use super::{EntityId, ItemError, ItemInstance, ItemInstanceId, Phase4Checkpoint, Tick};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Shared logical operations. Consume is automatic; toggle has no per-instance ledger row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemOperation {
    Grant,
    Starter,
    Backfill,
    Equip,
    Unequip,
    Consume,
    Enchant,
    Destroy,
    Crystallize,
    Refine,
    CancelRefine,
    Exchange,
    Use,
    ToggleAutoShot,
}

/// Exact audit values cannot deserialize a fractional numeric literal into domain state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[allow(missing_docs)]
pub enum ItemEvidenceValue {
    Signed(i64),
    Unsigned(u64),
    Ratio(super::effects::FrozenRatio),
    Boolean(bool),
    Text(String),
    Values(Vec<ItemEvidenceValue>),
}
/// Named formula/draw/cause facts preserve exact inputs and evaluation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemAuditEvidence {
    pub family: String,
    pub fields: BTreeMap<String, ItemEvidenceValue>,
}

/// One real instance change; aggregate consumption carries explicit net count and ordered
/// causes, even when another logical mutation intervened between two consumptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemDeltaPlan {
    pub operation: ItemOperation,
    pub operation_ordinal: u32,
    pub instance: ItemInstanceId,
    pub before: Option<ItemInstance>,
    pub after: Option<ItemInstance>,
    pub count_delta: i64,
    pub receipt: Option<uuid::Uuid>,
    pub causes: Vec<ItemAuditEvidence>,
}
impl ItemDeltaPlan {
    /// The top-level event template follows the resulting scalar, or the consumed scalar.
    pub fn template(&self) -> Result<u32, ItemError> {
        self.after
            .as_ref()
            .or(self.before.as_ref())
            .map(|i| i.template_id)
            .ok_or(ItemError("item delta has no state"))
    }
    /// Projection validation does not rerun gameplay; it rejects impossible audit identity.
    pub fn validate(&self, revision: u64) -> Result<(), ItemError> {
        if self.operation == ItemOperation::ToggleAutoShot
            || (self.before.is_none() && self.after.is_none())
            || (self.before == self.after && self.count_delta == 0)
            || self.receipt.is_some_and(|id| id.is_nil())
        {
            return Err(ItemError("item ledger requires a real instance change"));
        }
        for item in self.before.iter().chain(self.after.iter()) {
            item.validate()?;
            if item.id != self.instance || item.revision > revision {
                return Err(ItemError("item delta identity/revision mismatch"));
            }
        }
        let before_count = self.before.as_ref().map_or(0, |item| item.count);
        let after_count = self.after.as_ref().map_or(0, |item| item.count);
        let difference = i128::from(after_count)
            .checked_sub(i128::from(before_count))
            .ok_or(ItemError("item count delta overflow"))?;
        if self.operation == ItemOperation::Consume {
            if self.count_delta > 0 || self.causes.is_empty() || self.operation_ordinal != 0 {
                return Err(ItemError("consume aggregate lacks ordered causes"));
            }
        } else if difference != i128::from(self.count_delta) {
            return Err(ItemError("item count delta mismatch"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemEnchantOutcome {
    Success,
    SafeFailed,
    BlessedReset,
    Destroyed,
    SuccessNoIncrement,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemRefineOutcome {
    Applied,
    Removed,
}

/// Frozen refund identities/counts survive merging into an existing stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemRefund {
    pub instance: ItemInstanceId,
    pub template: u32,
    pub count: u64,
}

/// Exact owner result; the API maps these to the pinned result messages and freezes bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(missing_docs)]
pub enum ItemResultDetail {
    Operation {
        operation: ItemOperation,
        affected: Vec<ItemInstanceId>,
        recipe: Option<u64>,
    },
    Enchant {
        target: ItemInstanceId,
        old_enchant: u32,
        new_enchant: u32,
        outcome: ItemEnchantOutcome,
        refunds: Vec<ItemRefund>,
        announcement_tier: u32,
    },
    Refine {
        target: ItemInstanceId,
        augment: Option<u32>,
        outcome: ItemRefineOutcome,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct FrozenItemResult {
    pub entity: EntityId,
    pub incarnation: u32,
    pub tick: Tick,
    pub inventory_revision: u64,
    pub receipt: uuid::Uuid,
    pub detail: ItemResultDetail,
}

/// Account-scoped request settlement. Pending starter/backfill fingerprints retain the
/// exact admitted repository string; normalized WS commands use their canonical body hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemReceiptFact {
    pub request_id: uuid::Uuid,
    pub fingerprint: String,
    pub result: FrozenItemResult,
}

/// Pending input completion is in the same transaction as ownership and the frozen result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemMaterializationFact {
    pub request_id: uuid::Uuid,
    pub manifest_version: u32,
    pub manifest_hash: [u8; 32],
    pub inventory_revision: u64,
    pub receipt: uuid::Uuid,
}

/// Private `PbOutput40` payload. The actor supplies the ordinary character checkpoint at
/// this same boundary and commits scalars/items/ledger/receipts/outbox under one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4Projection {
    pub tick: Tick,
    pub entity: EntityId,
    pub account: crate::domain::AccountId,
    pub checkpoint: Phase4Checkpoint,
    pub ledger: Vec<ItemDeltaPlan>,
    pub receipts: Vec<ItemReceiptFact>,
    pub materializations: Vec<ItemMaterializationFact>,
    pub evidence: Vec<ItemAuditEvidence>,
}
impl Phase4Projection {
    /// Enforce identity/version/order before the application can stage this private fact.
    pub fn validate_structure(&self, phase3_sources: usize) -> Result<(), ItemError> {
        self.checkpoint.validate_structure(phase3_sources)?;
        if self.checkpoint.starter_kit_receipt.is_none() {
            return Err(ItemError("projection precedes kit materialization"));
        }
        let mut final_items = BTreeMap::new();
        let mut rows = BTreeSet::new();
        let mut consume = BTreeSet::new();
        for delta in &self.ledger {
            delta.validate(self.checkpoint.inventory_revision)?;
            final_items.insert(delta.instance, delta.after.as_ref());
            if !rows.insert((delta.operation_ordinal, delta.instance, delta.operation))
                || (delta.operation == ItemOperation::Consume && !consume.insert(delta.instance))
            {
                return Err(ItemError("duplicate item ledger plan"));
            }
        }
        for (id, after) in final_items {
            if self.checkpoint.instances.iter().find(|item| item.id == id) != after {
                return Err(ItemError("ledger does not end at checkpoint inventory"));
            }
        }
        let mut receipts = BTreeSet::new();
        for receipt in &self.receipts {
            receipt.result.validate()?;
            if receipt.request_id.is_nil()
                || receipt.fingerprint.is_empty()
                || !receipts.insert(receipt.request_id)
                || receipt.result.entity != self.entity
                || receipt.result.receipt != receipt.request_id
                || receipt.result.tick != self.tick
                || receipt.result.inventory_revision > self.checkpoint.inventory_revision
            {
                return Err(ItemError("invalid item receipt projection"));
            }
        }
        if self
            .ledger
            .iter()
            .filter_map(|delta| delta.receipt)
            .any(|id| !receipts.contains(&id))
        {
            return Err(ItemError("item ledger receipt is absent"));
        }
        let mut pending = BTreeSet::new();
        for result in &self.materializations {
            if result.manifest_version != 1
                || !pending.insert(result.request_id)
                || result.receipt != result.request_id
                || !receipts.contains(&result.receipt)
                || result.inventory_revision > self.checkpoint.inventory_revision
            {
                return Err(ItemError("invalid pending item completion"));
            }
        }
        Ok(())
    }
}

impl FrozenItemResult {
    /// Frozen result shapes are checked before persistence or replay projection.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.receipt.is_nil() || i64::try_from(self.inventory_revision).is_err() {
            return Err(ItemError("invalid frozen item result identity"));
        }
        match &self.detail {
            ItemResultDetail::Operation {
                operation,
                affected,
                recipe,
            } => {
                if matches!(
                    operation,
                    ItemOperation::Consume
                        | ItemOperation::Enchant
                        | ItemOperation::Refine
                        | ItemOperation::CancelRefine
                ) || affected.windows(2).any(|ids| ids.first() >= ids.get(1))
                    || ((*operation == ItemOperation::Exchange) != recipe.is_some())
                    || recipe.is_some_and(|id| id == 0 || i64::try_from(id).is_err())
                {
                    return Err(ItemError("invalid frozen operation result"));
                }
            },
            ItemResultDetail::Enchant {
                old_enchant,
                new_enchant,
                outcome,
                refunds,
                ..
            } => {
                let valid = match outcome {
                    ItemEnchantOutcome::Success => old_enchant.checked_add(1) == Some(*new_enchant),
                    ItemEnchantOutcome::SafeFailed | ItemEnchantOutcome::SuccessNoIncrement => {
                        old_enchant == new_enchant
                    },
                    ItemEnchantOutcome::BlessedReset | ItemEnchantOutcome::Destroyed => {
                        *new_enchant == 0
                    },
                };
                let mut seen = BTreeSet::new();
                if !valid
                    || refunds.iter().any(|r| {
                        !seen.insert(r.instance)
                            || r.template == 0
                            || i32::try_from(r.template).is_err()
                            || r.count == 0
                            || i64::try_from(r.count).is_err()
                    })
                {
                    return Err(ItemError("invalid frozen enchant result"));
                }
            },
            ItemResultDetail::Refine {
                augment, outcome, ..
            } => {
                if !matches!(
                    (outcome, augment),
                    (ItemRefineOutcome::Applied, Some(1..=u32::MAX))
                        | (ItemRefineOutcome::Removed, None)
                ) {
                    return Err(ItemError("invalid frozen refine result"));
                }
            },
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::{phase4_checkpoint::StarterKitReceipt, ItemLocation};
    use super::*;
    fn projection() -> Phase4Projection {
        let key = uuid::Uuid::from_u128(3);
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        Phase4Projection {
            tick: Tick(10),
            entity,
            account: crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(2)),
            checkpoint: Phase4Checkpoint {
                schema_version: 1,
                timer_policy_version: 1,
                rules_hash: [1; 32],
                inventory_revision: 2,
                instances: Vec::new(),
                paperdoll: BTreeMap::new(),
                auto_shots: BTreeSet::from([1835]),
                item_reuse_remainders: Vec::new(),
                equipment_install_order: Vec::new(),
                active_sets: BTreeMap::new(),
                weight: 0,
                max_load: 1000,
                weight_penalty_bonus: 1,
                overload_level: 0,
                overloaded: false,
                expertise: 0,
                weapon_penalty: 0,
                armor_penalty: 0,
                starter_kit_receipt: Some(StarterKitReceipt {
                    manifest_version: 1,
                    manifest_hash: [2; 32],
                    source_class: crate::domain::class::ClassId(0),
                    request_id: uuid::Uuid::from_u128(4),
                    materialized_revision: 1,
                }),
            },
            ledger: Vec::new(),
            receipts: vec![ItemReceiptFact {
                request_id: key,
                fingerprint: "body".into(),
                result: FrozenItemResult {
                    entity,
                    incarnation: 1,
                    tick: Tick(10),
                    inventory_revision: 2,
                    receipt: key,
                    detail: ItemResultDetail::Operation {
                        operation: ItemOperation::ToggleAutoShot,
                        affected: Vec::new(),
                        recipe: None,
                    },
                },
            }],
            materializations: Vec::new(),
            evidence: Vec::new(),
        }
    }
    fn item(template_id: u32) -> ItemInstance {
        ItemInstance {
            id: ItemInstanceId::new(7).unwrap(),
            template_id,
            count: 1,
            location: ItemLocation::Inventory,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: Tick(0),
            revision: 2,
        }
    }
    #[test]
    fn toggle_is_critical_without_fabricated_ledger_and_exchange_keeps_both_templates() {
        let mut state = projection();
        state.validate_structure(0).unwrap();
        let before = item(1);
        let mut after = item(2);
        after.id = ItemInstanceId::new(8).unwrap();
        state.ledger.push(ItemDeltaPlan {
            operation: ItemOperation::Exchange,
            operation_ordinal: 1,
            instance: before.id,
            before: Some(before),
            after: None,
            count_delta: -1,
            receipt: None,
            causes: Vec::new(),
        });
        let delta = ItemDeltaPlan {
            operation: ItemOperation::Exchange,
            operation_ordinal: 1,
            instance: after.id,
            before: None,
            after: Some(after.clone()),
            count_delta: 1,
            receipt: None,
            causes: Vec::new(),
        };
        assert_eq!(delta.template().unwrap(), 2);
        state.ledger.push(delta);
        state.checkpoint.instances.push(after);
        state.validate_structure(0).unwrap();
        let roundtrip: Phase4Projection =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(roundtrip, state);
        state.ledger.first_mut().unwrap().operation = ItemOperation::ToggleAutoShot;
        assert!(state.validate_structure(0).is_err());
    }
    #[test]
    fn duplicate_receipts_unbacked_deltas_and_fractional_audit_values_are_refused() {
        let mut state = projection();
        state.receipts.push(state.receipts.first().unwrap().clone());
        assert!(state.validate_structure(0).is_err());
        let mut state = projection();
        state.ledger.push(ItemDeltaPlan {
            operation: ItemOperation::Grant,
            operation_ordinal: 0,
            instance: item(1).id,
            before: None,
            after: Some(item(1)),
            count_delta: 1,
            receipt: None,
            causes: Vec::new(),
        });
        assert!(state.validate_structure(0).is_err());
        assert!(serde_json::from_str::<ItemEvidenceValue>(r#"{"kind":"unsigned","value":1.5}"#)
            .is_err());
        let mut state = projection();
        state.receipts.first_mut().unwrap().result.entity =
            EntityId::from_uuid(uuid::Uuid::from_u128(99));
        assert!(state.validate_structure(0).is_err());
    }
}
