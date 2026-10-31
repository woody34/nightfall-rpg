//! Postgres economy adapter (stories 4.3/4.4; design §10.0, §11.5).
//!
//! Leases: one transaction inserts the whole set in ascending key order with
//! `ON CONFLICT DO NOTHING`; any key that was already held rolls the transaction back and the
//! call returns `Busy` with nothing inserted (the lease table is the only lock, E39). Checkpoint:
//! one transaction applies every container delta as a compare-and-set on `revision` in ascending
//! lease-key order, completes/inserts deliveries, appends owner-kind ledger rows, stages the
//! outbox facts and deletes the lease rows in the same key order. SQLSTATE `40P01`/`40001` are
//! `Transient`; a CAS that touches no row is the permanent `economy_lease_violation` fence; other
//! integrity violations are `Constraint(<name>)`.

use async_trait::async_trait;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, RuntimeErr,
    Statement, TransactionTrait,
};
use uuid::Uuid;

use crate::application::economy::{
    ContainerDelta, DeliveryKind, DeliveryRepository, DeliveryRow, DeltaChange,
    EconomyCheckpointRepository, EconomyCheckpointWrite, EconomyCounterRepository, EconomyError,
    LeaseGrant, LeaseKey, LeaseKind, LeaseRepository,
};

/// `SeaORM`-backed economy repository over the story 4.1 tables.
pub struct PgEconomyRepository {
    db: DatabaseConnection,
}

