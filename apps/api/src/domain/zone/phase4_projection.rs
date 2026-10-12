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

/// Account is supplied by the enclosing projection; the request operation may differ
/// from a ledger operation (for example, Use pays a Consume or Equip removes a slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemReceiptRef {
    pub operation: ItemOperation,
    pub key: uuid::Uuid,
}
impl ItemReceiptRef {
    /// Automatic consumption is never an independently idempotent request.
    pub fn validate(self) -> Result<(), ItemError> {
        if self.key.is_nil() || self.operation == ItemOperation::Consume {
            return Err(ItemError("invalid item receipt scope"));
        }
        Ok(())
    }
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
    pub receipt: Option<ItemReceiptRef>,
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
        if let Some(receipt) = self.receipt {
            receipt.validate()?;
        }
        if self.operation == ItemOperation::ToggleAutoShot
            || (self.before.is_none() && self.after.is_none())
            || (self.before == self.after && self.count_delta == 0)
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
    #[serde(with = "result_payload")]
    pub result: FrozenItemResult,
}
impl ItemReceiptFact {
    /// Scope is derived from the frozen typed outcome, never the last ledger operation.
    #[must_use]
    pub const fn reference(&self) -> ItemReceiptRef {
        ItemReceiptRef {
            operation: self.result.operation(),
            key: self.request_id,
        }
    }
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
    pub receipt: ItemReceiptRef,
}

/// Private `PbOutput40` payload. The actor supplies the ordinary character checkpoint at
/// this same boundary and commits scalars/items/ledger/receipts/outbox under one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase4Projection {
    /// Every accepted attempt, including unchanged safe/bonus-only outcomes, in operation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enchant_attempts: Vec<super::EnchantAttempt>,
    /// Complete final paused owner for direct offline settlement; absent for live owners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_core: Option<Box<super::ItemPausedCore>>,
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
        if self.entity.as_uuid().is_nil()
            || self.account.as_uuid().is_nil()
            || i64::try_from(self.tick.0).is_err()
        {
            return Err(ItemError("invalid item projection owner or tick"));
        }
        self.checkpoint.validate_structure(phase3_sources)?;
        if let Some(core) = &self.paused_core {
            core.validate(self.account)?;
            if core.load.phase4.as_ref().is_none_or(|items| {
                !items.pending.is_empty() || items.checkpoint.as_ref() != Some(&self.checkpoint)
            }) || core
                .load
                .phase3
                .as_ref()
                .is_none_or(|skills| skills.modifier_install_order.len() != phase3_sources)
                || core.load.hp.is_none()
                || core.load.mp.is_none()
            {
                return Err(ItemError("paused final core differs from item projection"));
            }
        }
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
                || !receipts.insert(receipt.reference())
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
        let mut attempts = BTreeSet::new();
        let mut previous_ordinal = 0;
        for attempt in &self.enchant_attempts {
            let receipt = self
                .receipts
                .iter()
                .find(|r| r.reference() == attempt.receipt)
                .ok_or(ItemError("enchant attempt receipt is absent"))?;
            attempt.validate(receipt)?;
            if attempt.operation_ordinal <= previous_ordinal || !attempts.insert(attempt.receipt) {
                return Err(ItemError("duplicate or unordered enchant attempt"));
            }
            previous_ordinal = attempt.operation_ordinal;
        }
        if self.receipts.iter().any(|r| {
            r.reference().operation == ItemOperation::Enchant && !attempts.contains(&r.reference())
        }) {
            return Err(ItemError("enchant receipt lacks its captured attempt"));
        }
        let mut pending = BTreeSet::new();
        for result in &self.materializations {
            if result.manifest_version != 1
                || !pending.insert(result.receipt)
                || result.receipt.key != result.request_id
                || !matches!(
                    result.receipt.operation,
                    ItemOperation::Starter | ItemOperation::Backfill
                )
                || !receipts.contains(&result.receipt)
                || result.inventory_revision > self.checkpoint.inventory_revision
            {
                return Err(ItemError("invalid pending item completion"));
            }
        }
        Ok(())
    }
}

