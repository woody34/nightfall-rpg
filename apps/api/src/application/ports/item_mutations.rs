//! Atomic, account-scoped settlement of already acknowledged private zone facts.
use super::{CharacterCheckpoint, ItemLedgerDelta, ItemLedgerEntry, ItemLedgerOperation};
use crate::application::{CheckpointError, CheckpointOutcome, IdempotencyKey};
use crate::domain::zone::phase4_projection::{
    FrozenItemResult, ItemOperation, ItemRefineOutcome, ItemResultDetail,
};
use crate::domain::zone::{ItemInstanceRange, Phase4Checkpoint, Phase4Projection, ZoneId};
use crate::domain::{AccountId, CharacterId, DomainEvent};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Exactly the account/operation scope of the SQL request keys and frozen receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemMutationOperation {
    EquipItem,
    UnequipItem,
    UseItem,
    DestroyItem,
    EnchantItem,
    ToggleAutoShot,
    RefineItem,
    CancelRefine,
    ExchangeSpecialAbility,
    CrystallizeItem,
    GrantItem,
    Starter,
    Backfill,
}
impl ItemMutationOperation {
    /// Stable request operation names, independent of the per-instance ledger vocabulary.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EquipItem => "equip_item",
            Self::UnequipItem => "unequip_item",
            Self::UseItem => "use_item",
            Self::DestroyItem => "destroy_item",
            Self::EnchantItem => "enchant_item",
            Self::ToggleAutoShot => "toggle_auto_shot",
            Self::RefineItem => "refine_item",
            Self::CancelRefine => "cancel_refine",
            Self::ExchangeSpecialAbility => "exchange_special_ability",
            Self::CrystallizeItem => "crystallize_item",
            Self::GrantItem => "grant_item",
            Self::Starter => "starter",
            Self::Backfill => "backfill",
        }
    }
    /// The typed frozen result binds its operation; automatic consumption has no request.
    pub fn from_result(result: &FrozenItemResult) -> Result<Self, CheckpointError> {
        Ok(match result.detail {
            ItemResultDetail::Enchant { .. } => Self::EnchantItem,
            ItemResultDetail::Refine {
                outcome: ItemRefineOutcome::Applied,
                ..
            } => Self::RefineItem,
            ItemResultDetail::Refine {
                outcome: ItemRefineOutcome::Removed,
                ..
            } => Self::CancelRefine,
            ItemResultDetail::Operation { operation, .. } => match operation {
                ItemOperation::Equip => Self::EquipItem,
                ItemOperation::Unequip => Self::UnequipItem,
                ItemOperation::Use => Self::UseItem,
                ItemOperation::Destroy => Self::DestroyItem,
                ItemOperation::Crystallize => Self::CrystallizeItem,
                ItemOperation::ToggleAutoShot => Self::ToggleAutoShot,
                ItemOperation::Exchange => Self::ExchangeSpecialAbility,
                ItemOperation::Grant => Self::GrantItem,
                ItemOperation::Starter => Self::Starter,
                ItemOperation::Backfill => Self::Backfill,
                ItemOperation::Consume
                | ItemOperation::Enchant
                | ItemOperation::Refine
                | ItemOperation::CancelRefine => return Err(invalid("item_receipt_operation")),
            },
        })
    }
}

/// `GrantItem` includes a frozen baseline; retries must never reread a later inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct FrozenItemInventory {
    pub character_revision: u64,
    pub phase3_source_count: u32,
    pub checkpoint: Phase4Checkpoint,
    pub ordinary_capacity: u32,
    pub quest_capacity: u32,
}

