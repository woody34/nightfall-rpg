//! API-owned durable allocator and pending item inputs; simulation owns actual item state.
use crate::application::CheckpointError;
use crate::domain::{class::ClassId, zone::ZoneId, CharacterId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Durable, disjoint production reservation. The low 2^32 block belongs to fixture seeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemIdRange {
    pub zone_id: ZoneId,
    pub epoch: u64,
    pub range_start: u64,
    pub range_end_exclusive: u64,
    pub next_reserved: u64,
}
impl ItemIdRange {
    /// Check signed bigint endpoints and persisted exhaustion before exposing a reservation.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.range_start < 1_u64 << 32
            || self.range_start >= self.range_end_exclusive
            || self.range_start > self.next_reserved
            || self.next_reserved > self.range_end_exclusive
            || i64::try_from(self.range_end_exclusive).is_err()
            || i64::try_from(self.epoch).is_err()
            || i32::try_from(self.zone_id.0).is_err()
        {
            return Err(CheckpointError::Constraint("item_id_ranges_bounds".into()));
        }
        Ok(())
    }
}

/// Exact XML row, including unsupported-use extras and its source equipped flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StarterKitItem {
    pub template_id: u32,
    pub count: u64,
    pub equipped: bool,
}

/// XML order is semantic. The creation class never changes after a later transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct StarterKitManifest {
    pub source_class_id: ClassId,
    pub items: Vec<StarterKitItem>,
}
impl StarterKitManifest {
    /// Structural import checks; template capability/stackability belongs to the zone rules.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.items.is_empty(), "empty starter manifest");
        for item in &self.items {
            anyhow::ensure!(
                item.template_id > 0 && i32::try_from(item.template_id).is_ok(),
                "invalid starter template"
            );
            anyhow::ensure!(
                item.count > 0 && i64::try_from(item.count).is_ok(),
                "invalid starter count"
            );
        }
        Ok(())
    }
    /// Hash compact sorted-key JSON, shared with frozen migration 000011 manifests.
    pub fn hash(&self) -> anyhow::Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        self.validate()?;
        let canonical = serde_json::to_value(self)?;
        Ok(Sha256::digest(serde_json::to_vec(&canonical)?).into())
    }
}

/// Dev grant input; no owned item exists before actor materialization commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct PendingGrantManifest {
    pub template_id: u32,
    pub count: u64,
}

/// Typed pending contents, encoded as the contained value in the SQL payload column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingItemManifest {
    /// Source kit, selected from base creation class.
    Kit(StarterKitManifest),
    /// Authenticated fixture-only dev request.
    Grant(PendingGrantManifest),
}

/// The source of first-admission materialization (and its ledger operation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingItemKind {
    /// Newly created character.
    Starter,
    /// Existing character selected by migration 000011.
    Backfill,
    /// Dev RPC, gated before any persistence.
    Grant,
}

/// Completion proof is separate from pending inputs, never speculative ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingItemStatus {
    /// Awaiting log-backed actor admission.
    Pending,
    /// Committed in the fenced character transaction.
    Materialized {
        /// Final inventory revision.
        revision: u64,
        /// Frozen result identity.
        receipt: uuid::Uuid,
    },
}

/// Durable pending request read under the character lock for admission.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PendingItemRequest {
    pub request_id: uuid::Uuid,
    pub character_id: CharacterId,
    pub kind: PendingItemKind,
    pub fingerprint: String,
    pub manifest_version: u32,
    pub manifest_hash: [u8; 32],
    pub manifest: PendingItemManifest,
    pub status: PendingItemStatus,
}

