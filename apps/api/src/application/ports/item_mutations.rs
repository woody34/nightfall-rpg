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
        self.validate_refines(rules)?;
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

    /// D95: each captured refine audit is re-derived at the trust boundary. A Refine's three
    /// retail draws reproduce its selection (and so its packed augment) from the hashed tables
    /// (raw-word/selection consistency only; that the words came from the RNG stream is
    /// replay's job);
    /// a `CancelRefine` has no draw and pays exactly the hashed Adena fee. The audit's payments
    /// equal, per template, the Consume causes of its family carrying its receipt, and no such
    /// cause is left unaccounted.
    fn validate_refines(
        &self,
        rules: &crate::domain::zone::Phase4Rules,
    ) -> Result<(), CheckpointError> {
        use crate::domain::zone::item_refine::{cancel_fee, RetailSelectors, ADENA};
        use crate::domain::zone::phase4_projection::ItemEvidenceValue;
        // Every Refine/CancelRefine target row is backed by its captured audit: the outbox
        // publishes the audit in place of the row, so a row without one would go unaudited.
        if self.projection.ledger.iter().any(|delta| {
            matches!(delta.operation, ItemOperation::Refine | ItemOperation::CancelRefine)
                && !self.projection.refines.iter().any(|refine| {
                    delta.receipt == Some(refine.receipt) && delta.instance == refine.after.id
                })
        }) {
            return Err(invalid("item_refine_audit_missing"));
        }
        for refine in &self.projection.refines {
            let applied = refine.outcome == ItemRefineOutcome::Applied;
            match (&refine.selection, refine.stone) {
                (Some(selection), Some(stone)) if applied => {
                    let selectors =
                        RetailSelectors::new(rules, refine.accessory, refine.mage, stone)
                            .map_err(|_| invalid("item_refine_draws"))?;
                    let mut words = [
                        selection.first_raw,
                        selection.category_raw,
                        selection.second_raw,
                    ]
                    .into_iter();
                    let mut drawn = 0_u8;
                    let recomputed = selectors
                        .select(|| {
                            drawn = drawn.saturating_add(1);
                            words.next().unwrap_or(u32::MAX)
                        })
                        .map_err(|_| invalid("item_refine_draws"))?;
                    if drawn != 3
                        || &recomputed != selection
                        || refine.payments.first().map(|payment| payment.template_id) != Some(stone)
                    {
                        return Err(invalid("item_refine_draws"));
                    }
                },
                (None, None) if !applied => {
                    let definition = rules
                        .parts()
                        .items
                        .get(&refine.before.template_id)
                        .ok_or_else(|| invalid("item_refine_cancel_fee"))?;
                    let fee = cancel_fee(rules, definition, refine.before.enchant)
                        .map_err(|_| invalid("item_refine_cancel_fee"))?;
                    if refine.payments.len() != 1
                        || refine.payments.first().map(|p| (p.template_id, p.count))
                            != Some((ADENA, fee))
                    {
                        return Err(invalid("item_refine_cancel_fee"));
                    }
                },
                _ => return Err(invalid("item_refine_draws")),
            }
            let family = if applied {
                "refine_payment"
            } else {
                "cancel_refine_payment"
            };
            let key = refine.receipt.key.to_string();
            let mut paid = BTreeMap::<u32, i128>::new();
            for payment in &refine.payments {
                let total = paid.entry(payment.template_id).or_insert(0);
                *total = total
                    .checked_add(i128::from(payment.count))
                    .ok_or_else(|| invalid("item_refine_payment"))?;
            }
            let mut consumed = BTreeMap::<u32, i128>::new();
            for delta in self
                .projection
                .ledger
                .iter()
                .filter(|delta| delta.operation == ItemOperation::Consume)
            {
                for cause in delta.causes.iter().filter(|cause| {
                    cause.family == family
                        && cause.fields.get("receipt")
                            == Some(&ItemEvidenceValue::Text(key.clone()))
                }) {
                    let Some(ItemEvidenceValue::Signed(debit)) = cause.fields.get("count_delta")
                    else {
                        return Err(invalid("item_refine_payment"));
                    };
                    let template = delta
                        .template()
                        .map_err(|_| invalid("item_refine_payment"))?;
                    let total = consumed.entry(template).or_insert(0);
                    *total = total
                        .checked_sub(i128::from(*debit))
                        .ok_or_else(|| invalid("item_refine_payment"))?;
                }
            }
            if paid != consumed {
                return Err(invalid("item_refine_payment"));
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
    pub fn validate_before(
        &self,
        before: Option<&Phase4Checkpoint>,
    ) -> Result<(), CheckpointError> {
        self.validate_before_with(before, None)
    }

    /// As [`Self::validate_before`], with the bound item rules (D127: an `insufficient`
    /// automatic-shot drop is checked against the equipped weapon's per-shot requirement;
    /// without rules it fails closed).
    #[allow(clippy::too_many_lines)] // One ordered scalar/count/identity gate for atomic settlement.
    pub fn validate_before_with(
        &self,
        before: Option<&Phase4Checkpoint>,
        rules: Option<&crate::domain::zone::Phase4Rules>,
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
        if let Some(before) = before {
            validate_auto_shots(before, &self.projection, rules)?;
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
        // D137/D143: a no-op receipt of any kind (nothing affected, no ledger row under its
        // key) keeps the un-stepped revision even when a later same-tick mutation steps the
        // owner; every mutating receipt carries the settled revision.
        let pre_tick = before.map(|cp| cp.inventory_revision);
        if (!unchanged
            && before.map_or(0, |cp| cp.inventory_revision).checked_add(1) != Some(revision))
            || self.projection.receipts.iter().any(|receipt| {
                let no_op = matches!(
                    &receipt.result.detail,
                    ItemResultDetail::Operation { affected, .. } if affected.is_empty()
                ) && !self.projection.ledger.iter().any(|delta| {
                    delta.receipt.is_some_and(|r| r.key == receipt.request_id)
                        || delta
                            .receipt_refs
                            .iter()
                            .any(|r| r.key == receipt.request_id)
                });
                receipt.result.inventory_revision != revision
                    && !(no_op && Some(receipt.result.inventory_revision) == pre_tick)
            })
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
            let (mut consumed, mut ended) = (false, false);
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
                // D66/D94: [non-terminal rows in cause order..., the single Consume aggregate, at
                // most one terminal row (after = None: whole transfer-out, Destroy, Crystallize)];
                // nothing follows a terminal row. D100: non-terminal rows keep D69 cause order,
                // so a credit may follow a non-Consume debit (partial Destroy, then a crystal
                // refund); continuity, conservation and the D74 caps still bind every row.
                let terminal = delta.operation != ItemOperation::Consume && delta.after.is_none();
                if ended || (consumed && !terminal) {
                    return Err(invalid("item_ledger_row_order"));
                }
                ended = terminal;
                // D76/D55: the ordinal is this row's canonical position 0..n-1 among the
                // instance's rows this tick, so it strictly increases and every row of one
                // logical operation (implicit Unequip + Destroy) takes its own ordinal; under D94
                // the Consume aggregate holds n-1, or n-2 when a terminal row follows it.
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
                    validate_consumption(delta, rules)?;
                    consumed = true;
                }
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

    /// D95: exactly one captured fact per Refine (`ItemRefined`) / `CancelRefine`
    /// (`ItemRefineCancelled`), carrying the audit as its single cause.
    fn validate_refine_events(&self, events: &[DomainEvent]) -> Result<(), CheckpointError> {
        let refine_facts: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let DomainEvent::ItemRefined { fact } = event {
                    Some((ItemRefineOutcome::Applied, fact))
                } else if let DomainEvent::ItemRefineCancelled { fact } = event {
                    Some((ItemRefineOutcome::Removed, fact))
                } else {
                    None
                }
            })
            .collect();
        if refine_facts.len() != self.projection.refines.len() {
            return Err(invalid("item_refine_outbox_count"));
        }
        for refine in &self.projection.refines {
            let causes = vec![serde_json::to_value(refine).map_err(anyhow::Error::from)?];
            if refine_facts
                .iter()
                .filter(|(outcome, fact)| {
                    *outcome == refine.outcome
                        && fact.instance_id == refine.after.id
                        && fact.template_id == refine.before.template_id
                        && fact.before.as_ref() == Some(&refine.before)
                        && fact.after.as_ref() == Some(&refine.after)
                        && fact.count_delta == 0
                        && fact.causes == causes
                })
                .count()
                != 1
            {
                return Err(invalid("item_refine_outbox_attempt"));
            }
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
        self.validate_refine_events(events)?;
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
                | DomainEvent::ItemRefineCancelled { fact }
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

/// D47/E3.4: a learned skill rank that requires items consumes them in the learning tick.
/// The payment has no item receipt; it is bound to the unified learn receipt instead.
const LEARN_PAYMENT: &str = "skill_learn_payment";

/// `(learn request key, skill id, skill level)` of a learn payment cause.
fn learn_payment(
    cause: &crate::domain::zone::phase4_projection::ItemAuditEvidence,
) -> Option<(uuid::Uuid, u64, u64)> {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue;
    let ItemEvidenceValue::Text(key) = cause.fields.get("receipt")? else {
        return None;
    };
    let key = key.parse::<uuid::Uuid>().ok().filter(|key| !key.is_nil())?;
    let (Some(ItemEvidenceValue::Unsigned(id)), Some(ItemEvidenceValue::Unsigned(level))) =
        (cause.fields.get("skill_id"), cause.fields.get("skill_level"))
    else {
        return None;
    };
    (*id > 0 && *level > 0).then_some((key, *id, *level))
}

/// D47/E3.4: every learn payment belongs to the learn receipt settled in the same
/// transaction, names its learned rank, and pays exactly that rank's required items (per
/// template, in the owner's current class tree); a learn receipt whose rank requires items
/// settles only together with exactly those payments (`ledger` empty when no item work).
pub fn validate_learn_payments(
    ledger: &[crate::domain::zone::phase4_projection::ItemDeltaPlan],
    cp: &CharacterCheckpoint,
    learning: Option<&super::SuccessfulLearnReceipt>,
    classes: &crate::domain::class::ClassRegistry,
) -> Result<(), CheckpointError> {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue;
    let mut paid = BTreeMap::<u32, i128>::new();
    for delta in ledger
        .iter()
        .filter(|delta| delta.operation == ItemOperation::Consume)
    {
        for cause in delta
            .causes
            .iter()
            .filter(|cause| cause.family == LEARN_PAYMENT)
        {
            let (key, id, level) =
                learn_payment(cause).ok_or_else(|| invalid("item_learn_payment"))?;
            let receipt = learning.ok_or_else(|| invalid("item_learn_payment_receipt"))?;
            if key != receipt.key.as_uuid()
                || id != u64::from(receipt.result.skill.id.0)
                || level != u64::from(receipt.result.skill.rank)
            {
                return Err(invalid("item_learn_payment_receipt"));
            }
            let Some(ItemEvidenceValue::Signed(debit)) = cause.fields.get("count_delta") else {
                return Err(invalid("item_learn_payment"));
            };
            let template = delta
                .template()
                .map_err(|_| invalid("item_learn_payment"))?;
            let total = paid.entry(template).or_insert(0);
            *total = total
                .checked_sub(i128::from(*debit))
                .ok_or_else(|| invalid("item_learn_payment"))?;
        }
    }
    let Some(receipt) = learning else {
        return Ok(());
    };
    let class = cp
        .class_state
        .as_ref()
        .ok_or_else(|| invalid("item_learn_payment"))?
        .current_class_id;
    let skill = receipt.result.skill;
    let row = classes
        .skill_tree(class)
        .map_err(|_| invalid("item_learn_payment"))?
        .into_iter()
        .find(|row| row.skill_id == skill.id.0 && row.skill_level == skill.rank)
        .ok_or_else(|| invalid("item_learn_payment"))?;
    let mut required = BTreeMap::<u32, i128>::new();
    for item in &row.required_items {
        let template = item
            .item
            .strip_prefix("l2.item.")
            .and_then(|id| id.parse::<u32>().ok())
            .ok_or_else(|| invalid("item_learn_payment"))?;
        let total = required.entry(template).or_insert(0);
        *total = total
            .checked_add(i128::from(item.count))
            .ok_or_else(|| invalid("item_learn_payment"))?;
    }
    if paid != required {
        return Err(invalid("item_learn_payment"));
    }
    Ok(())
}

/// D127 (amended) + D128 + D130: the per-template account of an automatic-shot set change.
/// Every changed template is either covered by a `ToggleAutoShot` receipt or is a removal
/// evidenced by exactly one `auto_shot_exhausted` projection entry:
/// - `exhausted`: remaining 0, no instance left, and this tick's Consume row of `instance`
///   (that template, `after` None) carries the `auto_shot_payment` cause with the same id;
/// - `missing`: remaining 0 and no instance of the template (D136: before or after the tick);
/// - `incarnation` (D142): the admission reset; no toggles and no charges remain settled;
/// - `insufficient`: `instance` is the template's only stack with count `remaining` (D136: in
///   the pre-tick or the settled checkpoint, with the weapon of that same checkpoint);
///   `required` is the wielded (`weapon`) right-hand weapon's unreduced per-shot count from the
///   catalogue; `remaining < required`, and below the reduced count when this resolution's
///   `soulshot_reduction` draw reduced it. Unknown (no rules, no weapon) fails closed.
///
/// Spurious, duplicate or malformed entries are refused (`item_auto_shot_evidence`); an
/// unaccounted change needs the receipt (`item_toggle_receipt_required`).
#[allow(clippy::too_many_lines)] // One entry match per D127/D130/D136/D142 reason keeps the account auditable.
fn validate_auto_shots(
    before: &Phase4Checkpoint,
    projection: &Phase4Projection,
    rules: Option<&crate::domain::zone::Phase4Rules>,
) -> Result<(), CheckpointError> {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue as V;
    let after = &projection.checkpoint;
    let removed: BTreeSet<u32> = before
        .auto_shots
        .difference(&after.auto_shots)
        .copied()
        .collect();
    let added = after.auto_shots.difference(&before.auto_shots).count();
    let bad = || invalid("item_auto_shot_evidence");
    let text = |fields: &BTreeMap<String, V>, key: &str| match fields.get(key) {
        Some(V::Text(value)) => Some(value.clone()),
        _ => None,
    };
    let unsigned = |fields: &BTreeMap<String, V>, key: &str| match fields.get(key) {
        Some(V::Unsigned(value)) => Some(*value),
        _ => None,
    };
    let mut evidenced = BTreeSet::new();
    for entry in projection
        .evidence
        .iter()
        .filter(|entry| entry.family == "auto_shot_exhausted")
    {
        let fields = &entry.fields;
        let template = unsigned(fields, "template")
            .and_then(|t| u32::try_from(t).ok())
            .ok_or_else(bad)?;
        let remaining = unsigned(fields, "remaining").ok_or_else(bad)?;
        let reason = text(fields, "reason").ok_or_else(bad)?;
        let resolution = text(fields, "resolution_id").ok_or_else(bad)?;
        if !removed.contains(&template) || !evidenced.insert(template) || resolution.is_empty() {
            return Err(bad());
        }
        let stacks: Vec<_> = after
            .instances
            .iter()
            .filter(|item| item.template_id == template)
            .collect();
        let instance = unsigned(fields, "instance");
        let ok = match reason.as_str() {
            "exhausted" => {
                remaining == 0
                    && stacks.is_empty()
                    && instance.is_some_and(|instance| {
                        projection.ledger.iter().any(|row| {
                            row.operation == ItemOperation::Consume
                                && row.instance.get() == instance
                                && row.after.is_none()
                                && row.before.as_ref().map(|item| item.template_id)
                                    == Some(template)
                                && row.causes.iter().any(|cause| {
                                    cause.family == "auto_shot_payment"
                                        && text(&cause.fields, "resolution_id").as_deref()
                                            == Some(resolution.as_str())
                                })
                        })
                    })
            },
            // D136: a retained entry is as-of its removal. Its stack and weapon facts must hold
            // in the pre-tick checkpoint or in the settled one (later same-tick commands can
            // change the stack or the wielded weapon after the removal).
            "missing" => {
                remaining == 0
                    && instance.is_none()
                    && [before, after]
                        .iter()
                        .any(|cp| cp.instances.iter().all(|item| item.template_id != template))
            },
            "insufficient" => {
                remaining > 0
                    && [before, after].iter().any(|cp| {
                        let stacks: Vec<_> = cp
                            .instances
                            .iter()
                            .filter(|item| item.template_id == template)
                            .collect();
                        matches!(stacks.as_slice(), [stack] if stack.count == remaining
                            && instance == Some(stack.id.get()))
                            && insufficient(
                                cp,
                                projection,
                                rules,
                                template,
                                remaining,
                                unsigned(fields, "required"),
                                unsigned(fields, "weapon"),
                                &resolution,
                            )
                    })
            },
            // D142: a new incarnation drops every automatic shot and every weapon charge (D71).
            // Accepted only when the settled state is that reset: no toggles and no charges left;
            // `remaining` is the template's settled count.
            "incarnation" => {
                instance.is_none()
                    && after.auto_shots.is_empty()
                    && after.instances.iter().all(|item| item.charges == 0)
                    && after
                        .instances
                        .iter()
                        .filter(|item| item.template_id == template)
                        .try_fold(0_u64, |total, item| total.checked_add(item.count))
                        == Some(remaining)
            },
            _ => false,
        };
        if !ok {
            return Err(bad());
        }
    }
    // D135: one toggle receipt explains any one unexplained change (Phase 5: per-target).
    let toggles = projection
        .receipts
        .iter()
        .filter(|receipt| {
            matches!(
                receipt.result.detail,
                ItemResultDetail::Operation {
                    operation: ItemOperation::ToggleAutoShot,
                    ..
                }
            )
        })
        .count();
    let unaccounted = removed
        .len()
        .checked_sub(evidenced.len())
        .and_then(|left| left.checked_add(added))
        .ok_or_else(bad)?;
    if unaccounted > toggles {
        return Err(invalid("item_toggle_receipt_required"));
    }
    Ok(())
}

/// D128 `insufficient`: `required` must be the wielded right-hand weapon's unreduced per-shot
/// count for this shot kind (catalogue), `weapon` must name that weapon (instance or template),
/// and the remaining stack must be below the count the charge actually needed: the reduced
/// count when this resolution's `soulshot_reduction` residue fell under the reduction chance.
#[allow(clippy::too_many_arguments)] // One entry's cross-checks against checkpoint, rules and draw.
fn insufficient(
    after: &Phase4Checkpoint,
    projection: &Phase4Projection,
    rules: Option<&crate::domain::zone::Phase4Rules>,
    template: u32,
    remaining: u64,
    required: Option<u64>,
    weapon: Option<u64>,
    resolution: &str,
) -> bool {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue as V;
    let (Some(rules), Some(required), Some(weapon)) = (rules, required, weapon) else {
        return false;
    };
    let items = &rules.parts().items;
    let Some(wielded) = after
        .paperdoll
        .get(&crate::domain::zone::EquipSlot::Rhand)
        .and_then(|id| after.instances.iter().find(|item| item.id == *id))
    else {
        return false;
    };
    if weapon != wielded.id.get() && weapon != u64::from(wielded.template_id) {
        return false;
    }
    let (Some(shot), Some(definition)) = (items.get(&template), items.get(&wielded.template_id))
    else {
        return false;
    };
    let soul = shot.handler == "SoulShots";
    let full = match shot.handler.as_str() {
        "SoulShots" => definition.soulshots,
        "SpiritShot" | "BlessedSpiritShot" => definition.spiritshots,
        _ => return false,
    };
    if full == 0 || required != u64::from(full) {
        return false;
    }
    let draw = projection.evidence.iter().find(|entry| {
        entry.family == "soulshot_reduction"
            && matches!(entry.fields.get("resolution_id"), Some(V::Text(id)) if id == resolution)
    });
    let needed = match (soul, definition.reduced_soulshot.as_slice(), draw) {
        (true, [chance, reduced], Some(draw)) if *reduced > 0 => match draw.fields.get("residue") {
            Some(V::Unsigned(residue)) if *residue < u64::from(*chance) => u64::from(*reduced),
            Some(V::Unsigned(_)) => u64::from(full),
            _ => return false,
        },
        // A reducing weapon always draws; a missing draw is not this resolution.
        (true, [_, reduced], None) if *reduced > 0 => return false,
        (_, _, None) => u64::from(full),
        (_, _, Some(_)) => return false,
    };
    remaining < needed
}

/// T26/D126, tightened by D129: receipt-less ammunition debits of an attack. Their identity is
/// the zone's attack resolution id, as the evidence exporter reads it (the receipt branch is
/// taken before this check, so the `receipt` test is defensive only).
/// - `arrow_payment`: exactly one arrow (`count_delta` -1), a `weapon` template, and the id
///   `attack:<attacker>:<tick>`;
/// - `auto_shot_payment`: the id `attack:…` or `cast:…` ending in `:shot:<debited template>`;
/// - with the bound item rules, the debited template must be ammunition (ARROW/BOLT) or a shot.
fn attack_payment(
    cause: &crate::domain::zone::phase4_projection::ItemAuditEvidence,
    delta: &crate::domain::zone::phase4_projection::ItemDeltaPlan,
    rules: Option<&crate::domain::zone::Phase4Rules>,
) -> bool {
    use crate::domain::zone::phase4_projection::ItemEvidenceValue;
    let Some(ItemEvidenceValue::Text(id)) = cause.fields.get("resolution_id") else {
        return false;
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) || cause.fields.contains_key("receipt")
    {
        return false;
    }
    let Some(template) = delta.before.as_ref().map(|item| item.template_id) else {
        return false;
    };
    let definition = rules.map(|rules| rules.parts().items.get(&template));
    match cause.family.as_str() {
        "arrow_payment" => {
            let mut parts = id.split(':');
            let shape = parts.next() == Some("attack")
                && parts.next().is_some_and(|attacker| !attacker.is_empty())
                && parts.next().is_some_and(|tick| {
                    !tick.is_empty() && tick.bytes().all(|b| b.is_ascii_digit())
                })
                && parts.next().is_none();
            shape
                && matches!(cause.fields.get("count_delta"), Some(ItemEvidenceValue::Signed(-1)))
                && matches!(cause.fields.get("weapon"), Some(ItemEvidenceValue::Unsigned(w)) if *w > 0)
                && definition.is_none_or(|definition| {
                    definition.is_some_and(|d| matches!(d.etcitem_type.as_str(), "ARROW" | "BOLT"))
                })
        },
        "auto_shot_payment" => {
            (id.starts_with("attack:") || id.starts_with("cast:"))
                && id.ends_with(&format!(":shot:{template}"))
                && definition.is_none_or(|definition| {
                    definition.is_some_and(|d| {
                        matches!(
                            d.handler.as_str(),
                            "SoulShots" | "SpiritShot" | "BlessedSpiritShot"
                        )
                    })
                })
        },
        _ => false,
    }
}

fn validate_consumption(
    delta: &crate::domain::zone::phase4_projection::ItemDeltaPlan,
    rules: Option<&crate::domain::zone::Phase4Rules>,
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
        } else if !(cause.family == "skill_cast_payment"
            && matches!(cause.fields.get("cast_id"), Some(ItemEvidenceValue::Unsigned(id)) if *id > 0)
            || cause.family == LEARN_PAYMENT && learn_payment(cause).is_some()
            || attack_payment(cause, delta, rules))
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod learn_payment_tests {
    use super::*;
    use crate::application::ports::SuccessfulLearnReceipt;
    use crate::domain::character_progression::ClassState;
    use crate::domain::class::ClassId;
    use crate::domain::zone::phase4_projection::{
        ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue,
    };
    use crate::domain::zone::skill::{SkillId, SkillRef};
    use crate::domain::zone::{
        FrozenLearnResult, ItemInstance, ItemInstanceId, ItemLocation, LearnedEffectiveStats, Tick,
    };

    fn classes() -> std::sync::Arc<crate::domain::class::ClassRegistry> {
        crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap()
        .registry
    }
    /// A Rogue (7) learning skill 1405 rank `rank`; rank 1 requires one book 8618.
    fn learner(rank: u32) -> (CharacterCheckpoint, SuccessfulLearnReceipt) {
        let character = CharacterId::from_uuid(uuid::Uuid::from_u128(5));
        let ledger = ClassState {
            base_class_id: ClassId(0),
            current_class_id: ClassId(7),
            sp: 0,
            learned_skills: Vec::new(),
            cp: 0,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            milestone_claimed_mask: 0,
            successful_transfer_receipts: Vec::new(),
        };
        let cp = CharacterCheckpoint {
            character_id: character,
            revision_seen: 1,
            level: 40,
            xp: 0,
            hp: 1,
            mp: 1,
            alive: true,
            position: crate::domain::Position { x: 0.0, y: 0.0 },
            idempotency: ("checkpoint".into(), IdempotencyKey::new()),
            class_state: Some(ledger),
            phase4: None,
            phase3: None,
        };
        let skill = SkillRef {
            id: SkillId(1405),
            rank,
        };
        let receipt = SuccessfulLearnReceipt {
            account_id: AccountId::from_uuid(uuid::Uuid::from_u128(6)),
            key: IdempotencyKey::new(),
            request_fingerprint: String::new(),
            result: FrozenLearnResult {
                character_id: character,
                skill,
                sp_spent: 0,
                sp_remaining: 0,
                tick: Tick(9),
                hp: 1,
                max_hp: 1,
                mp: 1,
                max_mp: 1,
                cp: 0,
                max_cp: 0,
                effective_stats: LearnedEffectiveStats {
                    p_atk_q: 0,
                    p_def_q: 0,
                    m_atk_q: 0,
                    m_def_q: 0,
                    attack_speed: 0,
                    cast_speed: 0,
                    speed_q: 0,
                    accuracy_q: 0,
                    evasion_q: 0,
                    crit_permille: 0,
                },
                incarnation: 1,
            },
        };
        (cp, receipt)
    }
    fn book(template: u32, count: u64) -> ItemInstance {
        ItemInstance {
            creation_origin: None,
            id: ItemInstanceId::new(1 << 33).unwrap(),
            template_id: template,
            count,
            location: ItemLocation::Inventory,
            slot: None,
            enchant: 0,
            augment_id: None,
            charges: 0,
            created_tick: Tick(1),
            revision: 2,
        }
    }
    fn payment(receipt: &SuccessfulLearnReceipt, template: u32, family: &str) -> ItemDeltaPlan {
        ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: ItemInstanceId::new(1 << 33).unwrap(),
            before: Some(book(template, 1)),
            after: None,
            count_delta: -1,
            receipt: None,
            receipt_refs: Vec::new(),
            causes: vec![ItemAuditEvidence {
                family: family.into(),
                fields: BTreeMap::from([
                    ("receipt".into(), ItemEvidenceValue::Text(receipt.key.to_string())),
                    (
                        "skill_id".into(),
                        ItemEvidenceValue::Unsigned(u64::from(receipt.result.skill.id.0)),
                    ),
                    (
                        "skill_level".into(),
                        ItemEvidenceValue::Unsigned(u64::from(receipt.result.skill.rank)),
                    ),
                    ("count_delta".into(), ItemEvidenceValue::Signed(-1)),
                ]),
            }],
        }
    }

    #[test]
    fn book_learning_pays_exactly_the_ranks_required_items_under_its_receipt() {
        let classes = classes();
        let (cp, receipt) = learner(1);
        let paid = [payment(&receipt, 8618, LEARN_PAYMENT)];
        validate_consumption(&paid[0], None).unwrap();
        validate_learn_payments(&paid, &cp, Some(&receipt), &classes).unwrap();
        // Unpaid, wrong book, or a payment without (or for another) learn receipt.
        let refused = |ledger: &[ItemDeltaPlan], learning: Option<&SuccessfulLearnReceipt>| {
            validate_learn_payments(ledger, &cp, learning, &classes).unwrap_err()
        };
        assert!(
            matches!(refused(&[], Some(&receipt)), CheckpointError::Constraint(c) if c == "item_learn_payment")
        );
        assert!(
            matches!(refused(&[payment(&receipt, 8619, LEARN_PAYMENT)], Some(&receipt)), CheckpointError::Constraint(c) if c == "item_learn_payment")
        );
        assert!(
            matches!(refused(&paid, None), CheckpointError::Constraint(c) if c == "item_learn_payment_receipt")
        );
        let (_, other) = learner(1);
        assert!(
            matches!(refused(&paid, Some(&other)), CheckpointError::Constraint(c) if c == "item_learn_payment_receipt")
        );
        // Rank 2 needs 8619, not the rank-1 book.
        let (_, rank2) = learner(2);
        let mut wrong_rank = payment(&rank2, 8618, LEARN_PAYMENT);
        validate_consumption(&wrong_rank, None).unwrap();
        assert!(validate_learn_payments(
            std::slice::from_ref(&wrong_rank),
            &cp,
            Some(&rank2),
            &classes
        )
        .is_err());
        wrong_rank.before = Some(book(8619, 1));
        validate_learn_payments(&[wrong_rank], &cp, Some(&rank2), &classes).unwrap();
        // An unknown family without an item receipt keeps the identity refusal.
        assert!(matches!(
            validate_consumption(&payment(&receipt, 8618, "skill_learn"), None),
            Err(CheckpointError::Constraint(c)) if c == "item_consume_cause_identity"
        ));
    }

    #[test]
    fn learning_without_required_items_needs_no_payment() {
        let classes = classes();
        let (mut cp, mut receipt) = learner(1);
        // Rogue skill 3 (Power Strike) has no required items in the tree.
        receipt.result.skill = SkillRef {
            id: SkillId(3),
            rank: 1,
        };
        validate_learn_payments(&[], &cp, Some(&receipt), &classes).unwrap();
        validate_learn_payments(&[], &cp, None, &classes).unwrap();
        cp.class_state = None;
        assert!(validate_learn_payments(&[], &cp, Some(&receipt), &classes).is_err());
    }
}

/// D126 as tightened by D129, and the D127/D128/D130 automatic-shot account (API session 21).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod attack_settlement_tests {
    use super::*;
    use crate::domain::zone::phase4_projection::{
        FrozenItemResult, ItemAuditEvidence, ItemDeltaPlan, ItemEvidenceValue as V,
        ItemReceiptFact, ItemReceiptRef,
    };
    use crate::domain::zone::{
        EquipSlot, ItemInstance, ItemInstanceId, ItemLocation, Phase4Checkpoint, Phase4Rules, Tick,
    };

    fn rules() -> std::sync::Arc<Phase4Rules> {
        crate::infrastructure::item_data::embedded_phase4_bundle()
            .unwrap()
            .items
    }

    fn template(
        rules: &Phase4Rules,
        keep: impl Fn(&crate::domain::zone::item_rules::ItemDefinition) -> bool,
    ) -> u32 {
        rules
            .parts()
            .items
            .iter()
            .find(|(_, d)| keep(d))
            .map(|(t, _)| *t)
            .unwrap()
    }

    fn item(id: u64, template_id: u32, count: u64) -> ItemInstance {
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
            created_tick: Tick(1),
            revision: 1,
        }
    }

    fn debit(family: &str, template: u32, fields: &[(&str, V)], count_delta: i64) -> ItemDeltaPlan {
        let mut fields: BTreeMap<String, V> = fields
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        fields
            .entry("count_delta".into())
            .or_insert(V::Signed(count_delta));
        ItemDeltaPlan {
            operation: ItemOperation::Consume,
            operation_ordinal: 0,
            instance: ItemInstanceId::new(1 << 33).unwrap(),
            before: Some(item(1 << 33, template, 5)),
            after: Some(item(1 << 33, template, 5_u64.checked_add_signed(count_delta).unwrap())),
            count_delta,
            receipt: None,
            receipt_refs: Vec::new(),
            causes: vec![ItemAuditEvidence {
                family: family.into(),
                fields,
            }],
        }
    }

    #[test]
    fn ammunition_debits_need_the_d129_identity_and_ammunition() {
        let rules = rules();
        let arrow = template(&rules, |d| d.etcitem_type == "ARROW");
        let soul = template(&rules, |d| d.handler == "SoulShots");
        let sword = template(&rules, |d| d.soulshots > 0);
        let id = |text: &str| ("resolution_id", V::Text(text.into()));
        let weapon = ("weapon", V::Unsigned(u64::from(sword)));
        let ok = |delta: &ItemDeltaPlan| validate_consumption(delta, Some(&rules)).is_ok();
        let arrow_row = |fields: &[(&str, V)], count| debit("arrow_payment", arrow, fields, count);
        assert!(ok(&arrow_row(&[id("attack:e1:40"), weapon.clone()], -1)));
        // Not one arrow, no weapon, malformed attack ids, or not ammunition.
        assert!(!ok(&arrow_row(&[id("attack:e1:40"), weapon.clone()], -2)));
        assert!(!ok(&arrow_row(&[id("attack:e1:40")], -1)));
        for bad in [
            "attack:e1",
            "attack:e1:4x",
            "cast:3",
            "attack::40",
            "attack:e1:40:9",
            "attack:e 1:40",
        ] {
            assert!(!ok(&arrow_row(&[id(bad), weapon.clone()], -1)), "{bad}");
        }
        assert!(!ok(&debit("arrow_payment", soul, &[id("attack:e1:40"), weapon.clone()], -1)));
        // Automatic shots: attack/cast ids naming the debited template.
        let shot = |text: String, t| debit("auto_shot_payment", t, &[id(&text)], -2);
        assert!(ok(&shot(format!("attack:e1:40:shot:{soul}"), soul)));
        assert!(ok(&shot(format!("cast:3:shot:{soul}"), soul)));
        assert!(!ok(&shot(format!("use:3:shot:{soul}"), soul)));
        assert!(!ok(&shot(format!("attack:e1:40:shot:{arrow}"), soul)));
        assert!(!ok(&shot(format!("attack:e1:40:shot:{arrow}"), arrow)));
        // Without the bound rules the identity checks still apply.
        assert!(validate_consumption(&shot(format!("cast:3:shot:{soul}"), soul), None).is_ok());
        // Contributions must sum to the aggregate; a row receipt is never contributed.
        let mut unbalanced = arrow_row(&[id("attack:e1:40"), weapon.clone()], -1);
        unbalanced.count_delta = -2;
        assert!(!ok(&unbalanced));
        let mut receipted = arrow_row(&[id("attack:e1:40"), weapon], -1);
        receipted.receipt = Some(ItemReceiptRef {
            operation: ItemOperation::Use,
            key: uuid::Uuid::from_u128(7),
        });
        assert!(!ok(&receipted));
    }

    fn checkpoint(instances: Vec<ItemInstance>, shots: &[u32]) -> Phase4Checkpoint {
        let paperdoll = instances
            .iter()
            .filter(|i| i.slot.is_some())
            .map(|i| (EquipSlot::Rhand, i.id))
            .collect();
        Phase4Checkpoint {
            schema_version: 1,
            timer_policy_version: 1,
            rules_hash: [0; 32],
            inventory_revision: 1,
            instances,
            paperdoll,
            auto_shots: shots.iter().copied().collect(),
            item_reuse_remainders: Vec::new(),
            equipment_install_order: Vec::new(),
            active_sets: BTreeMap::new(),
            weight: 0,
            max_load: 100_000,
            ordinary_capacity: 80,
            quest_capacity: 100,
            weight_penalty_bonus: 1,
            overload_level: 0,
            overloaded: false,
            expertise: 0,
            weapon_penalty: 0,
            armor_penalty: 0,
            starter_kit_receipt: None,
        }
    }

    fn projection(after: Phase4Checkpoint, evidence: Vec<ItemAuditEvidence>) -> Phase4Projection {
        Phase4Projection {
            exchanges: Vec::new(),
            refines: Vec::new(),
            enchant_attempts: Vec::new(),
            paused_core: None,
            tick: Tick(2),
            entity: crate::domain::zone::EntityId::from_uuid(uuid::Uuid::from_u128(3)),
            account: crate::domain::AccountId::from_uuid(uuid::Uuid::from_u128(4)),
            checkpoint: after,
            ledger: Vec::new(),
            receipts: Vec::new(),
            materializations: Vec::new(),
            evidence,
        }
    }

    fn insufficient(
        template: u32,
        remaining: u64,
        instance: u64,
        required: u64,
        weapon: u64,
    ) -> ItemAuditEvidence {
        ItemAuditEvidence {
            family: "auto_shot_exhausted".into(),
            fields: BTreeMap::from([
                ("template".into(), V::Unsigned(u64::from(template))),
                ("remaining".into(), V::Unsigned(remaining)),
                ("reason".into(), V::Text("insufficient".into())),
                ("resolution_id".into(), V::Text(format!("attack:e:2:shot:{template}"))),
                ("instance".into(), V::Unsigned(instance)),
                ("required".into(), V::Unsigned(required)),
                ("weapon".into(), V::Unsigned(weapon)),
            ]),
        }
    }

    #[test]
    fn insufficient_is_checked_against_the_unreduced_count_and_the_reduction_draw() {
        let rules = rules();
        let soul = template(&rules, |d| d.handler == "SoulShots");
        let (reducing, chance, reduced, full) = rules
            .parts()
            .items
            .iter()
            .find_map(|(t, d)| match d.reduced_soulshot.as_slice() {
                [c, r] if *r > 0 && d.soulshots > *r.max(&1) => {
                    Some((*t, u64::from(*c), u64::from(*r), u64::from(d.soulshots)))
                },
                _ => None,
            })
            .unwrap();
        let mut weapon = item(9, reducing, 1);
        weapon.location = ItemLocation::Paperdoll;
        weapon.slot = Some(EquipSlot::Rhand);
        let before = checkpoint(vec![weapon.clone(), item(5, soul, reduced)], &[soul]);
        let after = checkpoint(vec![weapon.clone(), item(5, soul, reduced)], &[]);
        let draw = |residue: u64| ItemAuditEvidence {
            family: "soulshot_reduction".into(),
            fields: BTreeMap::from([
                ("resolution_id".into(), V::Text(format!("attack:e:2:shot:{soul}"))),
                ("residue".into(), V::Unsigned(residue)),
            ]),
        };
        let check = |evidence: Vec<ItemAuditEvidence>| {
            validate_auto_shots(&before, &projection(after.clone(), evidence), Some(&rules))
        };
        let entry = insufficient(soul, reduced, 5, full, weapon.id.get());
        // Not reduced: `reduced` shots are below the full count.
        assert!(check(vec![entry.clone(), draw(chance)]).is_ok());
        // Reduced: `reduced` shots would have been enough, so the disable is not insufficient.
        assert!(check(vec![entry.clone(), draw(chance.checked_sub(1).unwrap())]).is_err());
        // A reducing weapon always draws; a missing draw fails closed, as does no rules.
        assert!(check(vec![entry.clone()]).is_err());
        assert!(validate_auto_shots(
            &before,
            &projection(after.clone(), vec![entry, draw(chance)]),
            None
        )
        .is_err());
        // `required` must be the unreduced count and `weapon` the wielded one.
        assert!(check(vec![
            insufficient(soul, reduced, 5, reduced, weapon.id.get()),
            draw(chance)
        ])
        .is_err());
        assert!(check(vec![insufficient(soul, reduced, 5, full, 8), draw(chance)]).is_err());
    }

    #[test]
    fn a_toggle_receipt_covers_one_template_and_entries_must_match_removals() {
        let rules = rules();
        let soul = template(&rules, |d| d.handler == "SoulShots");
        let spirit = template(&rules, |d| d.handler == "SpiritShot");
        let missing = |template: u32| ItemAuditEvidence {
            family: "auto_shot_exhausted".into(),
            fields: BTreeMap::from([
                ("template".into(), V::Unsigned(u64::from(template))),
                ("remaining".into(), V::Unsigned(0)),
                ("reason".into(), V::Text("missing".into())),
                ("resolution_id".into(), V::Text(format!("cast:1:shot:{template}"))),
            ]),
        };
        let toggle = ItemReceiptFact {
            request_id: uuid::Uuid::from_u128(11),
            fingerprint: "toggle".into(),
            result: FrozenItemResult {
                accepted_sequence: None,
                request_fingerprint: "toggle".into(),
                entity: crate::domain::zone::EntityId::from_uuid(uuid::Uuid::from_u128(3)),
                incarnation: 0,
                tick: Tick(2),
                inventory_revision: 2,
                receipt: uuid::Uuid::from_u128(11),
                detail: ItemResultDetail::Operation {
                    operation: ItemOperation::ToggleAutoShot,
                    affected: Vec::new(),
                    recipe: None,
                },
            },
        };
        let before = checkpoint(Vec::new(), &[soul, spirit]);
        let run = |shots: &[u32], evidence: Vec<ItemAuditEvidence>, toggles: usize| {
            let mut p = projection(checkpoint(Vec::new(), shots), evidence);
            p.receipts = std::iter::repeat_n(toggle.clone(), toggles).collect();
            validate_auto_shots(&before, &p, Some(&rules))
        };
        let code = |r: Result<(), CheckpointError>| match r {
            Err(CheckpointError::Constraint(name)) => name,
            other => format!("{other:?}"),
        };
        // Both removed: two evidenced, or one evidenced plus one toggle.
        assert!(run(&[], vec![missing(soul), missing(spirit)], 0).is_ok());
        assert!(run(&[], vec![missing(soul)], 1).is_ok());
        // One toggle never covers two unevidenced changes.
        assert_eq!(code(run(&[], Vec::new(), 1)), "item_toggle_receipt_required");
        assert_eq!(code(run(&[spirit, 7], vec![missing(soul)], 0)), "item_toggle_receipt_required");
        // A spurious entry (template kept) or a duplicate entry is refused even with a toggle.
        assert_eq!(
            code(run(&[soul], vec![missing(soul), missing(spirit)], 1)),
            "item_auto_shot_evidence"
        );
        assert_eq!(
            code(run(&[soul], vec![missing(spirit), missing(spirit)], 1)),
            "item_auto_shot_evidence"
        );
        // `missing` needs the stack absent before or after the tick (D136): a stack held on both
        // sides refuses; one granted later in the tick does not.
        let mut kept = checkpoint(vec![item(5, spirit, 3)], &[soul]);
        kept.paperdoll.clear();
        let mut stocked = checkpoint(vec![item(5, spirit, 3)], &[soul, spirit]);
        stocked.paperdoll.clear();
        let p = projection(kept.clone(), vec![missing(spirit)]);
        assert_eq!(
            code(validate_auto_shots(&stocked, &p, Some(&rules))),
            "item_auto_shot_evidence"
        );
        assert!(validate_auto_shots(&before, &p, Some(&rules)).is_ok());
    }

    /// D142: the admission reset of a new incarnation drops every toggle and every charge.
    #[test]
    fn an_incarnation_entry_is_accepted_only_for_the_complete_reset() {
        let rules = rules();
        let soul = template(&rules, |d| d.handler == "SoulShots");
        let spirit = template(&rules, |d| d.handler == "SpiritShot");
        let entry = |template: u32, remaining: u64, instance: Option<u64>| {
            let mut fields = BTreeMap::from([
                ("template".into(), V::Unsigned(u64::from(template))),
                ("remaining".into(), V::Unsigned(remaining)),
                ("reason".into(), V::Text("incarnation".into())),
                ("resolution_id".into(), V::Text("incarnation:e:9".into())),
            ]);
            if let Some(instance) = instance {
                fields.insert("instance".into(), V::Unsigned(instance));
            }
            ItemAuditEvidence {
                family: "auto_shot_exhausted".into(),
                fields,
            }
        };
        let mut charged = item(7, 2369, 1);
        charged.charges = 1;
        let before = checkpoint(vec![charged.clone(), item(5, soul, 40)], &[soul, spirit]);
        let mut cleared = charged.clone();
        cleared.charges = 0;
        let reset = checkpoint(vec![cleared.clone(), item(5, soul, 40)], &[]);
        let run = |after: Phase4Checkpoint, evidence| {
            validate_auto_shots(&before, &projection(after, evidence), Some(&rules))
        };
        assert!(run(reset.clone(), vec![entry(soul, 40, None), entry(spirit, 0, None)]).is_ok());
        // A charge left, a toggle left, an instance named, or a wrong count: refused.
        let still_charged = checkpoint(vec![charged, item(5, soul, 40)], &[]);
        assert!(run(still_charged, vec![entry(soul, 40, None), entry(spirit, 0, None)]).is_err());
        let kept = checkpoint(vec![cleared, item(5, soul, 40)], &[spirit]);
        assert!(run(kept, vec![entry(soul, 40, None)]).is_err());
        assert!(run(reset.clone(), vec![entry(soul, 40, Some(5)), entry(spirit, 0, None)]).is_err());
        assert!(run(reset, vec![entry(soul, 39, None), entry(spirit, 0, None)]).is_err());
    }
}
