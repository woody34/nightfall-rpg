//! Recovery baselines and durable epoch discovery, independent of `JetStream` retention.

use std::future::Future;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    QueryFilter, Statement, TransactionTrait,
};

use super::entities::zone_snapshots;
use crate::application::replay_log::{Seq, ZoneSnapshotRow, ZoneSnapshotStore};
use crate::domain::zone::ZoneId;
use crate::infrastructure::telemetry::Metrics;

/// Zone epoch index in Postgres. Baseline and first-sequence writes update both tables
/// transactionally; progress and closure updates are single atomic statements.
#[derive(Clone)]
pub struct PgZoneSnapshotStore {
    db: DatabaseConnection,
    metrics: Option<Metrics>,
}

impl PgZoneSnapshotStore {
    /// Wraps a connection.
    #[must_use]
    pub fn new(db: impl Into<DatabaseConnection>) -> Self {
        Self {
            db: db.into(),
            metrics: None,
        }
    }

    /// Records every query in `db_query_seconds{repo="zone_snapshot",op}`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn timed<T>(&self, op: &'static str, fut: impl Future<Output = T>) -> T {
        match &self.metrics {
            Some(m) => m.time_db("zone_snapshot", op, fut).await,
            None => fut.await,
        }
    }
}

fn zone_col(zone: ZoneId) -> anyhow::Result<i32> {
    Ok(i32::try_from(zone.0)?)
}

fn epoch_col(epoch: u64) -> anyhow::Result<i64> {
    Ok(i64::try_from(epoch)?)
}

fn seq_col(seq: Seq) -> anyhow::Result<i64> {
    Ok(i64::try_from(seq.0)?)
}

fn model_to_row(m: zone_snapshots::Model) -> anyhow::Result<ZoneSnapshotRow> {
    let seq = |v: i64| -> anyhow::Result<Seq> { Ok(Seq(u64::try_from(v)?)) };
    Ok(ZoneSnapshotRow {
        zone: ZoneId(u32::try_from(m.zone_id)?),
        epoch: u64::try_from(m.epoch)?,
        snapshot: m.snapshot,
        snapshot_seq: seq(m.jetstream_snapshot_seq)?,
        first_seq: m.jetstream_first_seq.map(seq).transpose()?,
        time_origin_ms: m.time_origin_ms,
        build_id: m.build_id,
        config_hash: m.config_hash,
        schema_version: u32::try_from(m.schema_version)?,
    })
}

