//! Authenticated grants enter the same actor log/checkpoint barrier as item intents.
use super::{IdempotencyKey, ItemMutationReceipt};
use crate::application::AppError;
use crate::domain::{AccountId, CharacterId};

/// Complete normalized input. Identity and hash must survive queuing and crash recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct ItemGrantRequest {
    pub key: IdempotencyKey,
    pub template_id: u32,
    pub count: u64,
    pub fingerprint: String,
    pub rules_hash: [u8; 32],
}

/// The implementation serializes online/offline grants with lifecycle transitions (D30).
#[async_trait::async_trait]
pub trait ItemGrantRuntime: Send + Sync {
    /// Fail UNAVAILABLE before repository reads if no admissible durable V6 zone exists.
    async fn available(&self) -> Result<(), AppError>;

    /// Never publicly spawn an offline player. Return the immutable successful receipt only
    /// after log acknowledgement AND atomic checkpoint commit; errors create no ownership.
    async fn enqueue_grant_item(
        &self,
        account: AccountId,
        character: CharacterId,
        request: ItemGrantRequest,
    ) -> Result<ItemMutationReceipt, AppError>;
}

/// Account/operation key scope is separate; hash the complete body, excluding the key.
pub fn grant_item_fingerprint(character: CharacterId, template: u32, count: u64) -> String {
    crate::domain::zone::item_grant::grant_fingerprint(character, template, count)
}
