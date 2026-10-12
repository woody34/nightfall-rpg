//! Dev grants retain the production endpoint but require the exact startup auth gate (A15).
use crate::application::AppError;

/// Gate before any receipt lookup, queued input, allocator use or ownership write.
pub fn require_dev_grants(enabled: bool) -> Result<(), AppError> {
    if enabled {
        Ok(())
    } else {
        Err(AppError::FailedPrecondition("GrantItem requires AUTH_DEV_TOKENS=1".into()))
    }
}

use crate::application::ports::{
    grant_item_fingerprint, ItemCataloguePort, ItemGrantRequest, ItemGrantRuntime,
    ItemMutationOperation, ItemMutationReceipt, ItemReceiptLookup, ItemReceiptRepository,
};
use crate::application::{CharacterRepository, IdempotencyKey};
use crate::domain::{AccountId, CharacterId};
use std::{sync::Arc, time::Duration};

/// Wire input contains no trusted account field.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct GrantItemInput {
    pub caller: AccountId,
    pub character_id: Vec<u8>,
    pub template_id: u32,
    pub count: u64,
    pub idempotency_key: String,
}

/// Dev gate and immutable receipt routing; only the zone may materialize ownership.
pub struct GrantItem {
    enabled: bool,
    characters: Arc<dyn CharacterRepository>,
    receipts: Option<Arc<dyn ItemReceiptRepository>>,
    catalogue: Option<Arc<dyn ItemCataloguePort>>,
    runtime: Option<Arc<dyn ItemGrantRuntime>>,
}
impl GrantItem {
    /// Missing mandatory runtime/catalogue/receipt ports preserve rollout refusal.
    pub fn new(
        enabled: bool,
        characters: Arc<dyn CharacterRepository>,
        receipts: Option<Arc<dyn ItemReceiptRepository>>,
        catalogue: Option<Arc<dyn ItemCataloguePort>>,
        runtime: Option<Arc<dyn ItemGrantRuntime>>,
    ) -> Self {
        Self {
            enabled,
            characters,
            receipts,
            catalogue,
            runtime,
        }
    }

    /// Gate/zone availability precede DB reads. Exact success precedes later eligibility.
    pub async fn execute(&self, input: GrantItemInput) -> Result<ItemMutationReceipt, AppError> {
        require_dev_grants(self.enabled)?;
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("item runtime is unavailable".into()))?;
        tokio::time::timeout(Duration::from_secs(10), runtime.available())
            .await
            .map_err(|_| AppError::Unavailable("item runtime availability timed out".into()))??;
        let receipts = self.receipts.as_ref().ok_or_else(|| {
            AppError::Unavailable("item receipt persistence is unavailable".into())
        })?;
        let catalogue = self
            .catalogue
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("item catalogue is unavailable".into()))?;
        let character =
            CharacterId::from_uuid(uuid::Uuid::from_slice(&input.character_id).map_err(|_| {
                AppError::InvalidArgument("character_id must be exactly 16 UUID bytes".into())
            })?);
        let key = IdempotencyKey::parse(&input.idempotency_key)
            .map_err(|error| AppError::InvalidArgument(error.to_string()))?;
        if key.as_uuid().is_nil()
            || input.template_id == 0
            || i32::try_from(input.template_id).is_err()
            || input.count == 0
            || i64::try_from(input.count).is_err()
        {
            return Err(AppError::InvalidArgument(
                "positive template/count and nonempty UUID key are required within storage bounds"
                    .into(),
            ));
        }
        let fingerprint = grant_item_fingerprint(character, input.template_id, input.count);
        match receipts
            .item_receipt_lookup(input.caller, ItemMutationOperation::GrantItem, &key, &fingerprint)
            .await?
        {
            ItemReceiptLookup::Known(receipt) => {
                validate_reply(&receipt, input.caller, character, key, &fingerprint)?;
                return Ok(*receipt);
            },
            ItemReceiptLookup::Conflict => return Err(AppError::IdempotencyConflict),
            ItemReceiptLookup::Unknown => {},
        }
        let owner = self
            .characters
            .get(character)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: character.to_string(),
            })?;
        if owner.account_id != input.caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        let rules = catalogue.catalogue()?;
        if !rules.parts().items.contains_key(&input.template_id) {
            return Err(AppError::InvalidArgument("unknown template_id".into()));
        }
        let capability = rules
            .parts()
            .capabilities
            .get(&input.template_id)
            .ok_or_else(|| anyhow::anyhow!("item capability is missing"))?;
        if !capability.storable {
            return Err(AppError::FailedPrecondition(format!(
                "item storage is deferred: {}",
                capability.blockers.join(", ")
            )));
        }
        let request = ItemGrantRequest {
            key,
            template_id: input.template_id,
            count: input.count,
            fingerprint: fingerprint.clone(),
            rules_hash: rules.hash(),
        };
        // A timed-out queued operation may still commit. Retrying the same key returns its
        // original receipt, never a second grant or a reread of later inventory.
        let receipt = tokio::time::timeout(
            Duration::from_secs(10),
            runtime.enqueue_grant_item(input.caller, character, request),
        )
        .await
        .map_err(|_| {
            AppError::Unavailable("grant acknowledgement timed out; retry the same key".into())
        })??;
        validate_reply(&receipt, input.caller, character, key, &fingerprint)?;
        if receipt
            .inventory
            .as_ref()
            .is_none_or(|inventory| inventory.checkpoint.rules_hash != rules.hash())
        {
            return Err(
                anyhow::anyhow!("grant reply rules hash differs from admitted input").into()
            );
        }
        Ok(receipt)
    }
}
fn validate_reply(
    receipt: &ItemMutationReceipt,
    account: AccountId,
    character: CharacterId,
    key: IdempotencyKey,
    fingerprint: &str,
) -> Result<(), AppError> {
    receipt.validate().map_err(anyhow::Error::from)?;
    if receipt.account_id != account
        || receipt.character_id != character
        || receipt.operation != ItemMutationOperation::GrantItem
        || receipt.key != key.as_uuid()
        || receipt.fingerprint != fingerprint
    {
        return Err(anyhow::anyhow!("grant reply identity differs from admitted input").into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "grant_item_tests.rs"]
mod tests;