/// Canonical frozen result bytes shared by private receipts and recorded actor preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct ItemResultPayload(Vec<u8>);
impl ItemResultPayload {
    /// Exact original bytes; decoding never accepts whitespace, unknown fields or aliases.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    /// Recover the validated typed result without consulting mutable state.
    pub fn result(&self) -> Result<FrozenItemResult, ItemError> {
        FrozenItemResult::from_payload(&self.0)
    }
}
impl TryFrom<Vec<u8>> for ItemResultPayload {
    type Error = ItemError;
    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        FrozenItemResult::from_payload(&bytes)?;
        Ok(Self(bytes))
    }
}
impl From<ItemResultPayload> for Vec<u8> {
    fn from(payload: ItemResultPayload) -> Self {
        payload.0
    }
}

/// Receipts persist canonical payload bytes. Strict decoding proves that re-encoding the
/// immutable typed value reproduces those bytes exactly; no duplicated mutable result copy.
pub mod result_payload {
    use super::{FrozenItemResult, ItemResultPayload};
    use serde::{Deserialize as _, Serialize as _};
    /// Serialize receipt payload bytes, rather than a second editable JSON object.
    pub fn serialize<S: serde::Serializer>(
        result: &FrozenItemResult,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        result
            .payload()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
    /// Reject noncanonical/malformed stored bytes at the repository/replay boundary.
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<FrozenItemResult, D::Error> {
        ItemResultPayload::deserialize(deserializer)?
            .result()
            .map_err(serde::de::Error::custom)
    }
}

impl FrozenItemResult {
    /// Canonical payload used verbatim by replay result tags37–39 and receipt storage.
    pub fn payload(&self) -> Result<ItemResultPayload, ItemError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(ItemResultPayload)
            .map_err(|_| ItemError("item result encoding failed"))
    }
    /// Accept exactly one validated canonical representation.
    pub fn from_payload(bytes: &[u8]) -> Result<Self, ItemError> {
        let result: Self =
            serde_json::from_slice(bytes).map_err(|_| ItemError("invalid item result payload"))?;
        if result.payload()?.as_bytes() != bytes {
            return Err(ItemError("noncanonical item result payload"));
        }
        Ok(result)
    }

    /// Stable idempotency operation, including the two distinct refine requests.
    #[must_use]
    pub const fn operation(&self) -> ItemOperation {
        match &self.detail {
            ItemResultDetail::Operation { operation, .. } => *operation,
            ItemResultDetail::Enchant { .. } => ItemOperation::Enchant,
            ItemResultDetail::Refine {
                outcome: ItemRefineOutcome::Applied,
                ..
            } => ItemOperation::Refine,
            ItemResultDetail::Refine {
                outcome: ItemRefineOutcome::Removed,
                ..
            } => ItemOperation::CancelRefine,
        }
    }

