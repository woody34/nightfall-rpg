//! Item settlement is a child of the normal fenced checkpoint transaction, never DB-first.
use super::{
    character_repository::checkpoint_anyhow_error, entities, idempotency, PgCharacterRepository,
};
use crate::application::ports::{
    ItemCheckpointRepository, ItemCheckpointWork, ItemMutationOperation, ItemMutationReceipt,
    ItemReceiptLookup, ItemReceiptRepository, PendingItemManifest, PendingItemRequest,
};
use crate::application::{CharacterCheckpoint, CheckpointError, CheckpointOutcome, IdempotencyKey};
use crate::domain::zone::{ItemLocation, Phase4Checkpoint};
use crate::domain::{AccountId, CharacterId, DomainEvent};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, EntityTrait, Statement};

#[async_trait::async_trait]
impl ItemReceiptRepository for PgCharacterRepository {
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<ItemReceiptLookup> {
        let stored = entities::idempotency_keys::Entity::find_by_id((
            key.as_uuid(),
            account.as_uuid(),
            operation.as_str().to_owned(),
        ))
        .one(&self.db)
        .await?;
        let Some(stored) = stored else {
            return Ok(ItemReceiptLookup::Unknown);
        };
        if stored.fingerprint != fingerprint {
            return Ok(ItemReceiptLookup::Conflict {
                stored_fingerprint: stored.fingerprint,
            });
        }
        let frozen = entities::item_mutation_receipts::Entity::find_by_id((
            account.as_uuid(),
            operation.as_str().to_owned(),
            key.as_uuid(),
        ))
        .one(&self.db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("item key has no frozen receipt"))?;
        let receipt: ItemMutationReceipt = serde_json::from_value(frozen.response.clone())?;
        receipt.validate()?;
        anyhow::ensure!(
            receipt.account_id == account
                && receipt.key == key.as_uuid()
                && receipt.operation == operation
                && receipt.character_id.as_uuid() == frozen.character_id
                && receipt.fingerprint == frozen.fingerprint
                && receipt.fingerprint == stored.fingerprint
                && stored.response == frozen.response,
            "item receipt scope/body disagreement"
        );
        Ok(ItemReceiptLookup::Known(Box::new(receipt)))
    }
}

#[async_trait::async_trait]
impl ItemCheckpointRepository for PgCharacterRepository {
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &ItemCheckpointWork,
        learning: Option<&crate::application::ports::SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        match self
            .timed(
                "checkpoint_items",
                Box::pin(self.checkpoint_tx(cp, events, learning, Some(work))),
            )
            .await?
        {
            crate::application::ports::LearningCheckpointOutcome::Checkpoint(outcome) => {
                Ok(outcome)
            },
            crate::application::ports::LearningCheckpointOutcome::ReceiptReplayed(_) => {
                Ok(CheckpointOutcome::Stale)
            },
        }
    }
}

