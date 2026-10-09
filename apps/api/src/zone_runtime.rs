//! Composition of the zone runtime (Stories 3.2, 3.4): picks the replay-log and snapshot-index
//! adapters and starts the fixture zone in a new epoch. Called once from `main`.

use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::application::replay_log::{EventLog, ZoneSnapshotStore};
use crate::application::zone_actor::IntervalTicks;
use crate::application::zone_bootstrap::{RunningZone, ZoneBootstrap};
use crate::infrastructure::eventlog::{InMemoryEventLog, JetStreamEventLog};
use crate::infrastructure::postgres::PgZoneSnapshotStore;
use crate::infrastructure::{rules_data, zone_data};

/// Where the zone comes from.
#[derive(Debug, Clone, Default)]
pub struct ZoneRuntimeConfig {
    /// `ZONE_FILE`: a zone TOML. `None` uses the compiled-in fixture
    /// (`packages/data/zones/test_zone.toml`).
    pub zone_file: Option<PathBuf>,
    /// `RULES_DIR`: a `packages/data` directory holding `tables/` and `classes/`. `None`
    /// uses the compiled-in rule files.
    pub rules_dir: Option<PathBuf>,
}

impl ZoneRuntimeConfig {
    /// Reads `ZONE_FILE`.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            zone_file: std::env::var_os("ZONE_FILE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            rules_dir: std::env::var_os("RULES_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// Starts the zone on a 100 ms interval. The replay log is `JetStream` when `nats_url` is set
/// (in memory otherwise, with a warning); the snapshot index is Postgres when `db` is set.
pub async fn start(
    cfg: &ZoneRuntimeConfig,
    nats_url: Option<&str>,
    db: Option<DatabaseConnection>,
    deps: &crate::Dependencies,
) -> anyhow::Result<RunningZone> {
    let metrics = deps.metrics.clone();
    let clock = deps.clock.clone();
    let def = match &cfg.zone_file {
        Some(path) => zone_data::load_zone(path)?,
        None => zone_data::parse_zone(zone_data::TEST_ZONE_TOML)?,
    };
    // Invalid rules abort startup with every error listed (Story E1.2).
    let rules = match &cfg.rules_dir {
        Some(dir) => rules_data::load_rules_dir(dir)?,
        None => rules_data::load_rules(&rules_data::RulesSource::embedded())?,
    };
    tracing::info!(config_hash = %rules.config_hash, "stat rules loaded");
    let log: Arc<dyn EventLog> = if let Some(url) = nats_url {
        let client = async_nats::connect(url).await?;
        Arc::new(JetStreamEventLog::connect(client, metrics.clone()).await?)
    } else {
        tracing::warn!("NATS_URL not set: zone replay log is in memory (lost on exit)");
        Arc::new(InMemoryEventLog::default())
    };
    let snapshots: Option<Arc<dyn ZoneSnapshotStore>> = db.map(|db| {
        Arc::new(PgZoneSnapshotStore::new(db).with_metrics(metrics.clone()))
            as Arc<dyn ZoneSnapshotStore>
    });
    let mut checkpoints = crate::application::checkpoint::CheckpointService::new(
        deps.characters.clone(),
        deps.audit.clone(),
        Arc::new(metrics.clone()),
    );
    if let Some(store) = &snapshots {
        checkpoints
            .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
            .await?;
        checkpoints = checkpoints.with_durability(log.clone(), store.clone());
    } else if let Some(epoch) = log.latest_epoch(def.zone).await? {
        checkpoints.recover(log.as_ref(), def.zone, epoch).await?;
    }
    let running = ZoneBootstrap::new(log, snapshots, clock, Arc::new(metrics.clone()))
        .with_telemetry(Arc::new(metrics))
        .with_rules(rules)
        .start(&def, IntervalTicks::new())
        .await?;
    running.handle().checkpoints.install(checkpoints);
    Ok(running)
}