/// API-owned immutable success, bound to the authenticated account before simulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemMutationReceipt {
    pub account_id: AccountId,
    pub character_id: CharacterId,
    pub operation: ItemMutationOperation,
    pub key: uuid::Uuid,
    pub fingerprint: String,
    pub result: FrozenItemResult,
    pub inventory: Option<FrozenItemInventory>,
}
impl ItemMutationReceipt {
    /// Verify duplicate identities and the entire response without querying mutable state.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        self.result
            .validate()
            .map_err(|_| invalid("item_receipt_result"))?;
        if i64::try_from(self.result.tick.0).is_err()
            || self.key.is_nil()
            || self.result.receipt != self.key
            || self.result.entity.as_uuid() != self.character_id.as_uuid()
            || self.fingerprint.is_empty()
            || ItemMutationOperation::from_result(&self.result)? != self.operation
            || (self.operation == ItemMutationOperation::GrantItem) != self.inventory.is_some()
        {
            return Err(invalid("item_receipt_identity"));
        }
        if let Some(inventory) = &self.inventory {
            inventory
                .checkpoint
                .validate_structure(
                    usize::try_from(inventory.phase3_source_count).map_err(anyhow::Error::from)?,
                )
                .map_err(|_| invalid("item_receipt_inventory"))?;
            if i64::try_from(inventory.character_revision).is_err()
                || inventory.checkpoint.inventory_revision < self.result.inventory_revision
                || inventory.checkpoint.starter_kit_receipt.is_none()
                || inventory.ordinary_capacity == 0
                || inventory.quest_capacity == 0
            {
                return Err(invalid("item_receipt_inventory"));
            }
        }
        Ok(())
    }
}

/// Exact immutable response, independent of current session/eligibility/inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemReceiptLookup {
    /// No successful request in this account/operation scope.
    Unknown,
    /// Same key names a different normalized body.
    Conflict,
    /// Frozen original result, including the Grant baseline when applicable.
    Known(Box<ItemMutationReceipt>),
}

/// Receipts are checked in authenticated actor preflight, before drafting or consuming RNG.
#[async_trait]
pub trait ItemReceiptRepository: Send + Sync {
    /// Account and operation are trusted normalized context, never client-supplied ownership.
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<ItemReceiptLookup>;
}

/// The actor supplies enclosing epoch/range context from the acknowledged tick snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemCheckpointWork {
    pub zone: ZoneId,
    pub epoch: u64,
    pub allocation: ItemInstanceRange,
    pub projection: Phase4Projection,
    pub grant_capacities: Option<(u32, u32)>,
}
impl ItemCheckpointWork {
    /// Structural agreement with the ordinary checkpoint; contextual fold validation is zone-owned.
    pub fn validate(
        &self,
        cp: &CharacterCheckpoint,
        owner: AccountId,
    ) -> Result<(), CheckpointError> {
        let phase3 = cp
            .phase3
            .as_ref()
            .ok_or_else(|| invalid("item_checkpoint_skills"))?;
        self.projection
            .validate_structure(phase3.modifier_install_order.len())
            .map_err(|_| invalid("item_checkpoint_projection"))?;
        if self.projection.entity.as_uuid() != cp.character_id.as_uuid()
            || self.projection.account != owner
            || cp.phase4.as_ref() != Some(&self.projection.checkpoint)
            || i32::try_from(self.zone.0).is_err()
            || i64::try_from(self.epoch).is_err()
            || i64::try_from(self.projection.tick.0).is_err()
        {
            return Err(invalid("item_checkpoint_identity"));
        }
        for row in self.ledger_entries()? {
            row.validate()?;
        }
        for receipt in self.receipts(cp)? {
            receipt.validate()?;
        }
        Ok(())
    }

