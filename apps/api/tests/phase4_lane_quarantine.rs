//! T11 recovery: an unresolved V6 epoch whose history ends in a permanently failing item
//! checkpoint lane must not leave the zone unbootable. Recovery quarantines that owner (named,
//! durable), settles every other owner, and the zone boots; admission refuses only the
//! quarantined owner. Owned PostgreSQL and `JetStream` are required (no self-skip).
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
mod common;
use api::application::ports::*;
use api::application::replay_log::{EpochStatus, EventLog, WatermarkReason, ZoneSnapshotStore};
use api::domain::zone::ZoneId;
use api::domain::{AccountId, Character, CharacterId, CharacterName, DomainEvent, Race};
use api::infrastructure::eventlog::JetStreamEventLog;
use api::infrastructure::postgres::{
    connection_from_pool, PgCharacterRepository, PgZoneSnapshotStore,
};
use api::infrastructure::telemetry::Metrics;
use api::interface::grpc::pb::{self, server_message, world_event};
use api::zone_runtime::ZoneRuntimeConfig;
use common::ws;
use nightfall_api as api;
use parking_lot::Mutex;
use std::sync::Arc;
use uuid::Uuid;

/// Delegates to Postgres, but refuses every item checkpoint of the poisoned owner with a
/// permanent constraint, live and during recovery alike.
struct Poisoned {
    inner: Arc<PgCharacterRepository>,
    owner: Mutex<Option<CharacterId>>,
}
#[async_trait::async_trait]
impl ItemReceiptRepository for Poisoned {
    async fn item_receipt_lookup(
        &self,
        account: AccountId,
        operation: ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<ItemReceiptLookup> {
        self.inner
            .item_receipt_lookup(account, operation, key, fingerprint)
            .await
    }
}
#[async_trait::async_trait]
impl ItemCheckpointRepository for Poisoned {
    async fn checkpoint_items(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        work: &ItemCheckpointWork,
        learning: Option<&SuccessfulLearnReceipt>,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        if *self.owner.lock() == Some(cp.character_id) {
            return Err(CheckpointError::Constraint("injected_permanent_item_failure".into()));
        }
        self.inner
            .checkpoint_items(cp, events, work, learning)
            .await
    }
}

fn inventory_sync(m: &pb::ServerMessage) -> Option<&pb::InventorySync> {
    match &m.payload {
        Some(server_message::Payload::Event(pb::WorldEvent {
            event: Some(world_event::Event::InventorySync(sync)),
        })) => Some(sync),
        _ => None,
    }
}

async fn app_for(
    running: &api::application::zone_bootstrap::RunningZone,
    repo: &Arc<PgCharacterRepository>,
    writer: &Arc<Poisoned>,
) -> common::TestApp {
    let ports = repo.clone();
    let writer = writer.clone();
    common::TestApp::spawn_with_handle(
        move |deps| {
            deps.characters = ports.clone();
            deps.item_receipts = Some(ports.clone());
            deps.item_inventory = Some(ports.clone());
            deps.item_checkpoints = Some(writer);
            deps.item_admission = Some(ports.clone());
            deps.learning = Some(ports);
        },
        running.handle().clone(),
    )
    .await
}

async fn character(repo: &PgCharacterRepository, pool: &sqlx::PgPool, name: &str) -> ws::Player {
    let account = Uuid::now_v7();
    common::pg::insert_account(pool, account).await;
    let c = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), name, &c)
        .await
        .unwrap();
    ws::Player {
        account,
        character: c.id,
        name: name.to_owned(),
    }
}