    /// Frozen result shapes are checked before persistence or replay projection.
    pub fn validate(&self) -> Result<(), ItemError> {
        if self.receipt.is_nil()
            || self.entity.as_uuid().is_nil()
            || i64::try_from(self.tick.0).is_err()
            || i64::try_from(self.inventory_revision).is_err()
        {
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
pub(crate) mod tests {
    use super::super::{phase4_checkpoint::StarterKitReceipt, ItemLocation};
    use super::*;
    pub(crate) fn projection() -> Phase4Projection {
        let key = uuid::Uuid::from_u128(3);
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
        Phase4Projection {
            enchant_attempts: Vec::new(),
            paused_core: None,
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
                ordinary_capacity: 80,
                quest_capacity: 100,
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
    #[test]
    fn receipt_json_persists_exact_canonical_result_bytes_and_rejects_aliases() {
        let fact = projection().receipts.remove(0);
        let payload = fact.result.payload().unwrap();
        assert_eq!(payload.result().unwrap(), fact.result);
        let json = serde_json::to_value(&fact).unwrap();
        assert_eq!(json.get("result").unwrap(), &serde_json::to_value(payload.as_bytes()).unwrap());
        let roundtrip: ItemReceiptFact = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(roundtrip.result.payload().unwrap().as_bytes(), payload.as_bytes());
        let mut bytes = payload.as_bytes().to_vec();
        bytes.push(b' ');
        assert!(ItemResultPayload::try_from(bytes.clone()).is_err());
        let mut invalid = json;
        *invalid.get_mut("result").unwrap() = serde_json::to_value(bytes).unwrap();
        assert!(serde_json::from_value::<ItemReceiptFact>(invalid).is_err());
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
    fn reused_uuid_keeps_each_operation_scope_and_implicit_ledger_reference() {
        let mut state = projection();
        let mut equip = state.receipts.first().unwrap().clone();
        equip.result.detail = ItemResultDetail::Operation {
            operation: ItemOperation::Equip,
            affected: vec![item(1).id],
            recipe: None,
        };
        equip.fingerprint = "equip-body".into();
        let mut unequip = equip.clone();
        unequip.result.detail = ItemResultDetail::Operation {
            operation: ItemOperation::Unequip,
            affected: vec![item(1).id],
            recipe: None,
        };
        unequip.fingerprint = "unequip-body".into();
        let reference = equip.reference();
        state.receipts.extend([equip, unequip]);
        let after = item(1);
        let mut before = after.clone();
        before.location = ItemLocation::Paperdoll;
        before.slot = Some(super::super::EquipSlot::Rhand);
        state.checkpoint.instances.push(after.clone());
        state.ledger.push(ItemDeltaPlan {
            operation: ItemOperation::Unequip,
            operation_ordinal: 1,
            instance: after.id,
            before: Some(before),
            after: Some(after),
            count_delta: 0,
            receipt: Some(reference),
            causes: Vec::new(),
        });
        state.validate_structure(0).unwrap();
        let bytes = serde_json::to_vec(&state).unwrap();
        let restored: Phase4Projection = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, state);
        let mut absent = state.clone();
        absent
            .ledger
            .first_mut()
            .unwrap()
            .receipt
            .as_mut()
            .unwrap()
            .operation = ItemOperation::Use;
        assert!(absent.validate_structure(0).is_err());
        let mut automatic = state.clone();
        automatic
            .ledger
            .first_mut()
            .unwrap()
            .receipt
            .as_mut()
            .unwrap()
            .operation = ItemOperation::Consume;
        assert!(automatic.validate_structure(0).is_err());
        let mut duplicate = state.receipts.last().unwrap().clone();
        duplicate.fingerprint = "conflicting-body".into();
        state.receipts.push(duplicate);
        assert!(state.validate_structure(0).is_err());
    }

    #[test]
    fn pending_completion_requires_the_exact_kit_receipt_scope() {
        let mut state = projection();
        let mut receipt = state.receipts.first().unwrap().clone();
        receipt.result.detail = ItemResultDetail::Operation {
            operation: ItemOperation::Starter,
            affected: Vec::new(),
            recipe: None,
        };
        state.materializations.push(ItemMaterializationFact {
            request_id: receipt.request_id,
            manifest_version: 1,
            manifest_hash: [2; 32],
            inventory_revision: 2,
            receipt: receipt.reference(),
        });
        state.receipts.push(receipt);
        state.validate_structure(0).unwrap();
        state
            .materializations
            .first_mut()
            .unwrap()
            .receipt
            .operation = ItemOperation::Backfill;
        assert!(state.validate_structure(0).is_err());
        state
            .materializations
            .first_mut()
            .unwrap()
            .receipt
            .operation = ItemOperation::ToggleAutoShot;
        assert!(state.validate_structure(0).is_err());
    }

    fn attempted(outcome: ItemEnchantOutcome) -> Phase4Projection {
        let mut state = projection();
        let mut before = item(71);
        before.enchant = 6;
        let new_enchant = match outcome {
            ItemEnchantOutcome::Success => 7,
            ItemEnchantOutcome::SafeFailed | ItemEnchantOutcome::SuccessNoIncrement => 6,
            ItemEnchantOutcome::BlessedReset | ItemEnchantOutcome::Destroyed => 0,
        };
        let mut after = before.clone();
        after.enchant = new_enchant;
        let receipt = state.receipts.first_mut().unwrap();
        receipt.result.detail = ItemResultDetail::Enchant {
            target: before.id,
            old_enchant: 6,
            new_enchant,
            outcome,
            refunds: Vec::new(),
            announcement_tier: 0,
        };
        let success =
            matches!(outcome, ItemEnchantOutcome::Success | ItemEnchantOutcome::SuccessNoIncrement);
        state.enchant_attempts.push(super::super::EnchantAttempt {
            operation_ordinal: 1,
            receipt: receipt.reference(),
            target: before.id,
            template: before.template_id,
            old_enchant: 6,
            new_enchant,
            outcome,
            before,
            after: (outcome != ItemEnchantOutcome::Destroyed).then_some(after),
            scroll: ItemInstanceId::new(20).unwrap(),
            scroll_template: 951,
            chance_1e5: if success { 100_000 } else { 0 },
            raw_draw: 199_999,
            draw: 99_999,
            cause: ItemAuditEvidence {
                family: "enchant".into(),
                fields: BTreeMap::new(),
            },
        });
        state
    }

    #[test]
    fn captured_enchant_attempt_survives_later_target_destruction() {
        for outcome in [
            ItemEnchantOutcome::Success,
            ItemEnchantOutcome::SafeFailed,
            ItemEnchantOutcome::BlessedReset,
            ItemEnchantOutcome::Destroyed,
            ItemEnchantOutcome::SuccessNoIncrement,
        ] {
            let mut state = attempted(outcome);
            let attempt = state.enchant_attempts.first().unwrap().clone();
            // Final inventory contains no target: audit still retains the exact attempt.
            if let Some(after) = &attempt.after {
                state.ledger.push(ItemDeltaPlan {
                    operation: ItemOperation::Destroy,
                    operation_ordinal: 2,
                    instance: attempt.target,
                    before: Some(after.clone()),
                    after: None,
                    count_delta: -1,
                    receipt: None,
                    causes: Vec::new(),
                });
            }
            state.validate_structure(0).unwrap();
            assert!(state.checkpoint.instances.is_empty());
            let bytes = serde_json::to_vec(&state).unwrap();
            assert_eq!(serde_json::from_slice::<Phase4Projection>(&bytes).unwrap(), state);
            let event = super::super::ZoneEvent::ItemCheckpoint(Box::new(state));
            let record = crate::application::replay_log::AppliedTickRecord::from_applied(
                super::super::ZoneId(1),
                &super::super::AppliedTick {
                    epoch: 1,
                    tick: Tick(10),
                    server_time_ms: 1000,
                    commands: Vec::new(),
                    dispositions: Vec::new(),
                    events: vec![event],
                    outputs: BTreeMap::new(),
                    state_digest: [0; 32],
                    digest_version: super::super::StateDigestVersion::BinaryV6,
                },
            );
            assert_eq!(
                crate::application::replay_log::AppliedTickRecord::decode(&record.encode())
                    .unwrap(),
                record
            );
        }
    }

    #[test]
    fn enchant_attempt_requires_receipt_scalar_and_quantized_draw_identity() {
        let state = attempted(ItemEnchantOutcome::SuccessNoIncrement);
        for case in 0..10 {
            let mut invalid = state.clone();
            let attempt = invalid.enchant_attempts.first_mut().unwrap();
            match case {
                0 => attempt.draw = 100_000,
                1 => attempt.chance_1e5 = 100_001,
                2 => attempt.raw_draw = 0,
                3 => attempt.after = None,
                4 => attempt.before.template_id = 72,
                5 => attempt.scroll = attempt.target,
                6 => attempt.old_enchant = 5,
                7 => attempt.receipt.key = uuid::Uuid::from_u128(99),
                8 => attempt.cause.family.clear(),
                _ => attempt.operation_ordinal = 0,
            }
            assert!(invalid.validate_structure(0).is_err(), "case {case}");
        }
        let mut absent = state.clone();
        absent.enchant_attempts.clear();
        assert!(absent.validate_structure(0).is_err());
        let mut duplicate = state.clone();
        duplicate
            .enchant_attempts
            .push(state.enchant_attempts.first().unwrap().clone());
        assert!(duplicate.validate_structure(0).is_err());
        let mut json = serde_json::to_value(state.enchant_attempts.first().unwrap()).unwrap();
        *json.get_mut("draw").unwrap() = serde_json::from_str("0.5").unwrap();
        assert!(serde_json::from_value::<super::super::EnchantAttempt>(json).is_err());
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
