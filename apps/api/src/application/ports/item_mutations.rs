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
use std::collections::{BTreeMap, BTreeSet};

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
    /// Canonical command scope, independent of any resulting per-instance ledger operation.
    pub const fn from_command_kind(kind: &crate::domain::zone::ItemCommandKind) -> Self {
        use crate::domain::zone::ItemCommandKind as K;
        match kind {
            K::Equip { .. } => Self::EquipItem,
            K::Unequip { .. } => Self::UnequipItem,
            K::Use { .. } => Self::UseItem,
            K::Destroy { .. } => Self::DestroyItem,
            K::Enchant { .. } => Self::EnchantItem,
            K::ToggleAutoShot { .. } => Self::ToggleAutoShot,
            K::Refine { .. } => Self::RefineItem,
            K::CancelRefine { .. } => Self::CancelRefine,
            K::ExchangeSpecialAbility { .. } => Self::ExchangeSpecialAbility,
            K::Crystallize { .. } => Self::CrystallizeItem,
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
    #[serde(with = "crate::domain::zone::phase4_projection::result_payload")]
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
            || self.result.request_fingerprint != self.fingerprint
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
                || inventory.ordinary_capacity != inventory.checkpoint.ordinary_capacity
                || inventory.quest_capacity != inventory.checkpoint.quest_capacity
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
    Conflict {
        /// Immutable repository fingerprint, recorded before gameplay.
        stored_fingerprint: String,
    },
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
        if let Some(core) = &self.projection.paused_core {
            let expected = crate::application::checkpoint::paused::paused_checkpoint(
                core,
                &self.projection,
                self.zone,
                self.epoch,
            )
            .map_err(CheckpointError::from)?;
            if &expected != cp {
                return Err(invalid("item_checkpoint_paused_core"));
            }
        }
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

    /// Immutable catalogue invariants are checked at the storage trust boundary as well.
    pub fn validate_catalogue(
        &self,
        rules: &crate::domain::zone::Phase4Rules,
    ) -> Result<(), CheckpointError> {
        for item in
            self.projection
                .checkpoint
                .instances
                .iter()
                .chain(
                    self.projection
                        .ledger
                        .iter()
                        .flat_map(|delta| delta.before.iter().chain(delta.after.iter())),
                )
                .chain(self.projection.enchant_attempts.iter().flat_map(|attempt| {
                    std::iter::once(&attempt.before).chain(attempt.after.iter())
                }))
                .chain(self.projection.exchanges.iter().flat_map(|exchange| {
                    exchange
                        .rows
                        .iter()
                        .flat_map(|row| row.before.iter().chain(row.after.iter()))
                }))
        {
            let definition = rules
                .parts()
                .items
                .get(&item.template_id)
                .ok_or_else(|| invalid("item_checkpoint_template"))?;
            if !rules
                .parts()
                .capabilities
                .get(&item.template_id)
                .is_some_and(|capability| capability.storable)
                || (!definition.stackable && item.count != 1)
            {
                return Err(invalid("item_checkpoint_stackability"));
            }
        }
        self.validate_count_caps(rules)?;
        // Retained creation ticks belong to their origin epoch, whose clock may be ahead.
        for item in self
            .projection
            .ledger
            .iter()
            .filter(|delta| delta.before.is_none())
            .flat_map(|delta| delta.after.iter())
        {
            if item.created_tick > self.projection.tick
                || item
                    .creation_origin
                    .is_some_and(|origin| origin.zone != self.zone || origin.epoch != self.epoch)
            {
                return Err(invalid("item_checkpoint_creation_tick"));
            }
        }
        Ok(())
    }

    /// D68/D70 caps under D74: retained (tick-end), tick-start, enchant/exchange and Consume
    /// `after` states are capped directly. The canonical export (positive rows first) may show a
    /// row `after` above the cap only when the instance's later same-tick consumption brings it
    /// back: `after.count - later consumption <= cap`, all in i128.
    fn validate_count_caps(
        &self,
        rules: &crate::domain::zone::Phase4Rules,
    ) -> Result<(), CheckpointError> {
        let capped = |item: &crate::domain::zone::ItemInstance, consumed_later: i128| {
            let stackable = rules
                .parts()
                .items
                .get(&item.template_id)
                .ok_or_else(|| invalid("item_checkpoint_template"))?
                .stackable;
            if i128::from(item.count)
                .checked_sub(consumed_later)
                .is_none_or(|count| count > pinned_count_cap(item.template_id, stackable))
            {
                return Err(invalid("item_count_cap"));
            }
            Ok(())
        };
        for item in
            self.projection
                .checkpoint
                .instances
                .iter()
                .chain(self.projection.enchant_attempts.iter().flat_map(|attempt| {
                    std::iter::once(&attempt.before).chain(attempt.after.iter())
                }))
                .chain(self.projection.exchanges.iter().flat_map(|exchange| {
                    exchange
                        .rows
                        .iter()
                        .flat_map(|row| row.before.iter().chain(row.after.iter()))
                }))
        {
            capped(item, 0)?;
        }
        let ledger = &self.projection.ledger;
        for (index, delta) in ledger.iter().enumerate() {
            let (earlier, later) = ledger.split_at(index);
            if let Some(start) = delta.before.as_ref() {
                if !earlier.iter().any(|row| row.instance == delta.instance) {
                    capped(start, 0)?;
                }
            }
            if let Some(after) = delta.after.as_ref() {
                let mut consumed_later = 0_i128;
                for row in later.iter().skip(1).filter(|row| {
                    row.instance == delta.instance && row.operation == ItemOperation::Consume
                }) {
                    consumed_later = consumed_later
                        .checked_add(i128::from(row.count_delta).abs())
                        .ok_or_else(|| invalid("item_ledger_count_overflow"))?;
                }
                capped(after, consumed_later)?;
            }
        }
        Ok(())
    }

    /// Every changed scalar needs a real ledger fact. Sidecar-only changes need no fake ID.
    #[allow(clippy::too_many_lines)] // One ordered scalar/count/identity gate for atomic settlement.
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
        let revision = self.projection.checkpoint.inventory_revision;
        let unchanged = before.is_some_and(|cp| {
            self.projection.ledger.is_empty()
                && cp.instances == self.projection.checkpoint.instances
                && cp.auto_shots == self.projection.checkpoint.auto_shots
                && cp.inventory_revision == revision
        });
        // A fresh no-op receipt can retain the revision; actual mutations take one step.
        if (!unchanged
            && before.map_or(0, |cp| cp.inventory_revision).checked_add(1) != Some(revision))
            || self
                .projection
                .receipts
                .iter()
                .any(|receipt| receipt.result.inventory_revision != revision)
            || self
                .projection
                .materializations
                .iter()
                .any(|fact| fact.inventory_revision != revision)
        {
            return Err(invalid("item_checkpoint_revision_step"));
        }
        let changed_ids: BTreeSet<_> = old
            .keys()
            .chain(new.keys())
            .chain(self.projection.ledger.iter().map(|delta| &delta.instance))
            .copied()
            .collect();
        for id in &changed_ids {
            let mut endpoint = old.get(id).copied();
            let (mut consumed, mut debited) = (false, false);
            for (position, delta) in self
                .projection
                .ledger
                .iter()
                .filter(|delta| delta.instance == *id)
                .enumerate()
            {
                if delta.before.as_ref() != endpoint {
                    return Err(invalid("item_ledger_scalar_continuity"));
                }
                // D66: positive rows first in cause order, then the single Consume aggregate as
                // the instance's last row; no credit follows a debit.
                if consumed || (debited && delta.count_delta > 0) {
                    return Err(invalid("item_ledger_row_order"));
                }
                // D76/D55: the ordinal is this row's canonical position 0..n-1 among the
                // instance's rows this tick, so it strictly increases, every row of one logical
                // operation (implicit Unequip + Destroy) takes its own ordinal and the Consume
                // aggregate holds the highest one.
                if u64::from(delta.operation_ordinal)
                    != u64::try_from(position).map_err(anyhow::Error::from)?
                {
                    return Err(invalid("item_ledger_row_ordinal"));
                }
                if delta
                    .after
                    .as_ref()
                    .is_some_and(|item| item.revision != revision)
                {
                    return Err(invalid("item_ledger_revision_step"));
                }
                let difference = i128::from(delta.after.as_ref().map_or(0, |item| item.count))
                    .checked_sub(i128::from(delta.before.as_ref().map_or(0, |item| item.count)))
                    .ok_or_else(|| invalid("item_ledger_count_overflow"))?;
                if difference != i128::from(delta.count_delta) {
                    return Err(invalid("item_ledger_count_conservation"));
                }
                if delta.operation == ItemOperation::Consume {
                    validate_consumption(delta)?;
                    consumed = true;
                }
                debited |= delta.count_delta < 0;
                endpoint = delta.after.as_ref();
            }
            if endpoint != new.get(id).copied() {
                return Err(invalid("item_ledger_final_scalar"));
            }
        }
        let mut created = BTreeSet::new();
        for delta in &self.projection.ledger {
            if delta.before.is_none()
                && (old.contains_key(&delta.instance) || !created.insert(delta.instance))
            {
                return Err(invalid("item_instance_identity_reused"));
            }
            if let Some(original) = old.get(&delta.instance) {
                if delta.before.iter().chain(delta.after.iter()).any(|item| {
                    item.created_tick != original.created_tick
                        || item.creation_origin != original.creation_origin
                }) {
                    return Err(invalid("item_instance_creation_immutable"));
                }
            }
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
        let enchant_facts: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let DomainEvent::ItemEnchanted { fact } = event {
                    Some(fact)
                } else {
                    None
                }
            })
            .collect();
        if enchant_facts.len() != self.projection.enchant_attempts.len() {
            return Err(invalid("item_enchant_outbox_count"));
        }
        for attempt in &self.projection.enchant_attempts {
            let causes = vec![serde_json::to_value(attempt).map_err(anyhow::Error::from)?];
            if enchant_facts
                .iter()
                .filter(|fact| {
                    fact.instance_id == attempt.target
                        && fact.template_id == attempt.template
                        && fact.before.as_ref() == Some(&attempt.before)
                        && fact.after == attempt.after
                        && fact.count_delta == if attempt.after.is_none() { -1 } else { 0 }
                        && fact.causes == causes
                })
                .count()
                != 1
            {
                return Err(invalid("item_enchant_outbox_attempt"));
            }
        }
        let exchange_facts: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let DomainEvent::ItemExchanged { fact } = event {
                    Some(fact)
                } else {
                    None
                }
            })
            .collect();
        if exchange_facts.len() != self.projection.exchanges.len() {
            return Err(invalid("item_exchange_outbox_count"));
        }
        for exchange in &self.projection.exchanges {
            let causes = vec![serde_json::to_value(exchange).map_err(anyhow::Error::from)?];
            if exchange_facts
                .iter()
                .filter(|fact| {
                    fact.instance_id == exchange.before.id
                        && fact.template_id == exchange.before.template_id
                        && fact.before.as_ref() == Some(&exchange.before)
                        && fact.after.is_none()
                        && fact.count_delta == -1
                        && fact.causes == causes
                })
                .count()
                != 1
            {
                return Err(invalid("item_exchange_outbox_attempt"));
            }
        }
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
                .find(|request| {
                    request.request_id == completion.request_id
                        && match request.kind {
                            PendingItemKind::Starter => ItemOperation::Starter,
                            PendingItemKind::Backfill => ItemOperation::Backfill,
                        } == completion.receipt.operation
                })
                .ok_or_else(|| invalid("pending_item_completion_missing"))?;
            let receipt = self
                .projection
                .receipts
                .iter()
                .find(|fact| fact.reference() == completion.receipt)
                .ok_or_else(|| invalid("pending_item_completion_receipt"))?;
            let expected = match pending.kind {
                PendingItemKind::Starter => ItemMutationOperation::Starter,
                PendingItemKind::Backfill => ItemMutationOperation::Backfill,
            };
            if pending.character_id.as_uuid() != self.projection.entity.as_uuid()
                || pending.account_id != self.projection.account
                || pending.operation != expected
                || pending.status != PendingItemStatus::Pending
                || pending.manifest_version != completion.manifest_version
                || pending.manifest_hash != completion.manifest_hash
                || pending.fingerprint != receipt.fingerprint
                || ItemMutationOperation::from_result(&receipt.result)? != expected
                || receipt.result.inventory_revision != completion.inventory_revision
            {
                return Err(invalid("pending_item_completion_identity"));
            }
            let PendingItemManifest::Kit(manifest) = &pending.manifest;
            {
                let kit = self
                    .projection
                    .checkpoint
                    .starter_kit_receipt
                    .as_ref()
                    .ok_or_else(|| invalid("pending_kit_receipt"))?;
                if kit.request_id != completion.receipt.key
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
                        receipt_refs: delta.receipt_refs.clone(),
                        receipt: delta.receipt,
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
                    receipt_ref: delta.receipt.map(|receipt| receipt.key),
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
                    let ordinary_capacity = self.projection.checkpoint.ordinary_capacity;
                    let quest_capacity = self.projection.checkpoint.quest_capacity;
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
/// D68 pinned stack maxima. L2ItemInstance.java:422 caps adena (template 57) at `MaxAdena`
/// (character.properties:671 = 99,900,000,000) and every other stack at `Integer.MAX_VALUE`;
/// non-stackable templates hold exactly one instance.
fn pinned_count_cap(template_id: u32, stackable: bool) -> i128 {
    const ADENA_TEMPLATE: u32 = 57;
    const MAX_ADENA: i128 = 99_900_000_000;
    match (stackable, template_id) {
        (false, _) => 1,
        (true, ADENA_TEMPLATE) => MAX_ADENA,
        (true, _) => i128::from(i32::MAX),
    }
}

fn invalid(name: &str) -> CheckpointError {
    CheckpointError::Constraint(name.into())
}

fn validate_consumption(
    delta: &crate::domain::zone::phase4_projection::ItemDeltaPlan,
) -> Result<(), CheckpointError> {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue;
    let mut total = 0_i128;
    let mut ordinal = None;
    let mut references = Vec::new();
    for cause in &delta.causes {
        let Some(ItemEvidenceValue::Signed(debit)) = cause.fields.get("count_delta") else {
            return Err(invalid("item_consume_contribution"));
        };
        if *debit >= 0 {
            return Err(invalid("item_consume_contribution"));
        }
        let receipt = cause
            .contributing_receipt()
            .map_err(|_| invalid("item_consume_cause_identity"))?;
        if let Some(receipt) = receipt {
            if !references.contains(&receipt) {
                references.push(receipt);
            }
        } else if cause.family != "skill_cast_payment"
            || !matches!(cause.fields.get("cast_id"), Some(ItemEvidenceValue::Unsigned(id)) if *id > 0)
        {
            return Err(invalid("item_consume_cause_identity"));
        }
        if let Some(ItemEvidenceValue::Unsigned(current)) = cause.fields.get("operation_ordinal") {
            if ordinal.is_some_and(|previous| *current < previous) {
                return Err(invalid("item_consume_cause_order"));
            }
            ordinal = Some(*current);
        }
        total = total
            .checked_add(i128::from(*debit))
            .ok_or_else(|| invalid("item_ledger_count_overflow"))?;
    }
    if total != i128::from(delta.count_delta) || total >= 0 {
        return Err(invalid("item_consume_contribution"));
    }
    let expected = if delta.receipt_refs.is_empty() {
        delta.receipt.iter().copied().collect::<Vec<_>>()
    } else {
        delta.receipt_refs.clone()
    };
    if references != expected {
        return Err(invalid("item_consume_receipt_order"));
    }
    Ok(())
}
