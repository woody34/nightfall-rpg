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