impl PgEconomyRepository {
    /// Over an existing connection (the same pool as the character repository).
    #[must_use]
    pub const fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

/// Classifies a database error: `40P01`/`40001` transient, `23xxx` constraint, else other.
pub fn classify(error: DbErr) -> EconomyError {
    let (DbErr::Exec(RuntimeErr::SqlxError(sqlx)) | DbErr::Query(RuntimeErr::SqlxError(sqlx))) =
        &error
    else {
        return EconomyError::Other(error.into());
    };
    let Some(db) = sqlx.as_database_error() else {
        return EconomyError::Other(error.into());
    };
    match db.code().as_deref() {
        Some("40P01" | "40001") => EconomyError::Transient(db.message().to_owned()),
        Some(code) if code.starts_with("23") => {
            EconomyError::Constraint(db.constraint().unwrap_or("economy_integrity").to_owned())
        },
        _ => EconomyError::Other(error.into()),
    }
}

fn signed(value: u64) -> Result<i64, EconomyError> {
    i64::try_from(value).map_err(|e| EconomyError::Other(e.into()))
}

async fn execute(
    tx: &DatabaseTransaction,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<u64, EconomyError> {
    Ok(tx
        .execute_raw(Statement::from_sql_and_values(DbBackend::Postgres, sql, values))
        .await
        .map_err(classify)?
        .rows_affected())
}

fn kind_of(name: &str) -> Option<LeaseKind> {
    Some(match name {
        "auction" => LeaseKind::Auction,
        "delivery" => LeaseKind::Delivery,
        "freight" => LeaseKind::Freight,
        "mail" => LeaseKind::Mail,
        "mailbox" => LeaseKind::Mailbox,
        "manor" => LeaseKind::Manor,
        "warehouse" => LeaseKind::Warehouse,
        _ => return None,
    })
}

fn grant_from_row(row: &sea_orm::QueryResult) -> Result<(LeaseKey, LeaseGrant), EconomyError> {
    let other = |e: DbErr| EconomyError::Other(e.into());
    let kind: String = row.try_get("", "owner_kind").map_err(other)?;
    let owner: String = row.try_get("", "owner_id").map_err(other)?;
    let key = LeaseKey {
        kind: kind_of(&kind)
            .ok_or_else(|| EconomyError::Constraint("economy_leases_owner_kind".into()))?,
        owner: owner
            .parse::<u128>()
            .map_err(|_| EconomyError::Constraint("economy_leases_owner_id".into()))?,
    };
    let zone: i32 = row.try_get("", "zone_id").map_err(other)?;
    let epoch: i64 = row.try_get("", "epoch").map_err(other)?;
    Ok((
        key,
        LeaseGrant {
            zone_id: u32::try_from(zone).map_err(|e| EconomyError::Other(e.into()))?,
            epoch: u64::try_from(epoch).map_err(|e| EconomyError::Other(e.into()))?,
            account_id: row.try_get("", "account_id").map_err(other)?,
            operation: row.try_get("", "operation").map_err(other)?,
            request_key: row.try_get("", "request_key").map_err(other)?,
            keys: vec![key],
        },
    ))
}

/// Groups held rows by request key into grants (each with its whole key set).
fn group_grants(rows: Vec<(LeaseKey, LeaseGrant)>) -> Vec<LeaseGrant> {
    let mut grants: Vec<LeaseGrant> = Vec::new();
    for (key, grant) in rows {
        if let Some(existing) = grants
            .iter_mut()
            .find(|g| g.request_key == grant.request_key)
        {
            existing.keys.push(key);
            existing.keys.sort();
        } else {
            grants.push(grant);
        }
    }
    grants
}

#[async_trait]
impl LeaseRepository for PgEconomyRepository {
    async fn acquire(&self, grant: &LeaseGrant) -> Result<(), EconomyError> {
        let tx = self.db.begin().await.map_err(classify)?;
        let mut held = Vec::new();
        for key in &grant.keys {
            // Ascending key order: the set is already canonical (lease_set), checked here.
            let inserted = execute(
                &tx,
                "INSERT INTO economy_leases(owner_kind,owner_id,zone_id,epoch,account_id,operation,request_key) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (owner_kind,owner_id) DO NOTHING",
                vec![
                    key.kind.name().into(),
                    format!("{:039}", key.owner).into(),
                    i32::try_from(grant.zone_id)
                        .map_err(|e| EconomyError::Other(e.into()))?
                        .into(),
                    signed(grant.epoch)?.into(),
                    grant.account_id.into(),
                    grant.operation.clone().into(),
                    grant.request_key.into(),
                ],
            )
            .await?;
            if inserted == 0 {
                held.push(*key);
            }
        }
        if !held.is_empty() {
            tx.rollback().await.map_err(classify)?;
            return Err(EconomyError::Busy(held));
        }
        tx.commit().await.map_err(classify)
    }

    async fn release(
        &self,
        zone_id: u32,
        epoch: u64,
        keys: &[LeaseKey],
    ) -> Result<(), EconomyError> {
        let tx = self.db.begin().await.map_err(classify)?;
        for key in keys {
            execute(
                &tx,
                "DELETE FROM economy_leases WHERE owner_kind=$1 AND owner_id=$2 AND zone_id=$3 AND epoch=$4",
                vec![
                    key.kind.name().into(),
                    format!("{:039}", key.owner).into(),
                    i32::try_from(zone_id)
                        .map_err(|e| EconomyError::Other(e.into()))?
                        .into(),
                    signed(epoch)?.into(),
                ],
            )
            .await?;
        }
        tx.commit().await.map_err(classify)
    }

    async fn release_epoch(
        &self,
        zone_id: u32,
        epoch: u64,
    ) -> Result<Vec<LeaseGrant>, EconomyError> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "DELETE FROM economy_leases WHERE zone_id=$1 AND epoch=$2 RETURNING owner_kind,owner_id,zone_id,epoch,account_id,operation,request_key",
                vec![
                    i32::try_from(zone_id)
                        .map_err(|e| EconomyError::Other(e.into()))?
                        .into(),
                    signed(epoch)?.into(),
                ],
            ))
            .await
            .map_err(classify)?;
        let parsed = rows
            .iter()
            .map(grant_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(group_grants(parsed))
    }