/// Claim all request scopes before the revision fence; existing success cannot acknowledge
/// another speculative tick. Exact checkpoint retries have already been checked first.
pub(super) async fn claim_receipts(
    tx: &DatabaseTransaction,
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
) -> Result<bool, CheckpointError> {
    for receipt in work.receipts(cp)? {
        let claim = idempotency::claim(
            tx,
            receipt.account_id,
            receipt.operation.as_str(),
            &IdempotencyKey::from_uuid(receipt.key),
            &receipt.fingerprint,
            serde_json::to_value(&receipt).map_err(anyhow::Error::from)?,
        )
        .await
        .map_err(checkpoint_anyhow_error)?;
        if let idempotency::Claim::Existing { fingerprint, .. } = claim {
            if fingerprint != receipt.fingerprint {
                return Err(CheckpointError::KeyReused);
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Ownership, append-only audit, frozen replies and pending completion share the parent TX.
pub(super) async fn persist(
    tx: &DatabaseTransaction,
    cp: &CharacterCheckpoint,
    work: &ItemCheckpointWork,
    before: Option<&Phase4Checkpoint>,
) -> Result<(), CheckpointError> {
    work.validate_before(before)?;
    let requests = super::item_admission::load_pending(tx, cp.character_id).await?;
    work.validate_pending(&requests)?;
    persist_range(tx, work).await?;
    persist_rows(tx, cp.character_id, &work.projection.checkpoint, before, work).await?;
    for row in work.ledger_entries()? {
        execute(tx, "INSERT INTO item_ledger(character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta,receipt_ref) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            vec![row.character_id.as_uuid().into(), i32::try_from(row.zone_id.0).map_err(anyhow::Error::from)?.into(),
                signed(row.epoch)?.into(), signed(row.tick.0)?.into(), signed(row.revision)?.into(), row.operation.as_str().into(),
                signed(row.instance_id.get())?.into(), i32::try_from(row.operation_ordinal).map_err(anyhow::Error::from)?.into(),
                serde_json::to_value(&row.delta).map_err(anyhow::Error::from)?.into(), row.receipt_ref.into()]).await?;
    }
    // D76: the whole tick is appended, so check row-position ordinals now; a violation is a
    // permanent named constraint here rather than a retryable commit failure.
    execute(tx, "SET CONSTRAINTS item_ledger_row_ordinal IMMEDIATE", Vec::new()).await?;
    for receipt in work.receipts(cp)? {
        execute(tx, "INSERT INTO item_mutation_receipts(account_id,operation,key,character_id,fingerprint,response) VALUES($1,$2,$3,$4,$5,$6)",
            vec![receipt.account_id.as_uuid().into(), receipt.operation.as_str().into(), receipt.key.into(),
                receipt.character_id.as_uuid().into(), receipt.fingerprint.clone().into(), serde_json::to_value(&receipt).map_err(anyhow::Error::from)?.into()]).await?;
    }
    for completion in &work.projection.materializations {
        let request = requests
            .iter()
            .find(|request| request.request_id == completion.request_id)
            .ok_or_else(|| CheckpointError::Constraint("pending_item_completion_missing".into()))?;
        complete(tx, request, completion.inventory_revision, completion.receipt.key).await?;
    }
    super::phase4_checkpoint::persist_retained(tx, cp.character_id, &work.projection.checkpoint)
        .await?;
    Ok(())
}

async fn persist_range(
    tx: &DatabaseTransaction,
    work: &ItemCheckpointWork,
) -> Result<(), CheckpointError> {
    let (start, next, end) = work.allocation.bounds();
    if end <= 1_u64 << 32 {
        return Ok(());
    } // Recorded fixture range; never a production reservation.
    let rows = execute(tx, "UPDATE item_id_ranges SET next_reserved=GREATEST(next_reserved,$4),updated_at=now() WHERE zone_id=$1 AND epoch=$2 AND range_start=$3 AND range_end_exclusive=$5",
        vec![i32::try_from(work.zone.0).map_err(anyhow::Error::from)?.into(),signed(work.epoch)?.into(),signed(start)?.into(),signed(next)?.into(),signed(end)?.into()]).await?;
    if rows != 1 {
        return Err(CheckpointError::Constraint("item_checkpoint_reservation".into()));
    }
    Ok(())
}

async fn persist_rows(
    tx: &DatabaseTransaction,
    character: CharacterId,
    after: &Phase4Checkpoint,
    before: Option<&Phase4Checkpoint>,
    work: &ItemCheckpointWork,
) -> Result<(), CheckpointError> {
    // Also audit IDs allocated and destroyed within this one tick: they cannot be reused.
    for delta in &work.projection.ledger {
        if delta.before.is_none()
            && !before.is_some_and(|cp| cp.instances.iter().any(|item| item.id == delta.instance))
        {
            let exists = tx
                .query_one_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT instance_id FROM item_ledger WHERE instance_id=$1 LIMIT 1",
                    [signed(delta.instance.get())?.into()],
                ))
                .await
                .map_err(super::character_repository::checkpoint_db_error)?;
            if exists.is_some() {
                return Err(CheckpointError::Constraint("item_instance_identity_reused".into()));
            }
        }
    }
    for item in before.into_iter().flat_map(|cp| &cp.instances) {
        if !after
            .instances
            .iter()
            .any(|retained| retained.id == item.id)
        {
            let deleted = execute(
                tx,
                "DELETE FROM item_instances WHERE character_id=$1 AND id=$2",
                vec![character.as_uuid().into(), signed(item.id.get())?.into()],
            )
            .await?;
            if deleted != 1 {
                return Err(CheckpointError::Constraint("item_checkpoint_missing_scalar".into()));
            }
        }
    }
    // Clear slots before ordered upserts so swaps/displacements never hit the partial unique index.
    execute(tx, "UPDATE item_instances SET location='inventory',slot=NULL WHERE character_id=$1 AND location='paperdoll'", vec![character.as_uuid().into()]).await?;
    for item in &after.instances {
        let location = match item.location {
            ItemLocation::Inventory => "inventory",
            ItemLocation::Paperdoll => "paperdoll",
        };
        let written = execute(tx, "INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,bound,created_tick,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,false,$10,$11) ON CONFLICT(id) DO UPDATE SET location=EXCLUDED.location,slot=EXCLUDED.slot,template_id=EXCLUDED.template_id,count=EXCLUDED.count,enchant=EXCLUDED.enchant,augment_id=EXCLUDED.augment_id,charge_bits=EXCLUDED.charge_bits,created_tick=EXCLUDED.created_tick,revision=EXCLUDED.revision,updated_at=now() WHERE item_instances.character_id=EXCLUDED.character_id",
            vec![signed(item.id.get())?.into(),character.as_uuid().into(),location.into(),item.slot.map(|slot| i16::from(slot as u8)).into(),
                i32::try_from(item.template_id).map_err(anyhow::Error::from)?.into(),signed(item.count)?.into(),i64::from(item.enchant).into(),
                item.augment_id.map(i64::from).into(),i16::from(item.charges).into(),signed(item.created_tick.0)?.into(),signed(item.revision)?.into()]).await?;
        if written != 1 {
            return Err(CheckpointError::Constraint("item_instance_owner_immutable".into()));
        }
    }
    Ok(())
}

async fn complete(
    tx: &DatabaseTransaction,
    request: &PendingItemRequest,
    revision: u64,
    receipt: uuid::Uuid,
) -> Result<(), CheckpointError> {
    let payload = match &request.manifest {
        PendingItemManifest::Kit(kit) => serde_json::to_value(kit),
    }
    .map_err(anyhow::Error::from)?;
    let hash = request
        .manifest_hash
        .iter()
        .fold(String::new(), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        });
    let rows = execute(tx,"UPDATE pending_item_requests SET status='materialized',materialized_revision=$3,receipt_ref=$4,updated_at=now() WHERE request_id=$1 AND character_id=$2 AND status='pending' AND manifest_version=$5 AND manifest_hash=$6 AND payload=$7 AND request_fingerprint=$8 AND account_id=$9 AND operation=$10",
        vec![request.request_id.into(),request.character_id.as_uuid().into(),signed(revision)?.into(),receipt.into(),
            i32::try_from(request.manifest_version).map_err(anyhow::Error::from)?.into(),hash.into(),payload.into(),request.fingerprint.clone().into(),request.account_id.as_uuid().into(),request.operation.as_str().into()]).await?;
    if rows != 1 {
        return Err(CheckpointError::Constraint("pending_item_completion_fence".into()));
    }
    Ok(())
}
fn signed(value: u64) -> Result<i64, CheckpointError> {
    Ok(i64::try_from(value).map_err(anyhow::Error::from)?)
}
async fn execute(
    tx: &DatabaseTransaction,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<u64, CheckpointError> {
    Ok(tx
        .execute_raw(Statement::from_sql_and_values(DbBackend::Postgres, sql, values))
        .await
        .map_err(super::character_repository::checkpoint_db_error)?
        .rows_affected())
}
