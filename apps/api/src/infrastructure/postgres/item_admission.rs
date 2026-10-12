//! Durable allocator reservations and immutable pending manifests via `SeaORM` transactions.
use super::{character_repository::checkpoint_db_error, PgCharacterRepository};
use crate::application::ports::{
    ItemAdmissionRepository, ItemIdRange, PendingItemKind, PendingItemManifest, PendingItemRequest,
    PendingItemStatus,
};
use crate::application::CheckpointError;
use crate::domain::{zone::ZoneId, CharacterId};
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement, TransactionTrait};

// Design §1.1/I1: production block size, fixture block excluded. No combat RNG/DB sequence.
const RANGE_WIDTH: u64 = 1_u64 << 32;
// "NFITEMID": shared by all API allocator writers, independent of zone/epoch.
const RANGE_LOCK: i64 = 0x4e46_4954_454d_4944;

#[async_trait]
impl ItemAdmissionRepository for PgCharacterRepository {
    async fn reserve_item_range(
        &self,
        zone: ZoneId,
        epoch: u64,
    ) -> Result<ItemIdRange, CheckpointError> {
        let signed_zone = i32::try_from(zone.0)
            .map_err(|_| CheckpointError::Constraint("item_id_ranges_zone".into()))?;
        let signed_epoch = i64::try_from(epoch)
            .map_err(|_| CheckpointError::Constraint("item_id_ranges_epoch".into()))?;
        let tx = self.db.begin().await.map_err(checkpoint_db_error)?;
        tx.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock($1)",
            [RANGE_LOCK.into()],
        ))
        .await
        .map_err(checkpoint_db_error)?;
        if let Some(row) = tx.query_one_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "SELECT range_start,range_end_exclusive,next_reserved FROM item_id_ranges WHERE zone_id=$1 AND epoch=$2 ORDER BY range_start DESC LIMIT 1 FOR UPDATE",
            [signed_zone.into(),signed_epoch.into()])).await.map_err(checkpoint_db_error)? {
            let range=decode_range(zone,epoch,&row)?;
            tx.commit().await.map_err(checkpoint_db_error)?;
            return Ok(range);
        }
        let maximum = tx
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT max(range_end_exclusive) AS end FROM item_id_ranges",
            ))
            .await
            .map_err(checkpoint_db_error)?
            .ok_or_else(|| anyhow::anyhow!("allocator aggregate missing"))?;
        let start = maximum
            .try_get::<Option<i64>>("", "end")
            .map_err(anyhow::Error::from)?
            .map(u64::try_from)
            .transpose()
            .map_err(anyhow::Error::from)?
            .unwrap_or(RANGE_WIDTH);
        let end = start
            .checked_add(RANGE_WIDTH)
            .filter(|end| i64::try_from(*end).is_ok())
            .ok_or_else(|| CheckpointError::Constraint("item_id_ranges_exhausted".into()))?;
        let range = ItemIdRange {
            zone_id: zone,
            epoch,
            range_start: start,
            range_end_exclusive: end,
            next_reserved: start,
        };
        range.validate()?;
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "INSERT INTO item_id_ranges(zone_id,epoch,range_start,range_end_exclusive,next_reserved) VALUES($1,$2,$3,$4,$3)",
            [signed_zone.into(),signed_epoch.into(),i64::try_from(start).map_err(anyhow::Error::from)?.into(),i64::try_from(end).map_err(anyhow::Error::from)?.into()]))
            .await.map_err(checkpoint_db_error)?;
        tx.commit().await.map_err(checkpoint_db_error)?;
        Ok(range)
    }

    async fn pending_item_requests(
        &self,
        character: CharacterId,
    ) -> anyhow::Result<Vec<PendingItemRequest>> {
        let tx = self.db.begin().await?;
        lock_character(&tx, character).await?;
        let requests = load_pending(&tx, character).await?;
        tx.commit().await?;
        Ok(requests)
    }
}

fn decode_range(
    zone: ZoneId,
    epoch: u64,
    row: &sea_orm::QueryResult,
) -> Result<ItemIdRange, CheckpointError> {
    let value = |key| -> anyhow::Result<u64> { Ok(u64::try_from(row.try_get::<i64>("", key)?)?) };
    let range = ItemIdRange {
        zone_id: zone,
        epoch,
        range_start: value("range_start")?,
        range_end_exclusive: value("range_end_exclusive")?,
        next_reserved: value("next_reserved")?,
    };
    range.validate()?;
    Ok(range)
}

async fn lock_character(tx: &DatabaseTransaction, character: CharacterId) -> anyhow::Result<()> {
    anyhow::ensure!(
        tx.query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT id FROM characters WHERE id=$1 FOR SHARE",
            [character.as_uuid().into()]
        ))
        .await?
        .is_some(),
        "pending items character missing"
    );
    Ok(())
}