    async fn held(&self, keys: &[LeaseKey]) -> Result<Vec<LeaseGrant>, EconomyError> {
        let mut parsed = Vec::new();
        for key in keys {
            let rows = self
                .db
                .query_all_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "SELECT owner_kind,owner_id,zone_id,epoch,account_id,operation,request_key FROM economy_leases WHERE owner_kind=$1 AND owner_id=$2",
                    vec![key.kind.name().into(), format!("{:039}", key.owner).into()],
                ))
                .await
                .map_err(classify)?;
            for row in &rows {
                parsed.push(grant_from_row(row)?);
            }
        }
        Ok(group_grants(parsed))
    }
}

fn column_i64(
    columns: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<i64, EconomyError> {
    columns
        .get(name)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| EconomyError::Constraint(format!("economy_delta_column_{name}")))
}

fn column_str(
    columns: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<String, EconomyError> {
    columns
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| EconomyError::Constraint(format!("economy_delta_column_{name}")))
}

fn column_uuid(
    columns: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Uuid, EconomyError> {
    column_str(columns, name)?
        .parse()
        .map_err(|_| EconomyError::Constraint(format!("economy_delta_column_{name}")))
}

fn column_opt_uuid(
    columns: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<Option<Uuid>, EconomyError> {
    match columns.get(name) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(_) => column_uuid(columns, name).map(Some),
    }
}

fn column_bool(columns: &serde_json::Map<String, serde_json::Value>, name: &str) -> bool {
    columns
        .get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Applies one delta as insert / CAS update / CAS delete on its row kind. A CAS that changes
/// nothing is the lease violation (a held lease makes it impossible by construction, §10.0 step 3).
#[allow(clippy::too_many_lines)] // One match arm per row kind keeps the CAS shapes visible together.
async fn apply_delta(tx: &DatabaseTransaction, delta: &ContainerDelta) -> Result<(), EconomyError> {
    let c = &delta.columns;
    let expected = delta.expected_revision.map(signed).transpose()?;
    let affected = match (delta.row_kind.as_str(), delta.change) {
        ("item_instance", DeltaChange::Insert) => {
            execute(tx, "INSERT INTO item_instances(id,character_id,location,slot,template_id,count,enchant,augment_id,charge_bits,created_tick,revision,owner_kind,container_id,mail_id,delivery_id) VALUES($1,$2,$3,NULL,$4,$5,$6,$7,$8,$9,0,$10,$11,$12,$13)",
                vec![delta.row_key.parse::<i64>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(),
                     column_opt_uuid(c, "character_id")?.into(), column_str(c, "location")?.into(), column_i64(c, "template_id")?.into(),
                     column_i64(c, "count")?.into(), c.get("enchant").and_then(serde_json::Value::as_i64).unwrap_or(0).into(),
                     c.get("augment_id").and_then(serde_json::Value::as_i64).into(), i16::try_from(c.get("charge_bits").and_then(serde_json::Value::as_i64).unwrap_or(0)).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     column_i64(c, "created_tick")?.into(), column_str(c, "owner_kind")?.into(), column_opt_uuid(c, "container_id")?.into(),
                     column_opt_uuid(c, "mail_id")?.into(), column_opt_uuid(c, "delivery_id")?.into()]).await?
        },
        ("item_instance", DeltaChange::Update) => {
            execute(tx, "UPDATE item_instances SET character_id=$3,location=$4,count=$5,owner_kind=$6,container_id=$7,mail_id=$8,delivery_id=$9,revision=revision+1,updated_at=now() WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<i64>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into(),
                     column_opt_uuid(c, "character_id")?.into(), column_str(c, "location")?.into(), column_i64(c, "count")?.into(),
                     column_str(c, "owner_kind")?.into(), column_opt_uuid(c, "container_id")?.into(), column_opt_uuid(c, "mail_id")?.into(),
                     column_opt_uuid(c, "delivery_id")?.into()]).await?
        },
        ("item_instance", DeltaChange::Delete) => {
            execute(tx, "DELETE FROM item_instances WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<i64>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into()]).await?
        },
        ("mail_message", DeltaChange::Insert) => {
            // expires_at is computed by this writer in the insert (D §11.4): now() + 360 h, or 12 h with COD.
            let cod = column_i64(c, "cod_adena")?;
            execute(tx, "INSERT INTO mail_messages(id,sender_id,receiver_id,subject,body,cod_adena,has_attachments,returned,system_kind,read,expires_at,revision) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,false,now() + make_interval(hours => $10),0)",
                vec![delta.row_key.parse::<Uuid>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(),
                     column_uuid(c, "sender_id")?.into(), column_uuid(c, "receiver_id")?.into(), column_str(c, "subject")?.into(), column_str(c, "body")?.into(),
                     cod.into(), column_bool(c, "has_attachments").into(), column_bool(c, "returned").into(),
                     i16::try_from(c.get("system_kind").and_then(serde_json::Value::as_i64).unwrap_or(0)).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     i32::from(if cod > 0 { 12_i16 } else { 360_i16 }).into()]).await?
        },
        ("mail_message", DeltaChange::Update) => {
            execute(tx, "UPDATE mail_messages SET read=$3,returned=$4,has_attachments=$5,deleted_by_sender=$6,deleted_by_receiver=$7,revision=revision+1 WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<Uuid>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into(),
                     column_bool(c, "read").into(), column_bool(c, "returned").into(), column_bool(c, "has_attachments").into(),
                     column_bool(c, "deleted_by_sender").into(), column_bool(c, "deleted_by_receiver").into()]).await?
        },
        ("mail_message", DeltaChange::Delete) => {
            execute(tx, "DELETE FROM mail_messages WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<Uuid>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into()]).await?
        },
        ("item_auction", DeltaChange::Update) => {
            execute(tx, "UPDATE item_auctions SET state=$3,extend_state=$4,last_bidder=$5,highest_bidder=$6,highest_bid=$7,revision=revision+1 WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<i64>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into(),
                     i16::try_from(column_i64(c, "state")?).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     i16::try_from(c.get("extend_state").and_then(serde_json::Value::as_i64).unwrap_or(0)).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     column_opt_uuid(c, "last_bidder")?.into(), column_opt_uuid(c, "highest_bidder")?.into(),
                     c.get("highest_bid").and_then(serde_json::Value::as_i64).into()]).await?
        },
        ("item_auction", DeltaChange::Delete) => {
            execute(tx, "DELETE FROM item_auctions WHERE id=$1 AND revision=$2",
                vec![delta.row_key.parse::<i64>().map_err(|_| EconomyError::Constraint("economy_delta_row_key".into()))?.into(), expected.into()]).await?
        },
        ("item_auction_bid", DeltaChange::Insert) => {
            execute(tx, "INSERT INTO item_auction_bids(auction_id,character_id,last_bid,revision) VALUES($1,$2,$3,0)",
                vec![column_i64(c, "auction_id")?.into(), column_uuid(c, "character_id")?.into(), column_i64(c, "last_bid")?.into()]).await?
        },
        ("item_auction_bid", DeltaChange::Update) => {
            execute(tx, "UPDATE item_auction_bids SET last_bid=$4,revision=revision+1 WHERE auction_id=$1 AND character_id=$2 AND revision=$3",
                vec![column_i64(c, "auction_id")?.into(), column_uuid(c, "character_id")?.into(), expected.into(), column_i64(c, "last_bid")?.into()]).await?
        },
        ("item_auction_bid", DeltaChange::Delete) => {
            execute(tx, "DELETE FROM item_auction_bids WHERE auction_id=$1 AND character_id=$2 AND revision=$3",
                vec![column_i64(c, "auction_id")?.into(), column_uuid(c, "character_id")?.into(), expected.into()]).await?
        },
        ("manor_seed_production", DeltaChange::Update) => {
            execute(tx, "UPDATE manor_seed_production SET amount=$4,revision=revision+1 WHERE castle_id=$1 AND seed_id=$2 AND revision=$3",
                vec![i16::try_from(column_i64(c, "castle_id")?).map_err(|e| EconomyError::Other(e.into()))?.into(), i32::try_from(column_i64(c, "seed_id")?).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     expected.into(), column_i64(c, "amount")?.into()]).await?
        },
        ("manor_crop_procure", DeltaChange::Update) => {
            execute(tx, "UPDATE manor_crop_procure SET amount=$4,revision=revision+1 WHERE castle_id=$1 AND crop_id=$2 AND revision=$3",
                vec![i16::try_from(column_i64(c, "castle_id")?).map_err(|e| EconomyError::Other(e.into()))?.into(), i32::try_from(column_i64(c, "crop_id")?).map_err(|e| EconomyError::Other(e.into()))?.into(),
                     expected.into(), column_i64(c, "amount")?.into()]).await?
        },
        (kind, change) => {
            return Err(EconomyError::Constraint(format!(
                "economy_delta_unsupported_{kind}_{change:?}"
            )))
        },
    };
    if affected != 1 {
        return Err(EconomyError::LeaseViolation(delta.lease));
    }
    Ok(())
}

#[async_trait]
impl EconomyCheckpointRepository for PgEconomyRepository {
    async fn checkpoint_economy(&self, write: &EconomyCheckpointWrite) -> Result<(), EconomyError> {
        write.validate()?;
        let tx = self.db.begin().await.map_err(classify)?;
        let result = checkpoint_in(&tx, write).await;
        match result {
            Ok(()) => tx.commit().await.map_err(classify),
            Err(error) => {
                tx.rollback().await.map_err(classify)?;
                Err(error)
            },
        }
    }
}

async fn checkpoint_in(
    tx: &DatabaseTransaction,
    write: &EconomyCheckpointWrite,
) -> Result<(), EconomyError> {
    let zone = i32::try_from(write.zone_id).map_err(|e| EconomyError::Other(e.into()))?;
    for delta in write.ordered_deltas() {
        apply_delta(tx, delta).await?;
    }
    for delivery in &write.deliveries {
        execute(tx, "INSERT INTO economy_deliveries(id,target_character,kind,payload,status,revision) VALUES($1,$2,$3,$4,'pending',0)",
            vec![delivery.id.into(), delivery.target_character.into(), delivery.kind.name().into(), delivery.payload.clone().into()]).await?;
    }
    for completion in &write.completions {
        let affected = execute(tx, "UPDATE economy_deliveries SET status='delivered',delivered_zone=$3,delivered_epoch=$4,delivered_tick=$5,revision=revision+1 WHERE id=$1 AND revision=$2 AND status='pending'",
            vec![completion.id.into(), signed(completion.expected_revision)?.into(), i32::try_from(completion.zone_id).map_err(|e| EconomyError::Other(e.into()))?.into(),
                 signed(completion.epoch)?.into(), signed(completion.tick)?.into()]).await?;
        if affected != 1 {
            return Err(EconomyError::LeaseViolation(LeaseKey::of_uuid(
                LeaseKind::Delivery,
                completion.id,
            )));
        }
    }
    for row in &write.ledger {
        let character: Option<Uuid> = (row.owner_kind == "character").then_some(row.owner_id);
        let owner: Option<Uuid> = (row.owner_kind != "character").then_some(row.owner_id);
        execute(tx, "INSERT INTO item_ledger(owner_kind,owner_id,character_id,zone_id,epoch,tick,revision,op,instance_id,operation_ordinal,delta,receipt_ref) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
            vec![row.owner_kind.clone().into(), owner.into(), character.into(), i32::try_from(row.zone_id).map_err(|e| EconomyError::Other(e.into()))?.into(),
                 signed(row.epoch)?.into(), signed(row.tick)?.into(), signed(row.revision)?.into(), row.op.clone().into(), signed(row.instance_id)?.into(),
                 i32::try_from(row.operation_ordinal).map_err(|e| EconomyError::Other(e.into()))?.into(), row.delta.clone().into(), row.receipt_ref.into()]).await?;
    }
    for fact in &write.outbox {
        let mut payload = fact.payload.clone();
        if let serde_json::Value::Object(map) = &mut payload {
            map.insert("fact_id".into(), serde_json::Value::String(fact.fact_id.to_string()));
        }
        execute(
            tx,
            "INSERT INTO outbox(subject,payload) VALUES($1,$2)",
            vec![
                format!("nightfall.item.{}", fact.subject).into(),
                payload.into(),
            ],
        )
        .await?;
    }
    for key in &write.lease_releases {
        execute(tx, "DELETE FROM economy_leases WHERE owner_kind=$1 AND owner_id=$2 AND zone_id=$3 AND epoch=$4",
            vec![key.kind.name().into(), format!("{:039}", key.owner).into(), zone.into(), signed(write.epoch)?.into()]).await?;
    }
    Ok(())
}

#[async_trait]
impl DeliveryRepository for PgEconomyRepository {
    async fn pending(&self, target: Uuid) -> Result<Vec<DeliveryRow>, EconomyError> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id,target_character,kind,payload FROM economy_deliveries WHERE target_character=$1 AND status='pending' ORDER BY id",
                vec![target.into()],
            ))
            .await
            .map_err(classify)?;
        rows.iter()
            .map(|row| {
                let other = |e: DbErr| EconomyError::Other(e.into());
                let kind: String = row.try_get("", "kind").map_err(other)?;
                Ok(DeliveryRow {
                    id: row.try_get("", "id").map_err(other)?,
                    target_character: row.try_get("", "target_character").map_err(other)?,
                    kind: match kind.as_str() {
                        "cod_payment" => DeliveryKind::CodPayment,
                        "mail_expiry_return" => DeliveryKind::MailExpiryReturn,
                        "auction_award" => DeliveryKind::AuctionAward,
                        _ => {
                            return Err(EconomyError::Constraint("economy_deliveries_kind".into()))
                        },
                    },
                    payload: row.try_get("", "payload").map_err(other)?,
                })
            })
            .collect()
    }
}

