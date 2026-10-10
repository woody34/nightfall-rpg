//! Composition of the zone runtime (Stories 3.2, 3.4): picks the replay-log and snapshot-index
//! adapters and starts the fixture zone in a new epoch. Called once from `main`.

use std::path::PathBuf;
use std::sync::Arc;

use sea_orm::DatabaseConnection;
use serde::Deserialize;

use crate::application::replay_log::{EventLog, ZoneSnapshotStore};
use crate::application::zone_actor::IntervalTicks;
use crate::application::zone_bootstrap::{RunningZone, ZoneBootstrap, ZoneDefinition};
use crate::domain::zone::Speed;
use crate::infrastructure::data_hash::DataHash;
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
    /// `ZONE_SIM_FIXTURE`: explicitly selects a simulation-only fixture. Currently accepts
    /// `phase1a-social-aggro`; cannot be combined with `ZONE_FILE`.
    pub simulation_fixture: Option<String>,
    /// Simulation fixtures require the same exact `AUTH_DEV_TOKENS=1` opt-in as dev auth.
    pub auth_dev_tokens: bool,
}

impl ZoneRuntimeConfig {
    /// Reads `ZONE_FILE`, `RULES_DIR`, `ZONE_SIM_FIXTURE` and `AUTH_DEV_TOKENS`.
    #[must_use]
    pub fn from_env() -> Self {
        Self::read_env(|name| std::env::var_os(name))
    }

    fn read_env(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Self {
        Self {
            zone_file: get("ZONE_FILE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            rules_dir: get("RULES_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            // Even empty or non-Unicode selections are refused, never silently defaulted.
            simulation_fixture: get("ZONE_SIM_FIXTURE").map(|v| v.to_string_lossy().into_owned()),
            auth_dev_tokens: get("AUTH_DEV_TOKENS").is_some_and(|v| v == "1"),
        }
    }

    fn load_zone(&self) -> anyhow::Result<ZoneDefinition> {
        if let Some(name) = &self.simulation_fixture {
            anyhow::ensure!(self.auth_dev_tokens, "ZONE_SIM_FIXTURE requires AUTH_DEV_TOKENS=1");
            anyhow::ensure!(self.zone_file.is_none(), "ZONE_SIM_FIXTURE conflicts with ZONE_FILE");
            anyhow::ensure!(
                name == "phase1a-social-aggro",
                "unknown ZONE_SIM_FIXTURE {name:?}; expected phase1a-social-aggro"
            );
            tracing::warn!(fixture = name, "simulation-only zone fixture selected");
            return social_aggro_fixture();
        }
        match &self.zone_file {
            Some(path) => zone_data::load_zone(path),
            None => zone_data::parse_zone(zone_data::TEST_ZONE_TOML),
        }
    }
}

const SOCIAL_ZONE: &str = include_str!("../fixtures/phase1a-social-aggro/zones/social_aggro.toml");
const SOCIAL_NPC: &str = include_str!("../fixtures/phase1a-social-aggro/npcs/social_aggro.toml");
const SOCIAL_OVERRIDES: &str = include_str!("../fixtures/phase1a-social-aggro/fixture.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulationOverrides {
    npc_speed: u32,
}

fn social_aggro_fixture() -> anyhow::Result<ZoneDefinition> {
    let overrides: SimulationOverrides = toml::from_str(SOCIAL_OVERRIDES)?;
    let mut def = zone_data::parse_zone_with(SOCIAL_ZONE, &[("social_aggro", SOCIAL_NPC)])?;
    // Core template validation requires positive movement speed. Root only this explicit
    // simulation fixture before bootstrap resolves its real spawn-slot combat/AI profiles.
    for template in &mut def.npc_templates {
        template.move_speed = Speed::from_milli_tiles_per_tick(overrides.npc_speed);
    }
    def.config_hash = DataHash::new()
        .part("fixture", SOCIAL_OVERRIDES.as_bytes())
        .part("zone_and_templates", def.config_hash.as_bytes())
        .finish();
    Ok(def)
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
    let def = cfg.load_zone()?;
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
    let classes = match &deps.classes {
        Some(classes) => classes.clone(),
        None => crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )?,
    };
    let running = ZoneBootstrap::new(log, snapshots, clock, Arc::new(metrics.clone()))
        .with_telemetry(Arc::new(metrics))
        .with_rules(rules)
        .with_classes(classes.registry, classes.config_hash)
        .start(&def, IntervalTicks::new())
        .await?;
    running.handle().checkpoints.install(checkpoints);
    Ok(running)
}

#[cfg(test)]
#[path = "zone_runtime_tests.rs"]
mod tests;