pub(super) async fn load_pending<C: ConnectionTrait>(
    db: &C,
    character: CharacterId,
) -> anyhow::Result<Vec<PendingItemRequest>> {
    let parent = db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT base_class_id,account_id FROM characters WHERE id=$1",
            [character.as_uuid().into()],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("pending items character missing"))?;
    let expected =
        crate::domain::class::ClassId(u32::try_from(parent.try_get::<i32>("", "base_class_id")?)?);
    let owner = crate::domain::AccountId::from_uuid(parent.try_get("", "account_id")?);
    let rows=db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT request_id,account_id,operation,kind,request_fingerprint,manifest_version,manifest_hash,payload,status,materialized_revision,receipt_ref FROM pending_item_requests WHERE character_id=$1 ORDER BY request_id",
        [character.as_uuid().into()])).await?;
    rows.into_iter()
        .map(|row| {
            let request = decode_pending(character, &row)?;
            anyhow::ensure!(
                request.account_id == owner && request.operation == request.kind.operation(),
                "pending request owner/scope mismatch"
            );
            let PendingItemManifest::Kit(kit) = &request.manifest;
            {
                anyhow::ensure!(
                    kit.source_class_id == expected
                        && *kit
                            == crate::infrastructure::item_manifests::starter_manifest(expected)?,
                    "pending manifest differs from pinned creation class kit"
                );
            }
            Ok(request)
        })
        .collect()
}

fn decode_pending(
    character: CharacterId,
    row: &sea_orm::QueryResult,
) -> anyhow::Result<PendingItemRequest> {
    let kind = match row.try_get::<String>("", "kind")?.as_str() {
        "starter" => PendingItemKind::Starter,
        "backfill" => PendingItemKind::Backfill,
        _ => anyhow::bail!("unknown pending item kind"),
    };
    let version = u32::try_from(row.try_get::<i32>("", "manifest_version")?)?;
    anyhow::ensure!(version == 1, "unsupported pending manifest version");
    let hash = parse_hash(&row.try_get::<String>("", "manifest_hash")?)?;
    let payload: serde_json::Value = row.try_get("", "payload")?;
    let kit: crate::application::ports::StarterKitManifest = serde_json::from_value(payload)?;
    anyhow::ensure!(kit.hash()? == hash, "pending kit hash mismatch");
    let manifest = PendingItemManifest::Kit(kit);
    let operation =
        serde_json::from_value(serde_json::Value::String(row.try_get("", "operation")?))?;
    let revision = row.try_get::<Option<i64>>("", "materialized_revision")?;
    let receipt = row.try_get::<Option<uuid::Uuid>>("", "receipt_ref")?;
    let status = match (row.try_get::<String>("", "status")?.as_str(), revision, receipt) {
        ("pending", None, None) => PendingItemStatus::Pending,
        ("materialized", Some(revision), Some(receipt)) => PendingItemStatus::Materialized {
            revision: u64::try_from(revision)?,
            receipt,
        },
        _ => anyhow::bail!("invalid pending request completion"),
    };
    Ok(PendingItemRequest {
        account_id: crate::domain::AccountId::from_uuid(row.try_get("", "account_id")?),
        operation,
        request_id: row.try_get("", "request_id")?,
        character_id: character,
        kind,
        fingerprint: row.try_get("", "request_fingerprint")?,
        manifest_version: version,
        manifest_hash: hash,
        manifest,
        status,
    })
}

pub(super) fn parse_hash(raw: &str) -> anyhow::Result<[u8; 32]> {
    anyhow::ensure!(raw.len() == 64, "invalid manifest hash length");
    let mut hash = [0; 32];
    for (output, pair) in hash.iter_mut().zip(raw.as_bytes().as_chunks::<2>().0) {
        *output = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(hash)
}

/// Same parent creation transaction as character/idempotency/outbox. This allocates no item.
pub(super) async fn create_starter(
    tx: &DatabaseTransaction,
    character: &crate::domain::Character,
) -> anyhow::Result<()> {
    use std::fmt::Write as _;
    let request = crate::infrastructure::item_manifests::starter_request(character)?;
    let PendingItemManifest::Kit(kit) = request.manifest;
    let mut hash = String::with_capacity(64);
    for byte in request.manifest_hash {
        write!(hash, "{byte:02x}")?;
    }
    tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "INSERT INTO pending_item_requests(request_id,character_id,kind,request_fingerprint,manifest_version,manifest_hash,payload,account_id,operation) VALUES($1,$2,'starter',$3,1,$4,$5,$6,'starter')",
        [request.request_id.into(),character.id.as_uuid().into(),request.fingerprint.into(),hash.into(),serde_json::to_value(kit)?.into(),character.account_id.as_uuid().into()])).await?;
    Ok(())
}