#[tokio::test]
async fn recovery_quarantines_a_permanently_failing_item_lane_and_boots_the_zone() {
    let nats = std::env::var("NATS_URL").expect("owned NATS_URL required");
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let path = std::env::temp_dir().join(format!("phase4-quarantine-{}.toml", Uuid::now_v7()));
    std::fs::write(
        &path,
        format!("id='phase4_quarantine'\nzone_id={}\nname='Phase4 quarantine'\n[bounds]\nmin=[0,0]\nmax=[256,256]\n[safe_point]\npos=[126,126]\n", zone.0),
    )
    .unwrap();
    let cfg = ZoneRuntimeConfig {
        zone_file: Some(path.clone()),
        phase3_enabled: Some("1".into()),
        phase4_enabled: Some("1".into()),
        auth_dev_tokens: true,
        ..Default::default()
    };
    let classes = api::infrastructure::class_data::load_classes(
        &api::infrastructure::class_data::ClassSource::embedded(),
    )
    .unwrap();
    let bundle = cfg.prepare_phase4(&classes).unwrap().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(connection_from_pool(&pool))
            .with_classes(classes.registry.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let writer = Arc::new(Poisoned {
        inner: repo.clone(),
        owner: Mutex::new(None),
    });
    let mut deps = api::Dependencies::in_memory();
    deps.characters = repo.clone();
    deps.learning = Some(repo.clone());
    deps.item_receipts = Some(repo.clone());
    deps.item_inventory = Some(repo.clone());
    deps.item_admission = Some(repo.clone());
    deps.item_checkpoints = Some(writer.clone());
    deps.classes = Some(classes);
    let healthy = character(&repo, &pool, "QuarantineSafe").await;
    let poisoned = character(&repo, &pool, "QuarantineLost").await;

    // Epoch 1: the healthy owner's kit settles; the poisoned owner's first item checkpoint
    // (its kit at admission) fails permanently, the zone fences and the process "dies".
    let first =
        api::zone_runtime::start(&cfg, Some(&nats), Some(connection_from_pool(&pool)), &deps)
            .await
            .unwrap();
    let epoch = first.epoch();
    let app = app_for(&first, &repo, &writer).await;
    let mut socket = ws::join(&app, &healthy).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
        .await;
    let settled = inventory_sync(seen.last().unwrap()).unwrap().clone();
    socket.close().await;
    *writer.owner.lock() = Some(poisoned.character);
    let ticket = ws::ticket(&app, &poisoned).await;
    let _socket = ws::connect(&app, &ticket).await;
    tokio::time::timeout(std::time::Duration::from_secs(60), first.handle().stopped())
        .await
        .expect("the permanent failure fences the zone");
    assert!(first.handle().persistence_failed());
    drop(app);
    drop(first);
    let log =
        JetStreamEventLog::connect(async_nats::connect(&nats).await.unwrap(), Metrics::detached())
            .await
            .unwrap();
    assert!(matches!(
        log.epoch_status(zone, epoch).await.unwrap(),
        EpochStatus::Incomplete { .. }
    ));
    let store = PgZoneSnapshotStore::new(connection_from_pool(&pool));
    assert_eq!(store.unresolved(zone).await.unwrap().len(), 1);

    // Restart with the same history: recovery quarantines the poisoned owner and boots.
    let second =
        api::zone_runtime::start(&cfg, Some(&nats), Some(connection_from_pool(&pool)), &deps)
            .await
            .expect("a fenced item lane never leaves the zone unbootable");
    assert!(second.epoch() > epoch);
    assert!(store
        .unresolved(zone)
        .await
        .unwrap()
        .iter()
        .all(|row| row.epoch != epoch));
    assert!(matches!(log.epoch_status(zone, epoch).await.unwrap(), EpochStatus::Complete(_)));
    let refusal = repo
        .load_for_admission(poisoned.character)
        .await
        .expect_err("quarantined owners are never admitted");
    assert!(format!("{refusal:#}").starts_with(LANE_QUARANTINED), "{refusal:#}");
    let (reason, at_epoch): (String, i64) = sqlx::query_as(
        "SELECT reason, epoch FROM character_lane_quarantines WHERE character_id=$1",
    )
    .bind(poisoned.character.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(reason.contains("injected_permanent_item_failure"), "{reason}");
    assert_eq!(u64::try_from(at_epoch).unwrap(), epoch);
    // The operations listing (boot summary, runbook) names exactly this owner in this zone.
    let listed = repo.lane_quarantines(zone).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].character_id, listed[0].epoch), (poisoned.character, epoch));
    assert!(listed[0].reason.contains("injected_permanent_item_failure"));
    // The healthy owner is untouched: it logs into the new epoch with its settled inventory.
    let app = app_for(&second, &repo, &writer).await;
    let mut socket = ws::join(&app, &healthy).await;
    let seen = socket
        .until(|m| inventory_sync(m).is_some_and(|s| s.baseline))
        .await;
    let again = inventory_sync(seen.last().unwrap()).unwrap();
    assert_eq!(again.inventory_revision, settled.inventory_revision);
    assert_eq!(again.items, settled.items);
    socket.close().await;
    // The quarantined owner's login is refused by name with the permanent 4423 close (never
    // the retryable 1011 "zone unavailable"); the zone keeps running.
    let ticket = ws::ticket(&app, &poisoned).await;
    let mut refused = ws::connect(&app, &ticket)
        .await
        .expect("the refusal is a close after the upgrade");
    let frame = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        use futures_util::StreamExt as _;
        loop {
            match refused.socket.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => break frame,
                Some(Ok(_)) => {},
                // A socket that ends without a close frame fails the expect below.
                Some(Err(_)) | None => break None,
            }
        }
    })
    .await
    .expect("the quarantined login is closed at once")
    .expect("the close carries a code and reason");
    assert_eq!(u16::from(frame.code), 4423);
    assert_eq!(frame.reason.as_str(), LANE_QUARANTINED);
    assert!(!second.handle().persistence_failed());
    second.shutdown(WatermarkReason::Shutdown).await.unwrap();
    let _ = std::fs::remove_file(path);
}