#[async_trait]
impl ZoneSnapshotStore for PgZoneSnapshotStore {
    async fn insert(&self, row: &ZoneSnapshotRow) -> anyhow::Result<()> {
        let tx = self.db.begin().await?;
        tx.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO zone_epochs (zone_id, epoch) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            [zone_col(row.zone)?.into(), epoch_col(row.epoch)?.into()],
        ))
        .await?;
        let now = Utc::now().fixed_offset();
        let model = zone_snapshots::ActiveModel {
            zone_id: ActiveValue::Set(zone_col(row.zone)?),
            epoch: ActiveValue::Set(epoch_col(row.epoch)?),
            snapshot: ActiveValue::Set(row.snapshot.clone()),
            jetstream_snapshot_seq: ActiveValue::Set(seq_col(row.snapshot_seq)?),
            jetstream_first_seq: ActiveValue::Set(row.first_seq.map(seq_col).transpose()?),
            time_origin_ms: ActiveValue::Set(row.time_origin_ms),
            build_id: ActiveValue::Set(row.build_id.clone()),
            config_hash: ActiveValue::Set(row.config_hash.clone()),
            schema_version: ActiveValue::Set(i32::try_from(row.schema_version)?),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        };
        self.timed(
            "insert",
            zone_snapshots::Entity::insert(model)
                .on_conflict(
                    OnConflict::columns([
                        zone_snapshots::Column::ZoneId,
                        zone_snapshots::Column::Epoch,
                    ])
                    .update_columns([
                        zone_snapshots::Column::Snapshot,
                        zone_snapshots::Column::JetstreamSnapshotSeq,
                        zone_snapshots::Column::SchemaVersion,
                        zone_snapshots::Column::UpdatedAt,
                    ])
                    .to_owned(),
                )
                .try_insert()
                .exec(&tx),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn set_first_seq(&self, zone: ZoneId, epoch: u64, seq: Seq) -> anyhow::Result<()> {
        let tx = self.db.begin().await?;
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "UPDATE zone_epochs SET first_seq = COALESCE(first_seq, $3) WHERE zone_id = $1 AND epoch = $2",
            [zone_col(zone)?.into(), epoch_col(epoch)?.into(), seq_col(seq)?.into()])).await?;
        self.timed(
            "set_first_seq",
            zone_snapshots::Entity::update_many()
                .col_expr(zone_snapshots::Column::JetstreamFirstSeq, Expr::value(seq_col(seq)?))
                .col_expr(zone_snapshots::Column::UpdatedAt, Expr::current_timestamp())
                .filter(zone_snapshots::Column::ZoneId.eq(zone_col(zone)?))
                .filter(zone_snapshots::Column::Epoch.eq(epoch_col(epoch)?))
                .exec(&tx),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn get(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<Option<ZoneSnapshotRow>> {
        self.timed(
            "get",
            zone_snapshots::Entity::find_by_id((zone_col(zone)?, epoch_col(epoch)?)).one(&self.db),
        )
        .await?
        .map(model_to_row)
        .transpose()
    }

    async fn latest_epoch(&self, zone: ZoneId) -> anyhow::Result<Option<u64>> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT MAX(epoch) AS epoch FROM zone_epochs WHERE zone_id = $1",
                [zone_col(zone)?.into()],
            ))
            .await?;
        let epoch: Option<i64> = row.map(|r| r.try_get("", "epoch")).transpose()?.flatten();
        Ok(epoch.map(u64::try_from).transpose()?)
    }

    async fn unresolved(
        &self,
        zone: ZoneId,
    ) -> anyhow::Result<Vec<crate::application::replay_log::RecoveryEpoch>> {
        self.db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "SELECT epoch, last_recorded_tick FROM zone_epochs WHERE zone_id = $1 AND closed_at IS NULL ORDER BY epoch", [zone_col(zone)?.into()])).await?
            .into_iter().map(|row| {
                let epoch: i64 = row.try_get("", "epoch")?;
                let tick: Option<i64> = row.try_get("", "last_recorded_tick")?;
                Ok(crate::application::replay_log::RecoveryEpoch { epoch: u64::try_from(epoch)?, last_recorded_tick: tick.map(|t| u64::try_from(t).map(crate::domain::zone::Tick)).transpose()? })
            }).collect()
    }

    async fn recording(
        &self,
        zone: ZoneId,
        epoch: u64,
        tick: crate::domain::zone::Tick,
    ) -> anyhow::Result<()> {
        self.db.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "UPDATE zone_epochs SET last_recorded_tick = GREATEST(last_recorded_tick, $3) WHERE zone_id = $1 AND epoch = $2",
            [zone_col(zone)?.into(), epoch_col(epoch)?.into(), i64::try_from(tick.0)?.into()])).await?;
        Ok(())
    }

    async fn checkpointed(
        &self,
        zone: ZoneId,
        epoch: u64,
        tick: crate::domain::zone::Tick,
    ) -> anyhow::Result<()> {
        self.db.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "UPDATE zone_epochs SET last_checkpointed_tick = GREATEST(last_checkpointed_tick, $3) WHERE zone_id = $1 AND epoch = $2",
            [zone_col(zone)?.into(), epoch_col(epoch)?.into(), i64::try_from(tick.0)?.into()])).await?;
        Ok(())
    }

    async fn close(&self, zone: ZoneId, epoch: u64) -> anyhow::Result<()> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE zone_epochs SET closed_at = now() WHERE zone_id = $1 AND epoch = $2",
                [zone_col(zone)?.into(), epoch_col(epoch)?.into()],
            ))
            .await?;
        Ok(())
    }
}