#[async_trait]
impl EconomyCounterRepository for PgEconomyRepository {
    async fn apply_fact(
        &self,
        fact_id: Uuid,
        day: chrono::NaiveDate,
        counter: &str,
        value: i128,
    ) -> Result<bool, EconomyError> {
        let tx = self.db.begin().await.map_err(classify)?;
        let fresh = execute(&tx, "INSERT INTO economy_counter_facts(fact_id) VALUES($1) ON CONFLICT (fact_id) DO NOTHING", vec![fact_id.into()]).await?;
        if fresh == 1 {
            // numeric(39,0) accepts the i128 range as text.
            execute(&tx, "INSERT INTO economy_counters(day,counter,value) VALUES($1,$2,$3::numeric) ON CONFLICT (day,counter) DO UPDATE SET value = economy_counters.value + EXCLUDED.value",
                vec![day.into(), counter.into(), value.to_string().into()]).await?;
        }
        tx.commit().await.map_err(classify)?;
        Ok(fresh == 1)
    }

    async fn counter(&self, day: chrono::NaiveDate, counter: &str) -> Result<i128, EconomyError> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT value::text AS v FROM economy_counters WHERE day=$1 AND counter=$2",
                vec![day.into(), counter.into()],
            ))
            .await
            .map_err(classify)?;
        row.map_or(Ok(0), |r| {
            let text: String = r
                .try_get("", "v")
                .map_err(|e| EconomyError::Other(e.into()))?;
            text.parse::<i128>()
                .map_err(|e| EconomyError::Other(e.into()))
        })
    }
}
