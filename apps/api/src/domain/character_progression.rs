//! Shared deterministic character identity, class ledger and frozen transfer responses.
//! No persistence or transport types belong here; the zone is the mutable ledger's owner.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::class::ClassId;
use super::subclass::Sex;
use super::{AccountId, BaseStats, CharacterId, CharacterName, Race};

/// Only two class transfers are reachable in Phase 2. Successful receipts are retained forever.
pub const MAX_TRANSFER_RECEIPTS: usize = 2;

/// Appearance indices into the selectable race's assets (Phase 2 has only index zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CharacterAppearance {
    pub sex: Sex,
    pub hair_style: u32,
    pub hair_color: u32,
    pub face: u32,
}

impl Default for CharacterAppearance {
    fn default() -> Self {
        Self {
            sex: Sex::Male,
            hair_style: 0,
            hair_color: 0,
            face: 0,
        }
    }
}

/// Immutable identity, copied into admission commands and replay snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CharacterIdentity {
    pub account_id: AccountId,
    pub race: Race,
    pub base_class_id: ClassId,
    pub appearance: CharacterAppearance,
}

/// Exact response at the successful transfer tick. Integer position is in milli-tiles.
/// Deliberately contains no ClassState/receipts, avoiding recursive snapshots and responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct FrozenTransferResult {
    pub character_id: CharacterId,
    pub identity: CharacterIdentity,
    pub name: CharacterName,
    pub current_class_id: ClassId,
    pub level: u32,
    pub xp: u64,
    pub sp: u64,
    pub stats: BaseStats,
    pub position_millitiles: [i32; 2],
    pub hp: u32,
    pub mp: u32,
    pub cp: u32,
    pub max_hp: u32,
    pub max_mp: u32,
    pub max_cp: u32,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
    pub granted_skill_keys: Vec<String>,
}

/// Only successful mutations enter this ledger. Target + character define the fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SuccessfulTransferReceipt {
    pub key: Uuid,
    pub target_class_id: ClassId,
    pub result: FrozenTransferResult,
}

/// A bounded, forever-retained transfer history and the active main-class resource ledger.
/// XP/level/HP/MP remain in the existing progression state to avoid competing copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ClassState {
    pub base_class_id: ClassId,
    pub current_class_id: ClassId,
    pub sp: u64,
    pub cp: u32,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
    /// Bits 0 and 1 mark the level-20 and level-40 token grants, never reset by death.
    pub milestone_claimed_mask: u8,
    #[serde(deserialize_with = "deserialize_receipts")]
    pub successful_transfer_receipts: Vec<SuccessfulTransferReceipt>,
}

impl ClassState {
    /// Empty level-one ledger. Resource maxima are derived by the stat engine.
    #[must_use]
    pub const fn new(base_class_id: ClassId) -> Self {
        Self {
            base_class_id,
            current_class_id: base_class_id,
            sp: 0,
            cp: 0,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            milestone_claimed_mask: 0,
            successful_transfer_receipts: Vec::new(),
        }
    }

    /// Rejects impossible persisted ledgers before any checkpoint write.
    pub fn validate(&self) -> Result<(), ClassStateError> {
        validate_receipts(&self.successful_transfer_receipts)?;
        if self.milestone_claimed_mask & !3 != 0 {
            return Err(ClassStateError::MilestoneMask);
        }
        Ok(())
    }

    /// Inserts a success exactly once, refusing conflicting keys and history overflow.
    pub fn record_success(
        &mut self,
        receipt: SuccessfulTransferReceipt,
    ) -> Result<(), ClassStateError> {
        if let Some(known) = self.receipt(receipt.key) {
            return if known == &receipt {
                Ok(())
            } else {
                Err(ClassStateError::ConflictingReceipt)
            };
        }
        if self.successful_transfer_receipts.len() >= MAX_TRANSFER_RECEIPTS {
            return Err(ClassStateError::ReceiptLimit);
        }
        if receipt.result.current_class_id != receipt.target_class_id {
            return Err(ClassStateError::ReceiptTarget);
        }
        self.successful_transfer_receipts.push(receipt);
        Ok(())
    }

    /// Lookup precedes live eligibility, so retry returns its original frozen result.
    #[must_use]
    pub fn receipt(&self, key: Uuid) -> Option<&SuccessfulTransferReceipt> {
        self.successful_transfer_receipts
            .iter()
            .find(|r| r.key == key)
    }
}

/// Invalid shared class ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClassStateError {
    /// More successes than reachable transfer tiers.
    #[error("at most two successful transfer receipts are allowed")]
    ReceiptLimit,
    /// A key occurs twice or conflicts with its existing success.
    #[error("conflicting transfer receipt")]
    ConflictingReceipt,
    /// The frozen response disagrees with the requested target.
    #[error("transfer receipt target does not match result")]
    ReceiptTarget,
    /// An unknown token milestone bit is present.
    #[error("unknown token milestone bit")]
    MilestoneMask,
}

fn validate_receipts(receipts: &[SuccessfulTransferReceipt]) -> Result<(), ClassStateError> {
    if receipts.len() > MAX_TRANSFER_RECEIPTS {
        return Err(ClassStateError::ReceiptLimit);
    }
    let mut keys = std::collections::BTreeSet::new();
    for receipt in receipts {
        if !keys.insert(receipt.key) {
            return Err(ClassStateError::ConflictingReceipt);
        }
        if receipt.result.current_class_id != receipt.target_class_id {
            return Err(ClassStateError::ReceiptTarget);
        }
    }
    Ok(())
}

fn deserialize_receipts<'de, D>(deserializer: D) -> Result<Vec<SuccessfulTransferReceipt>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let receipts = Vec::deserialize(deserializer)?;
    validate_receipts(&receipts).map_err(serde::de::Error::custom)?;
    Ok(receipts)
}
