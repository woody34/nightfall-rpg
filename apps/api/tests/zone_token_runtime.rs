//! Production composition fences with a real isolated PostgreSQL schema and `JetStream`.
//! Requires `DATABASE_URL` for every case and `NATS_URL` for successful startup.
//! Missing dependencies fail explicitly; these acceptance checks never silently skip.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "common/pg.rs"]
mod pg;

use std::sync::Arc;
use std::time::Duration;

use nightfall_api::application::replay_log::{
    decode_snapshot, EpochStatus, EventLog, WatermarkReason, ZoneSnapshotStore,
};
use nightfall_api::domain::zone::{StateDigestVersion, ZoneId};
use nightfall_api::infrastructure::class_data::{load_classes, ClassSource};
use nightfall_api::infrastructure::eventlog::JetStreamEventLog;
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, PgCharacterRepository, PgZoneSnapshotStore,
};
use nightfall_api::infrastructure::telemetry::Metrics;
use nightfall_api::zone_runtime::{self, ZoneRuntimeConfig};
use nightfall_api::Dependencies;

struct ZoneFile(std::path::PathBuf);

impl Drop for ZoneFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn isolated_zone() -> (ZoneFile, ZoneId, ZoneRuntimeConfig) {
    let zone = ZoneId(u32::try_from(uuid::Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let path = std::env::temp_dir().join(format!("token-startup-{}.toml", uuid::Uuid::now_v7()));
    std::fs::write(
        &path,
        format!(
            "id = 'token_startup'\nzone_id = {}\nname = 'Token startup'\n\
             [bounds]\nmin = [0, 0]\nmax = [256, 256]\n\
             [safe_point]\npos = [126, 126]\n",
            zone.0
        ),
    )
    .unwrap();
    (
        ZoneFile(path.clone()),
        zone,
        ZoneRuntimeConfig {
            zone_file: Some(path),
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn persistent_startup_without_nats_creates_no_epoch() {
    let pool = pg::migrated_pool()
        .await
        .expect("DATABASE_URL is required for persistent runtime acceptance");
    let db = connection_from_pool(&pool);
    let store = PgZoneSnapshotStore::new(db.clone());
    let (_dir, zone, cfg) = isolated_zone();
    for url in [None, Some(""), Some("   ")] {
        let error = zone_runtime::start(&cfg, url, Some(db.clone()), &Dependencies::in_memory())
            .await
            .err()
            .expect("persistent runtime must reject missing broker configuration");
        assert!(error.to_string().contains("requires NATS_URL"));
        assert_eq!(store.latest_epoch(zone).await.unwrap(), None);
        assert!(store.unresolved(zone).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn persistent_startup_with_unreachable_broker_creates_no_epoch() {
    let pool = pg::migrated_pool()
        .await
        .expect("DATABASE_URL is required for persistent runtime acceptance");
    let db = connection_from_pool(&pool);
    let store = PgZoneSnapshotStore::new(db.clone());
    let (_dir, zone, cfg) = isolated_zone();
    // Port zero cannot host a broker. Exercise actual connection failure, after config
    // validation, without stopping the shared test broker or racing an ephemeral port.
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        zone_runtime::start(&cfg, Some("nats://127.0.0.1:0"), Some(db), &Dependencies::in_memory()),
    )
    .await
    .expect("broker startup failure must return promptly")
    .err()
    .expect("unreachable broker must prevent actor creation");
    assert!(!error.to_string().contains("requires NATS_URL"));
    assert_eq!(store.latest_epoch(zone).await.unwrap(), None);
    assert!(store.unresolved(zone).await.unwrap().is_empty());
}

#[tokio::test]
async fn persistent_startup_with_real_broker_persists_current_policy_before_returning() {
    let url = std::env::var("NATS_URL")
        .expect("NATS_URL is required for successful persistent runtime acceptance");
    let pool = pg::migrated_pool()
        .await
        .expect("DATABASE_URL is required for persistent runtime acceptance");
    let db = connection_from_pool(&pool);
    let store = PgZoneSnapshotStore::new(db.clone());
    let (_dir, zone, cfg) = isolated_zone();
    let classes = load_classes(&ClassSource::embedded()).unwrap();
    let mut deps = Dependencies::in_memory();
    deps.characters =
        Arc::new(PgCharacterRepository::new(db.clone()).with_classes(classes.registry.clone()));
    deps.classes = Some(classes);
    let running = zone_runtime::start(&cfg, Some(&url), Some(db), &deps)
        .await
        .unwrap();
    let epoch = running.epoch();
    assert_eq!(epoch, 1);
    let row = store.get(zone, epoch).await.unwrap().unwrap();
    let baseline = decode_snapshot(&row.snapshot).unwrap();
    assert_eq!(baseline.meta.schema_version, 8);
    assert_eq!(baseline.meta.digest_version, StateDigestVersion::BinaryV4);
    assert!(baseline.classes.is_some());
    assert_eq!(store.unresolved(zone).await.unwrap().len(), 1);
    let client = async_nats::connect(&url).await.unwrap();
    let log = JetStreamEventLog::connect(client, Metrics::detached())
        .await
        .unwrap();
    let recorded = log.read_snapshot(zone, epoch).await.unwrap().unwrap();
    assert_eq!(recorded.snapshot, baseline);
    let live = running.handle().snapshot().await.unwrap();
    assert_eq!(live.meta.digest_version, StateDigestVersion::BinaryV4);
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
    assert!(store.unresolved(zone).await.unwrap().is_empty());
    assert!(matches!(log.epoch_status(zone, epoch).await.unwrap(), EpochStatus::Complete(_)));
}
