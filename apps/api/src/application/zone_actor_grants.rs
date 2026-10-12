//! Server grants use the actor's ordinary log, checkpoint and pending-reply barriers.
use super::{
    class_error, MutationReply, Queued, TickGate, TickSource, ZoneActor, ZoneHandle, ZoneSendError,
};
use crate::application::ports::{ItemMutationOperation, ItemReceiptLookup};
use crate::domain::zone::{
    AppliedTick, CommandSource, ItemGrantCommand, Tick, TickError, ZoneCommand, ZoneInput,
};
use tokio::sync::oneshot;

impl ZoneHandle {
    /// No repository reads precede this durable V6 availability check.
    pub async fn item_grants_available(&self) -> Result<(), crate::application::AppError> {
        if !self.phase4_durable
            || self.is_paused()
            || self.commands.is_closed()
            || self.persistence_failed()
        {
            return Err(crate::application::AppError::Unavailable(
                "durable V6 zone unavailable".into(),
            ));
        }
        let service = self.checkpoints.service().ok_or_else(|| {
            crate::application::AppError::Unavailable("item checkpoint lane unavailable".into())
        })?;
        if !service.lock().await.items_enabled() {
            return Err(crate::application::AppError::Unavailable(
                "item checkpoint writer unavailable".into(),
            ));
        }
        Ok(())
    }

    /// Queue canonical server work. Success resolves only after log and checkpoint acknowledgement.
    pub async fn enqueue_grant_item(
        &self,
        command: ItemGrantCommand,
    ) -> Result<crate::application::ports::ItemGrantReply, crate::application::AppError> {
        self.item_grants_available().await?;
        command
            .validate()
            .map_err(|error| crate::application::AppError::InvalidArgument(error.to_string()))?;
        let (reply, receiver) = oneshot::channel();
        self.enqueue(Queued {
            input: ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(command))),
            trace: None,
            reply: Some(MutationReply::Grant(reply)),
        })
        .map_err(|error| match error {
            ZoneSendError::Full(_) | ZoneSendError::AdmissionTooLarge(_) => {
                crate::application::AppError::ResourceExhausted(
                    "item grant queue unavailable".into(),
                )
            },
            ZoneSendError::Paused(_) | ZoneSendError::Closed(_) => {
                crate::application::AppError::Unavailable("zone unavailable".into())
            },
        })?;
        Ok(receiver)
    }
}

impl<T: TickSource, G: TickGate> ZoneActor<T, G> {
    pub(super) async fn preflight_grants(
        &mut self,
        selected: &[usize],
    ) -> Result<bool, (Tick, TickError)> {
        let fence = (self.state.next_tick(), TickError::PersistenceFence);
        let mut changed = false;
        for index in selected {
            let queued = self.pending.get(*index).ok_or(fence)?;
            let ZoneCommand::MaterializePendingItems(command) = &queued.input.command else {
                continue;
            };
            if queued.input.source != CommandSource::System {
                continue;
            }
            command.validate().map_err(|_| fence)?;
            let service = self.checkpoints.service().ok_or(fence)?;
            let lane = service.lock().await;
            if !self.gate.durable() || !lane.items_enabled() {
                return Err(fence);
            }
            let key = crate::application::IdempotencyKey::from_uuid(command.request_id);
            let mut normalized = command.as_ref().clone();
            match lane
                .item_receipt_lookup(
                    command.account,
                    ItemMutationOperation::GrantItem,
                    &key,
                    &command.fingerprint,
                )
                .await
                .map_err(|_| fence)?
            {
                ItemReceiptLookup::Unknown => {
                    if command.known.is_some() || command.conflict.is_some() {
                        return Err(fence);
                    }
                    if self.state.entity(command.entity).is_some()
                        && !lane.transfer_ready(command.entity)
                    {
                        return Err(fence);
                    }
                },
                ItemReceiptLookup::Conflict { stored_fingerprint } => {
                    if command.known.is_some()
                        || command
                            .conflict
                            .as_ref()
                            .is_some_and(|old| old != &stored_fingerprint)
                    {
                        return Err(fence);
                    }
                    normalized.conflict = Some(stored_fingerprint);
                },
                ItemReceiptLookup::Known(receipt) => {
                    receipt.validate().map_err(|_| fence)?;
                    if receipt.account_id != command.account
                        || receipt.character_id.as_uuid() != command.entity.as_uuid()
                        || receipt.key != command.request_id
                        || receipt.fingerprint != command.fingerprint
                        || receipt.operation != ItemMutationOperation::GrantItem
                        || command.conflict.is_some()
                    {
                        return Err(fence);
                    }
                    let known = crate::domain::zone::phase4_projection::ItemReceiptFact {
                        request_id: receipt.key,
                        fingerprint: receipt.fingerprint,
                        result: receipt.result,
                    };
                    if command.known.as_ref().is_some_and(|old| old != &known) {
                        return Err(fence);
                    }
                    normalized.known = Some(known);
                },
            }
            normalized.validate().map_err(|_| fence)?;
            drop(lane);
            if normalized != **command {
                self.pending.get_mut(*index).ok_or(fence)?.input.command =
                    ZoneCommand::MaterializePendingItems(Box::new(normalized));
                changed = true;
            }
        }
        Ok(changed)
    }

    pub(super) async fn committed_grant(
        &mut self,
        record: &AppliedTick,
        command: &crate::domain::zone::AppliedCommand,
    ) -> Result<crate::application::ports::ItemMutationReceipt, crate::application::AppError> {
        if let Some(disposition) = record
            .dispositions
            .iter()
            .find(|d| d.ordinal == command.ordinal)
        {
            return Err(class_error(disposition.reason));
        }
        let ZoneCommand::MaterializePendingItems(grant) = &command.command else {
            return Err(crate::application::AppError::Unavailable("invalid grant envelope".into()));
        };
        let service = self.checkpoints.service().ok_or_else(|| {
            crate::application::AppError::Unavailable("item checkpoint lane unavailable".into())
        })?;
        let key = crate::application::IdempotencyKey::from_uuid(grant.request_id);
        let receipt = service
            .lock()
            .await
            .item_receipt_lookup(
                grant.account,
                ItemMutationOperation::GrantItem,
                &key,
                &grant.fingerprint,
            )
            .await
            .map_err(|_| {
                crate::application::AppError::Unavailable(
                    "committed grant receipt lookup failed; retry same key".into(),
                )
            })?;
        let ItemReceiptLookup::Known(receipt) = receipt else {
            return Err(crate::application::AppError::Unavailable(
                "committed grant receipt missing".into(),
            ));
        };
        receipt.validate().map_err(|_| {
            crate::application::AppError::Unavailable("invalid committed grant receipt".into())
        })?;
        if receipt.account_id != grant.account
            || receipt.character_id.as_uuid() != grant.entity.as_uuid()
            || receipt.key != grant.request_id
            || receipt.fingerprint != grant.fingerprint
            || receipt.operation != ItemMutationOperation::GrantItem
        {
            return Err(crate::application::AppError::Unavailable(
                "committed grant receipt identity mismatch".into(),
            ));
        }
        Ok(*receipt)
    }
}