    /// Every changed scalar needs a real ledger fact. Sidecar-only changes need no fake ID.
    pub fn validate_before(
        &self,
        before: Option<&Phase4Checkpoint>,
    ) -> Result<(), CheckpointError> {
        let kit = self
            .projection
            .checkpoint
            .starter_kit_receipt
            .as_ref()
            .ok_or_else(|| invalid("pending_kit_receipt"))?;
        if let Some(retained) = before.and_then(|cp| cp.starter_kit_receipt.as_ref()) {
            if retained != kit {
                return Err(invalid("item_starter_receipt_immutable"));
            }
        } else if !self
            .projection
            .materializations
            .iter()
            .any(|fact| fact.request_id == kit.request_id)
        {
            return Err(invalid("pending_kit_materialization_required"));
        }
        if before.is_some_and(|cp| cp.auto_shots != self.projection.checkpoint.auto_shots)
            && !self.projection.receipts.iter().any(|receipt| {
                matches!(
                    receipt.result.detail,
                    ItemResultDetail::Operation {
                        operation: ItemOperation::ToggleAutoShot,
                        ..
                    }
                )
            })
        {
            return Err(invalid("item_toggle_receipt_required"));
        }
        let old: BTreeMap<_, _> = before
            .into_iter()
            .flat_map(|cp| &cp.instances)
            .map(|i| (i.id, i))
            .collect();
        let new: BTreeMap<_, _> = self
            .projection
            .checkpoint
            .instances
            .iter()
            .map(|i| (i.id, i))
            .collect();
        let (start, next, _) = self.allocation.bounds();
        for id in old.keys().chain(new.keys()) {
            if old.get(id) != new.get(id)
                && !self
                    .projection
                    .ledger
                    .iter()
                    .any(|delta| delta.instance == *id)
            {
                return Err(invalid("item_checkpoint_unledgered_scalar"));
            }
        }
        // Aggregate consumption may overlap intervening operations, but all signed deltas
        // together must conserve the committed count of each actual instance.
        for id in old.keys().chain(new.keys()) {
            let deltas: Vec<_> = self
                .projection
                .ledger
                .iter()
                .filter(|delta| delta.instance == *id)
                .collect();
            if deltas.is_empty() {
                continue;
            }
            if let Some(initial) = old.get(id) {
                if !deltas
                    .iter()
                    .any(|delta| delta.before.as_ref() == Some(*initial))
                {
                    return Err(invalid("item_ledger_initial_scalar"));
                }
            }
            let total_delta = deltas.iter().try_fold(0_i128, |total, delta| {
                total
                    .checked_add(i128::from(delta.count_delta))
                    .ok_or_else(|| invalid("item_ledger_count_overflow"))
            })?;
            let expected = i128::from(new.get(id).map_or(0, |item| item.count))
                .checked_sub(i128::from(old.get(id).map_or(0, |item| item.count)))
                .ok_or_else(|| invalid("item_ledger_count_overflow"))?;
            if total_delta != expected {
                return Err(invalid("item_ledger_count_conservation"));
            }
        }
        for delta in &self.projection.ledger {
            if delta.before.is_none()
                && !old.contains_key(&delta.instance)
                && !(start <= delta.instance.get() && delta.instance.get() < next)
            {
                return Err(invalid("item_checkpoint_allocator"));
            }
        }
        if before.is_some_and(|cp| {
            cp.rules_hash != self.projection.checkpoint.rules_hash
                || cp.inventory_revision > self.projection.checkpoint.inventory_revision
        }) {
            return Err(invalid("item_checkpoint_downgrade"));
        }
        Ok(())
    }

