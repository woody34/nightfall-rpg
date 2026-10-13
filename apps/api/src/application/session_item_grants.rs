//! Grants serialize with socket lifecycle without admitting offline owners publicly.
use super::{saved_position, Live, SessionContext};
use crate::application::ports::{
    ItemGrantRequest, ItemGrantRuntime, ItemMutationOperation, ItemMutationReceipt,
    ItemReceiptLookup,
};
use crate::application::AppError;
use crate::domain::zone::{
    EntityId, ItemGrantCommand, ItemPausedCore, PlayerLoad, PlayerProgression,
};
use crate::domain::{AccountId, CharacterId};
use std::sync::Arc;

#[async_trait::async_trait]
impl ItemGrantRuntime for SessionContext {
    async fn available(&self) -> Result<(), AppError> {
        self.zone.item_grants_available().await?;
        if self.registry.characters.is_none() {
            return Err(AppError::Unavailable("committed owner loading unavailable".into()));
        }
        Ok(())
    }

    async fn enqueue_grant_item(
        &self,
        account: AccountId,
        character: CharacterId,
        request: ItemGrantRequest,
    ) -> Result<ItemMutationReceipt, AppError> {
        self.available().await?;
        let entity = EntityId::from_uuid(character.as_uuid());
        let mut command = ItemGrantCommand {
            entity,
            account,
            request_id: request.key.as_uuid(),
            template_id: request.template_id,
            count: request.count,
            fingerprint: request.fingerprint,
            rules_hash: request.rules_hash,
            paused: None,
            known: None,
            conflict: None,
            enchant: request.enchant,
        };
        command
            .validate()
            .map_err(|error| AppError::InvalidArgument(error.to_string()))?;
        let lifecycle = Arc::clone(&self.registry.live).lock_owned().await;
        let service = self
            .zone
            .checkpoints
            .service()
            .ok_or_else(|| AppError::Unavailable("item checkpoint lane unavailable".into()))?;
        let lookup = service
            .lock()
            .await
            .item_receipt_lookup(
                account,
                ItemMutationOperation::GrantItem,
                &request.key,
                &command.fingerprint,
            )
            .await?;
        match lookup {
            ItemReceiptLookup::Known(receipt) => {
                receipt
                    .validate()
                    .map_err(|error| AppError::Unavailable(error.to_string()))?;
                command.known = Some(crate::domain::zone::phase4_projection::ItemReceiptFact {
                    request_id: receipt.key,
                    fingerprint: receipt.fingerprint,
                    result: receipt.result,
                });
            },
            ItemReceiptLookup::Conflict { stored_fingerprint } => {
                command.conflict = Some(stored_fingerprint);
            },
            ItemReceiptLookup::Unknown => {
                // Finish any earlier queued departure/admission before loading its parent fence.
                let snapshot = self
                    .zone
                    .snapshot()
                    .await
                    .map_err(|_| AppError::Unavailable("zone stopped".into()))?;
                let present = snapshot.entities.iter().any(|owner| owner.id == entity);
                if present != lifecycle.contains_key(&entity) {
                    return Err(AppError::Unavailable(
                        "character lifecycle transition pending".into(),
                    ));
                }
                if !present {
                    let repo = self.registry.characters.as_ref().ok_or_else(|| {
                        AppError::Unavailable("committed owner loading unavailable".into())
                    })?;
                    let loaded = repo.load_for_admission(character).await?.ok_or_else(|| {
                        AppError::NotFound {
                            entity: "character",
                            id: character.to_string(),
                        }
                    })?;
                    if loaded.identity.account_id != account {
                        return Err(AppError::PermissionDenied(
                            "character belongs to another account".into(),
                        ));
                    }
                    let position = saved_position(loaded.position).ok_or_else(|| {
                        AppError::FailedPrecondition("invalid saved position".into())
                    })?;
                    // A pending owner with no learned providers has the canonical empty paused origin.
                    let phase3 = match loaded.phase3 {
                        Some(checkpoint) => checkpoint,
                        None if loaded.class_state.learned_skills.is_empty()
                            && loaded.phase4.as_ref().is_some_and(|items| {
                                items.checkpoint.is_none() && !items.pending.is_empty()
                            }) =>
                        {
                            crate::domain::zone::Phase3Checkpoint::new()
                        },
                        None => {
                            return Err(AppError::FailedPrecondition(
                                "paused skill checkpoint unavailable".into(),
                            ))
                        },
                    };
                    command.paused = Some(Box::new(ItemPausedCore {
                        name: loaded.name.as_str().to_owned(),
                        position,
                        load: PlayerLoad {
                            phase4: loaded.phase4,
                            phase3: Some(phase3),
                            progression: Some(Box::new(PlayerProgression {
                                identity: loaded.identity,
                                class_state: loaded.class_state,
                                max_cp: 0,
                            })),
                            checkpoint_revision: Some(loaded.revision),
                            class: loaded.class_profile,
                            level: loaded.level,
                            xp: loaded.xp,
                            hp: loaded.hp,
                            mp: loaded.mp,
                            alive: loaded.alive,
                        },
                    }));
                }
            },
        }
        let receiver = self.zone.enqueue_grant_item(command).await?;
        // A caller timeout must not release lifecycle ordering while acknowledged work may
        // still be committing. This task owns the guard until the actor resolves or stops.
        wait_for_grant_checkpoint(lifecycle, receiver).await
    }
}

async fn wait_for_grant_checkpoint(
    lifecycle: tokio::sync::OwnedMutexGuard<std::collections::BTreeMap<EntityId, Live>>,
    receiver: crate::application::ports::ItemGrantReply,
) -> Result<ItemMutationReceipt, AppError> {
    tokio::spawn(async move {
        let _lifecycle = lifecycle;
        receiver
            .await
            .map_err(|_| AppError::Unavailable("zone stopped; retry the same key".into()))?
    })
    .await
    .map_err(|_| AppError::Unavailable("grant reply task stopped; retry the same key".into()))?
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{wait_for_grant_checkpoint, AppError, Arc};
    use crate::application::session::SessionRegistry;
    use std::time::Duration;
    #[tokio::test]
    async fn grant_timeout_keeps_lifecycle_locked_until_checkpoint_reply() {
        let registry = SessionRegistry::default();
        let guard = Arc::clone(&registry.live).lock_owned().await;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            wait_for_grant_checkpoint(guard, receiver)
        )
        .await
        .is_err());
        assert!(registry.live.try_lock().is_err());
        sender
            .send(Err(AppError::Unavailable("actor stopped".into())))
            .unwrap();
        let _released = tokio::time::timeout(Duration::from_secs(1), registry.live.lock())
            .await
            .unwrap();
    }
}
