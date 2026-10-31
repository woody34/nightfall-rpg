//! Composition of the zone runtime (Stories 3.2, 3.4): picks the replay-log and snapshot-index
//! adapters and starts the fixture zone in a new epoch. Called once from `main`.

mod phase3;
pub use phase3::{prepare_phase3_bootstrap, PreparedPhase3Bootstrap};

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

/// (`RULES_DIR`, `ZONE_FILE`, `ZONE_SIM_FIXTURE`, classes hash) of one prepared V6 bundle.
type PreparedKey = (Option<PathBuf>, Option<PathBuf>, Option<String>, String);
/// V6 bundles composed and validated by `prepare_phase4`, once per configured source.
static PREPARED: parking_lot::Mutex<
    std::collections::BTreeMap<PreparedKey, crate::infrastructure::item_data::Phase4RuleBundle>,
> = parking_lot::Mutex::new(std::collections::BTreeMap::new());

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
    /// `PHASE3_ENABLED`: normal startup defaults to the validated V5 bundle. Exact0
    /// selects legacy new epochs; exact1 explicitly requests V5. Simulation fixtures
    /// retain legacy policy unless explicitly selected. Malformed values refuse startup.
    pub phase3_enabled: Option<String>,
    /// `PHASE4_ENABLED` (D27): unset or exact `0` keeps V5 new epochs (legacy replay
    /// unchanged); exact `1` boots every new epoch as V6 with the embedded, hash-referenced
    /// Phase 4 item and skill bundle. V6 extends V5, so it requires the V5 selection.
    /// Malformed values refuse startup.
    pub phase4_enabled: Option<String>,
    /// `PHASE5_ENABLED` (design §12.1): unset selects V7 new epochs, except under a historical
    /// simulation-fixture selection; exact `0` keeps the V6-and-older selection; exact `1`
    /// selects V7. Any other value refuses startup. The recorded version always wins on
    /// replay. V7 extends V6, so it refuses an explicit `PHASE4_ENABLED=0`/`PHASE3_ENABLED=0`.
    pub phase5_enabled: Option<String>,
}

