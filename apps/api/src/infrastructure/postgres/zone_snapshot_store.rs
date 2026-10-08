//! `zone_snapshots`: the Postgres index of zone epochs (Story 3.4).

use std::future::Future;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use super::entities::zone_snapshots;
use crate::application::replay_log::{Seq, ZoneSnapshotRow, ZoneSnapshotStore};
use crate::domain::zone::ZoneId;
use crate::infrastructure::telemetry::Metrics;

/// Zone epoch index in Postgres. Each method is one statement, so one atomic unit.
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
                    .do_nothing()
                    .to_owned(),
                )
                .try_insert()
                .exec(&self.db),
        )
        .await?;
        Ok(())
    }

    async fn set_first_seq(&self, zone: ZoneId, epoch: u64, seq: Seq) -> anyhow::Result<()> {
        self.timed(
            "set_first_seq",
            zone_snapshots::Entity::update_many()
                .col_expr(zone_snapshots::Column::JetstreamFirstSeq, Expr::value(seq_col(seq)?))
                .col_expr(zone_snapshots::Column::UpdatedAt, Expr::current_timestamp())
                .filter(zone_snapshots::Column::ZoneId.eq(zone_col(zone)?))
                .filter(zone_snapshots::Column::Epoch.eq(epoch_col(epoch)?))
                .exec(&self.db),
        )
        .await?;
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
        let epoch: Option<i64> = self
            .timed(
                "latest_epoch",
                zone_snapshots::Entity::find()
                    .select_only()
                    .column(zone_snapshots::Column::Epoch)
                    .filter(zone_snapshots::Column::ZoneId.eq(zone_col(zone)?))
                    .order_by_desc(zone_snapshots::Column::Epoch)
                    .limit(1)
                    .into_tuple()
                    .one(&self.db),
            )
            .await?;
        Ok(epoch.map(u64::try_from).transpose()?)
    }
}