    /// Audit events belong to this owner and this same committed parent revision.
    #[allow(clippy::manual_let_else)] // Exhaustive non-item arms force review for new event variants.
    pub fn validate_events(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<(), CheckpointError> {
        let revision = cp
            .revision_seen
            .checked_add(1)
            .ok_or_else(|| invalid("characters_revision"))?;
        for event in events {
            let fact = match event {
                DomainEvent::ItemGranted { fact }
                | DomainEvent::ItemEquipped { fact }
                | DomainEvent::ItemUnequipped { fact }
                | DomainEvent::ItemConsumed { fact }
                | DomainEvent::ItemEnchanted { fact }
                | DomainEvent::ItemDestroyed { fact }
                | DomainEvent::ItemRefined { fact }
                | DomainEvent::ItemExchanged { fact } => fact,
                DomainEvent::CharacterCreated { .. }
                | DomainEvent::CharacterLeveled { .. }
                | DomainEvent::CharacterClassChanged { .. }
                | DomainEvent::CharacterSkillLearned { .. }
                | DomainEvent::CharacterDied { .. }
                | DomainEvent::CharacterTokenGranted { .. } => continue,
            };
            fact.validate().map_err(|_| invalid("item_outbox_fact"))?;
            if fact.character_id != cp.character_id
                || fact.metadata.sequence.0 != revision
                || fact.metadata.event_id.is_nil()
                || !self.projection.ledger.iter().any(|delta| delta.instance == fact.instance_id)
                    && !self.projection.receipts.iter().any(|receipt| matches!(receipt.result.detail, ItemResultDetail::Enchant { target, .. } if target == fact.instance_id))
            {
                return Err(invalid("item_outbox_identity"));
            }
        }
        Ok(())
    }

    /// Validate completion against the frozen pending request, including kit lineage.
    pub fn validate_pending(
        &self,
        requests: &[super::PendingItemRequest],
    ) -> Result<(), CheckpointError> {
        use super::{PendingItemKind, PendingItemManifest, PendingItemStatus};
        for completion in &self.projection.materializations {
            let pending = requests
                .iter()
                .find(|request| request.request_id == completion.request_id)
                .ok_or_else(|| invalid("pending_item_completion_missing"))?;
            let receipt = self
                .projection
                .receipts
                .iter()
                .find(|fact| fact.request_id == completion.receipt)
                .ok_or_else(|| invalid("pending_item_completion_receipt"))?;
            let expected = match pending.kind {
                PendingItemKind::Starter => ItemMutationOperation::Starter,
                PendingItemKind::Backfill => ItemMutationOperation::Backfill,
                PendingItemKind::Grant => ItemMutationOperation::GrantItem,
            };
            if pending.character_id.as_uuid() != self.projection.entity.as_uuid()
                || pending.status != PendingItemStatus::Pending
                || pending.manifest_version != completion.manifest_version
                || pending.manifest_hash != completion.manifest_hash
                || pending.fingerprint != receipt.fingerprint
                || ItemMutationOperation::from_result(&receipt.result)? != expected
                || receipt.result.inventory_revision != completion.inventory_revision
            {
                return Err(invalid("pending_item_completion_identity"));
            }
            if let PendingItemManifest::Kit(manifest) = &pending.manifest {
                let kit = self
                    .projection
                    .checkpoint
                    .starter_kit_receipt
                    .as_ref()
                    .ok_or_else(|| invalid("pending_kit_receipt"))?;
                if kit.request_id != completion.receipt
                    || kit.manifest_hash != completion.manifest_hash
                    || kit.manifest_version != completion.manifest_version
                    || kit.source_class != manifest.source_class_id
                    || kit.materialized_revision != completion.inventory_revision
                {
                    return Err(invalid("pending_kit_receipt"));
                }
            }
        }
        Ok(())
    }

    /// Ordered SQL rows preserve explicit consumption count and exact typed audit causes.
    pub fn ledger_entries(&self) -> Result<Vec<ItemLedgerEntry>, CheckpointError> {
        self.projection
            .ledger
            .iter()
            .map(|delta| {
                Ok(ItemLedgerEntry {
                    character_id: CharacterId::from_uuid(self.projection.entity.as_uuid()),
                    zone_id: self.zone,
                    epoch: self.epoch,
                    tick: self.projection.tick,
                    revision: self.projection.checkpoint.inventory_revision,
                    operation: ledger_operation(delta.operation)?,
                    instance_id: delta.instance,
                    operation_ordinal: delta.operation_ordinal,
                    delta: ItemLedgerDelta {
                        before: delta.before.clone(),
                        after: delta.after.clone(),
                        count_delta: delta.count_delta,
                        causes: delta
                            .causes
                            .iter()
                            .map(serde_json::to_value)
                            .collect::<Result<_, _>>()
                            .map_err(anyhow::Error::from)?,
                    },
                    receipt_ref: delta.receipt,
                })
            })
            .collect()
    }

    /// Freeze Grant's complete final tick baseline alongside its exact operation result.
    pub fn receipts(
        &self,
        cp: &CharacterCheckpoint,
    ) -> Result<Vec<ItemMutationReceipt>, CheckpointError> {
        self.projection
            .receipts
            .iter()
            .map(|fact| {
                let operation = ItemMutationOperation::from_result(&fact.result)?;
                let inventory = if operation == ItemMutationOperation::GrantItem {
                    let (ordinary_capacity, quest_capacity) = self
                        .grant_capacities
                        .ok_or_else(|| invalid("grant_inventory_capacity"))?;
                    Some(FrozenItemInventory {
                        phase3_source_count: u32::try_from(
                            cp.phase3
                                .as_ref()
                                .ok_or_else(|| invalid("item_checkpoint_skills"))?
                                .modifier_install_order
                                .len(),
                        )
                        .map_err(anyhow::Error::from)?,
                        character_revision: cp
                            .revision_seen
                            .checked_add(1)
                            .ok_or_else(|| invalid("characters_revision"))?,
                        checkpoint: self.projection.checkpoint.clone(),
                        ordinary_capacity,
                        quest_capacity,
                    })
                } else {
                    None
                };
                Ok(ItemMutationReceipt {
                    account_id: self.projection.account,
                    character_id: cp.character_id,
                    operation,
                    key: fact.request_id,
                    fingerprint: fact.fingerprint.clone(),
                    result: fact.result.clone(),
                    inventory,
                })
            })
            .collect()
    }

    /// Include private work in checkpoint idempotency, never in legacy fingerprint bytes.
    pub fn fingerprint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        learning: Option<&super::SuccessfulLearnReceipt>,
    ) -> Result<String, CheckpointError> {
        use sha2::{Digest, Sha256};
        let checkpoint = learning.map_or_else(
            || Ok(cp.fingerprint(events)),
            |receipt| receipt.checkpoint_fingerprint(cp, events),
        )?;
        let value = serde_json::json!({"checkpoint":checkpoint,"items":self});
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&value).map_err(anyhow::Error::from)?)
        ))
    }
}