impl ZoneRuntimeConfig {
    /// Reads `ZONE_FILE`, `RULES_DIR`, `ZONE_SIM_FIXTURE`, `AUTH_DEV_TOKENS` and
    /// the fail-closed `PHASE3_ENABLED` selection.
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
            phase3_enabled: get("PHASE3_ENABLED").map(|v| v.to_string_lossy().into_owned()),
            phase4_enabled: get("PHASE4_ENABLED").map(|v| v.to_string_lossy().into_owned()),
            phase5_enabled: get("PHASE5_ENABLED").map(|v| v.to_string_lossy().into_owned()),
        }
    }

    fn phase3_requested(&self) -> anyhow::Result<bool> {
        match self.phase3_enabled.as_deref() {
            None => Ok(self.simulation_fixture.is_none()),
            Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(_) => anyhow::bail!("PHASE3_ENABLED must be unset, 0 or 1"),
        }
    }

    fn phase4_requested(&self) -> anyhow::Result<bool> {
        match self.phase4_enabled.as_deref() {
            None | Some("0") => Ok(false),
            Some("1") => {
                if !self.phase3_requested()? {
                    if self.phase3_enabled.is_none() {
                        anyhow::bail!(
                            "PHASE4_ENABLED=1 extends the V5 bundle, but ZONE_SIM_FIXTURE keeps \
                             legacy epochs by default; set PHASE3_ENABLED=1 to select V5"
                        );
                    }
                    anyhow::bail!(
                        "PHASE4_ENABLED=1 extends the V5 bundle; PHASE3_ENABLED must be unset or 1"
                    );
                }
                Ok(true)
            },
            Some(_) => anyhow::bail!("PHASE4_ENABLED must be unset, 0 or 1"),
        }
    }

    /// The `PHASE5_ENABLED` matrix (design §12.1): unset → V7 unless a simulation fixture
    /// keeps its historical epochs; `0` → the older selection; `1` → V7; anything else
    /// refuses boot. V7 extends V6 and V5, so an explicit `0` on either older switch is a
    /// contradiction and refuses boot too. The V7 bundle itself (Phase 5 rules and block)
    /// is composed by the engine stories; this selection is what they consult.
    pub fn phase5_requested(&self) -> anyhow::Result<bool> {
        let requested = match self.phase5_enabled.as_deref() {
            None => self.simulation_fixture.is_none(),
            Some("0") => false,
            Some("1") => true,
            Some(_) => anyhow::bail!("PHASE5_ENABLED must be unset, 0 or 1"),
        };
        if requested && self.phase5_enabled.is_some() {
            anyhow::ensure!(
                self.phase4_enabled.as_deref() != Some("0"),
                "PHASE5_ENABLED=1 extends the V6 bundle; PHASE4_ENABLED must be unset or 1"
            );
            anyhow::ensure!(
                self.phase3_enabled.as_deref() != Some("0"),
                "PHASE5_ENABLED=1 extends the V5 bundle; PHASE3_ENABLED must be unset or 1"
            );
            if self.simulation_fixture.is_some() && self.phase3_enabled.is_none() {
                anyhow::bail!(
                    "PHASE5_ENABLED=1 extends the V5 bundle, but ZONE_SIM_FIXTURE keeps legacy \
                     epochs by default; set PHASE3_ENABLED=1 to select V5"
                );
            }
        }
        Ok(requested)
    }

    /// The Phase 5 rules a V7 epoch is bound to (design §1.10, D27): the embedded tables
    /// validated and hashed against the V6 bundle this process composed. `None` when V7 is
    /// not selected, or when it is selected only by default without a V6 bundle (the epoch
    /// version itself is chosen by the engine stories); an explicit `PHASE5_ENABLED=1`
    /// without `PHASE4_ENABLED=1` refuses boot, as does any table validation failure.
    pub fn prepare_phase5(
        &self,
        phase4: Option<&crate::infrastructure::item_data::Phase4RuleBundle>,
    ) -> anyhow::Result<Option<Arc<crate::domain::zone::Phase5Rules>>> {
        if !self.phase5_requested()? {
            return Ok(None);
        }
        let Some(bundle) = phase4 else {
            anyhow::ensure!(
                self.phase5_enabled.is_none(),
                "PHASE5_ENABLED=1 requires the V6 bundle: set PHASE4_ENABLED=1"
            );
            return Ok(None);
        };
        let rules = crate::infrastructure::economy_data::phase5_rules_for(&bundle.items)
            .map_err(anyhow::Error::msg)?;
        tracing::info!(
            phase5_rules_hash = %format_args!("sha256:{}", rules.hash_hex()),
            global_multiplier = rules.global_multiplier_string(),
            "Phase 5 rules validated and bound to the V6 bundle"
        );
        Ok(Some(rules))
    }

    /// Validates the compiled epoch selection before adapter connections and recovery.
    /// Full bundle validation remains required whenever V5 or V6 is selected.
    pub fn validate_production_selection(&self) -> anyhow::Result<()> {
        self.phase3_requested()?;
        self.phase4_requested()?;
        self.phase5_requested().map(|_| ())
    }

    /// Loads and validates the V6 bundle (D27: embedded tables, referenced by hash) before
    /// adapters connect or epochs recover. `None` keeps V5 new epochs. The returned skills
    /// replace the V5 skill bundle for persistence validators and the zone.
    pub fn prepare_phase4(
        &self,
        classes: &crate::infrastructure::class_data::ResolvedClasses,
    ) -> anyhow::Result<Option<crate::infrastructure::item_data::Phase4RuleBundle>> {
        if !self.phase4_requested()? {
            return Ok(None);
        }
        // Composed and validated once per configured source; the composition root and the
        // zone start ask for the same bundle.
        let key = (
            self.rules_dir.clone(),
            self.zone_file.clone(),
            self.simulation_fixture.clone(),
            classes.config_hash.clone(),
        );
        if let Some(bundle) = PREPARED.lock().get(&key) {
            return Ok(Some(bundle.clone()));
        }
        // Same Phase 3 source as the V5 path (RULES_DIR overlay, simulation-fixture NPC
        // overrides); the embedded default resolves to the embedded D27 bundle itself.
        let bundle =
            crate::infrastructure::item_data::configured_phase4_bundle(&self.phase3_source()?)
                .map_err(anyhow::Error::msg)?;
        let rules = match &self.rules_dir {
            Some(dir) => rules_data::load_rules_dir(dir)?,
            None => rules_data::load_rules(&rules_data::RulesSource::embedded())?,
        };
        let zone = self.load_zone()?;
        // The V6 skills pass every V5 bootstrap check against this installation; the item
        // and skill agreement was validated when the bundle was composed.
        phase3::validate_phase3_rules(&bundle.skills, &rules, classes, &zone)?;
        let blacksmith = &bundle.items.parts().capacities.blacksmith;
        anyhow::ensure!(
            zone.bounds
                .contains(crate::domain::zone::Vec2Fixed::from_tiles(blacksmith.x, blacksmith.y)),
            "Phase4 blacksmith is outside zone bounds"
        );
        let hex = |hash: [u8; 32]| {
            let mut text = "sha256:".to_owned();
            for byte in hash {
                let _ = std::fmt::Write::write_fmt(&mut text, format_args!("{byte:02x}"));
            }
            text
        };
        tracing::info!(phase4_item_rules_hash = %hex(bundle.items.hash()),
            phase4_skill_rules_hash = %hex(bundle.skills.hash()),
            physical_rules_hash = %rules.config_hash,
            classes_hash = %classes.config_hash,
            zone_config_hash = %zone.config_hash,
            "Phase3 bootstrap bundle validated with the Phase4 (V6) item extension");
        PREPARED.lock().insert(key, bundle.clone());
        Ok(Some(bundle))
    }

    /// Loads the complete V5 bundle before adapters connect or epochs recover. The
    /// returned immutable rules can be shared with character persistence validators.
    pub fn prepare_phase3(
        &self,
        classes: crate::infrastructure::class_data::ResolvedClasses,
    ) -> anyhow::Result<Option<PreparedPhase3Bootstrap>> {
        if !self.phase3_requested()? {
            return Ok(None);
        }
        let source = self.phase3_source()?;
        let rules = match &self.rules_dir {
            Some(dir) => rules_data::load_rules_dir(dir)?,
            None => rules_data::load_rules(&rules_data::RulesSource::embedded())?,
        };
        Ok(Some(prepare_phase3_bootstrap(&source, rules, classes, self.load_zone()?)?))
    }

    /// T18: the configured Phase 3 source that replay tools register (lazily composed) so they
    /// resolve the V6 bundle of an epoch this configuration recorded: the `RULES_DIR` overlay
    /// and the `ZONE_SIM_FIXTURE` NPC overrides, composed exactly as [`Self::prepare_phase4`]
    /// composes them. `None` when only the embedded bundle can be recorded.
    pub fn phase4_replay_source(
        &self,
    ) -> anyhow::Result<Option<crate::infrastructure::skill_data::Phase3DataSource>> {
        if let Some(name) = &self.simulation_fixture {
            anyhow::ensure!(
                name == "phase1a-social-aggro",
                "unknown ZONE_SIM_FIXTURE {name:?}; expected phase1a-social-aggro"
            );
        } else if self.rules_dir.is_none() {
            return Ok(None);
        }
        self.phase3_source().map(Some)
    }

    fn phase3_source(&self) -> anyhow::Result<crate::infrastructure::skill_data::Phase3DataSource> {
        let mut source = match &self.rules_dir {
            Some(root) => crate::infrastructure::skill_data::Phase3DataSource::from_dir(root)?,
            None => crate::infrastructure::skill_data::Phase3DataSource::embedded(),
        };
        if self.simulation_fixture.as_deref() == Some("phase1a-social-aggro") {
            // Same explicit fixture override as social_aggro_fixture; never relax
            // production template agreement or silently copy arbitrary zone values.
            let overrides: SimulationOverrides = toml::from_str(SOCIAL_OVERRIDES)?;
            source.profiles.npc_speed_overrides.insert(
                "social_aggro".into(),
                Speed::from_milli_tiles_per_tick(overrides.npc_speed),
            );
        }
        Ok(source)
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
    // A PostgreSQL epoch must never depend on volatile history: after a crash its
    // unresolved checkpoint prefix could not be recovered, even if a grant committed.
    anyhow::ensure!(
        db.is_none() || nats_url.is_some_and(|url| !url.trim().is_empty()),
        "persistent character runtime requires NATS_URL for durable replay before startup"
    );
    let phase3_requested = cfg.phase3_requested()?;
    let phase4 = match &deps.classes {
        Some(classes) => cfg.prepare_phase4(classes)?,
        None => cfg.prepare_phase4(&crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )?)?,
    };
    // E2.3: a V7 selection validates and binds the Phase 5 rules before anything connects;
    // the engine stories consume the bound rules when they open V7 epochs.
    let _phase5 = cfg.prepare_phase5(phase4.as_ref())?;
    let metrics = deps.metrics.clone();
    let clock = deps.clock.clone();
    let def = cfg.load_zone()?;
    // Invalid rules abort startup with every error listed (Story E1.2).
    let rules = match &cfg.rules_dir {
        Some(dir) => rules_data::load_rules_dir(dir)?,
        None => rules_data::load_rules(&rules_data::RulesSource::embedded())?,
    };
    tracing::info!(config_hash = %rules.config_hash, "stat rules loaded");
    let classes = match &deps.classes {
        Some(classes) => classes.clone(),
        None => crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )?,
    };
    let phase3 = if let Some(bundle) = &phase4 {
        anyhow::ensure!(
            deps.learning.is_some() && deps.item_checkpoints.is_some() && deps.item_admission.is_some(),
            "Phase4 requires atomic learning, item checkpoint and item range persistence before recovery"
        );
        Some((*bundle.skills).clone())
    } else if phase3_requested {
        let source = cfg.phase3_source()?;
        anyhow::ensure!(
            deps.learning.is_some(),
            "Phase3 requires atomic learning persistence before recovery"
        );
        Some(prepare_phase3_bootstrap(&source, rules.clone(), classes.clone(), def.clone())?.phase3)
    } else {
        None
    };
    cfg.validate_production_selection()?;
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
    if let Some(learning) = &deps.learning {
        checkpoints = checkpoints.with_learning(learning.clone());
    }
    // Recovered V6 epochs and new V6 work settle through the same atomic item lane.
    if let Some(items) = &deps.item_checkpoints {
        checkpoints = checkpoints.with_items(items.clone());
    }
    if let Some(reservations) = &deps.item_admission {
        checkpoints = checkpoints.with_item_admission(reservations.clone());
    }
    if let Some(store) = &snapshots {
        checkpoints
            .recover_indexed(log.as_ref(), store.as_ref(), def.zone)
            .await?;
        checkpoints = checkpoints.with_durability(log.clone(), store.clone());
    } else if let Some(epoch) = log.latest_epoch(def.zone).await? {
        checkpoints.recover(log.as_ref(), def.zone, epoch).await?;
    }
    log_quarantined_lanes(deps.characters.as_ref(), def.zone).await;
    let mut bootstrap = ZoneBootstrap::new(log, snapshots, clock, Arc::new(metrics.clone()))
        .with_telemetry(Arc::new(metrics))
        .with_rules(rules)
        .with_classes(classes.registry, classes.config_hash);
    if let Some(phase3) = phase3 {
        bootstrap = bootstrap.with_phase3_rules(phase3);
    }
    if let (Some(bundle), Some(reservations)) = (phase4, &deps.item_admission) {
        // Every new epoch of this run boots V6 with fresh durable instance reservations.
        bootstrap = bootstrap.with_phase4_rules(bundle.items, reservations.clone());
    }
    let running = bootstrap.start(&def, IntervalTicks::new()).await?;
    running.handle().checkpoints.install(checkpoints);
    Ok(running)
}

/// T11 boot summary: after recovery, one line per zone with the quarantined owners (their
/// logins are refused until an operator clears the row; see running-locally.md).
async fn log_quarantined_lanes(
    characters: &dyn crate::application::CharacterRepository,
    zone: crate::domain::zone::ZoneId,
) {
    match characters.lane_quarantines(zone).await {
        Ok(rows) if rows.is_empty() => {
            tracing::info!(zone = zone.0, quarantined_lanes = 0, "no quarantined lanes");
        },
        Ok(rows) => {
            let owners = rows
                .iter()
                .map(|row| format!("{}@{}:{}", row.character_id, row.epoch, row.tick.0))
                .collect::<Vec<_>>()
                .join(",");
            tracing::warn!(
                zone = zone.0,
                quarantined_lanes = rows.len(),
                owners = %owners,
                "quarantined lanes: these owners are refused until an operator clears them"
            );
        },
        Err(error) => {
            tracing::warn!(zone = zone.0, error = %error, "quarantined lanes could not be listed");
        },
    }
}

#[cfg(test)]
#[path = "zone_runtime_tests.rs"]
mod tests;