/// API allocator and first-admission inputs. Item ownership is exclusively actor committed.
#[async_trait]
pub trait ItemAdmissionRepository: Send + Sync {
    /// Same zone/epoch returns the same reservation. Different epochs get disjoint blocks.
    /// Exhaustion and invalid integer bounds fail closed, without allocating a partial block.
    async fn reserve_item_range(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> Result<ItemIdRange, CheckpointError>;
    /// Reads both pending and completed manifests; completed ones prevent a second kit.
    async fn pending_item_requests(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Vec<PendingItemRequest>>;
}

/// One revision-consistent retained inventory projection. None identifies legacy state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RetainedInventory {
    pub character_id: CharacterId,
    pub character_revision: u64,
    pub checkpoint: Option<crate::domain::zone::Phase4Checkpoint>,
}

/// Retained state only; the use case must exclude concurrent/live admission before reading.
#[async_trait]
pub trait ItemInventoryRepository: Send + Sync {
    /// Reads checkpoint and scalar rows under the parent lock; corruption fails closed.
    async fn retained_inventory(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Option<RetainedInventory>>;
}

/// Startup-resolved immutable catalogue. Implementations validate the epoch skill binding;
/// requests never parse generated files or synthesize an empty rollout response.
pub trait ItemCataloguePort: Send + Sync {
    /// Unsupported or absent bundles are UNAVAILABLE; corrupt data is infrastructure.
    fn catalogue(
        &self,
    ) -> Result<std::sync::Arc<crate::domain::zone::Phase4Rules>, crate::application::AppError>;
}

/// Zone/lifecycle boundary for a detached retained read. Implementations hold the same
/// admission/departure barrier until the repository's revision-consistent read completes.
#[async_trait]
pub trait ItemInventoryRuntime: Send + Sync {
    /// `UNAVAILABLE` if the zone stopped, `FAILED_PRECONDITION` while admitted; no live DB baseline.
    async fn detached_inventory(
        &self,
        character: CharacterId,
        repository: &dyn ItemInventoryRepository,
    ) -> Result<Option<RetainedInventory>, crate::application::AppError>;
}
/// The SQL ledger operation vocabulary (D29). Pure owner-sidecar changes have no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ItemLedgerOperation {
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
}
impl ItemLedgerOperation {
    /// Stable SQL spelling; never derive it from a client-provided string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Grant => "grant",
            Self::Starter => "starter",
            Self::Backfill => "backfill",
            Self::Equip => "equip",
            Self::Unequip => "unequip",
            Self::Consume => "consume",
            Self::Enchant => "enchant",
            Self::Destroy => "destroy",
            Self::Crystallize => "crystallize",
            Self::Refine => "refine",
            Self::CancelRefine => "cancel_refine",
            Self::Exchange => "exchange",
            Self::Use => "use",
        }
    }
}

/// Exact before/after scalar endpoints plus the explicit net operation count delta.
/// Consumption aggregates may interleave with other facts; endpoint subtraction is not
/// a substitute for this delta or the ordered verified cause/evidence objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemLedgerDelta {
    pub before: Option<crate::domain::zone::ItemInstance>,
    pub after: Option<crate::domain::zone::ItemInstance>,
    pub count_delta: i64,
    pub causes: Vec<serde_json::Value>,
}

/// One actual instance change in a single acknowledged zone tick. No live-row FK is needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ItemLedgerEntry {
    pub character_id: CharacterId,
    pub zone_id: ZoneId,
    pub epoch: u64,
    pub tick: crate::domain::zone::Tick,
    pub revision: u64,
    pub operation: ItemLedgerOperation,
    pub instance_id: crate::domain::zone::ItemInstanceId,
    pub operation_ordinal: u32,
    pub delta: ItemLedgerDelta,
    pub receipt_ref: Option<uuid::Uuid>,
}
impl ItemLedgerEntry {
    /// Mirror SQL widths before any claim/write and reject fabricated pure-sidecar rows.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if i32::try_from(self.zone_id.0).is_err()
            || i64::try_from(self.epoch).is_err()
            || i64::try_from(self.tick.0).is_err()
            || i64::try_from(self.revision).is_err()
            || i32::try_from(self.operation_ordinal).is_err()
            || (self.operation == ItemLedgerOperation::Consume && self.operation_ordinal != 0)
            || (self.delta.before.is_none() && self.delta.after.is_none())
        {
            return Err(CheckpointError::Constraint("item_ledger_projection".into()));
        }
        for item in self.delta.before.iter().chain(self.delta.after.iter()) {
            item.validate()
                .map_err(|_| CheckpointError::Constraint("item_ledger_scalar".into()))?;
            if item.id != self.instance_id || item.revision > self.revision {
                return Err(CheckpointError::Constraint("item_ledger_identity".into()));
            }
        }
        Ok(())
    }
}