/// Same transaction as character progression, learning costs, rows, ledger, receipts and outbox.
#[async_trait]
pub trait ItemCheckpointRepository: ItemReceiptRepository + Send + Sync {
    /// Called only after durable log acknowledgement; retries retain the original immutable work.
    async fn checkpoint_items(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &ItemCheckpointWork,
        learning: Option<&super::SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError>;
}

fn ledger_operation(operation: ItemOperation) -> Result<ItemLedgerOperation, CheckpointError> {
    Ok(match operation {
        ItemOperation::Grant => ItemLedgerOperation::Grant,
        ItemOperation::Starter => ItemLedgerOperation::Starter,
        ItemOperation::Backfill => ItemLedgerOperation::Backfill,
        ItemOperation::Equip => ItemLedgerOperation::Equip,
        ItemOperation::Unequip => ItemLedgerOperation::Unequip,
        ItemOperation::Consume => ItemLedgerOperation::Consume,
        ItemOperation::Enchant => ItemLedgerOperation::Enchant,
        ItemOperation::Destroy => ItemLedgerOperation::Destroy,
        ItemOperation::Crystallize => ItemLedgerOperation::Crystallize,
        ItemOperation::Refine => ItemLedgerOperation::Refine,
        ItemOperation::CancelRefine => ItemLedgerOperation::CancelRefine,
        ItemOperation::Exchange => ItemLedgerOperation::Exchange,
        ItemOperation::Use => ItemLedgerOperation::Use,
        ItemOperation::ToggleAutoShot => return Err(invalid("item_ledger_owner_sidecar")),
    })
}
fn invalid(name: &str) -> CheckpointError {
    CheckpointError::Constraint(name.into())
}
