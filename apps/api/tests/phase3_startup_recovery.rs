//! Prepared V5 startup and crash recovery against owned PostgreSQL and `JetStream`.
//! Dependencies are required: these acceptance tests never count a self-skip as a pass.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
mod common;

use api::application::ports::*;
use api::application::replay_log::{
    decode_snapshot, EpochStatus, EventLog, WatermarkReason, ZoneSnapshotStore,
};
use api::application::session::{PlayerSpawn, SessionRegistry};
use api::application::zone_actor::manual_ticks;
use api::application::zone_bootstrap::ZoneBootstrap;
use api::application::{CharacterRepository, IdempotencyKey};
use api::domain::zone::{
    skill::{SkillId, SkillRef},
    *,
};
use api::domain::{
    AccountId, Character, CharacterId, CharacterName, DomainEvent, Position, Race, SessionId,
};
use api::infrastructure::class_data::{load_classes, ClassSource};
use api::infrastructure::eventlog::JetStreamEventLog;
use api::infrastructure::postgres::{
    connection_from_pool, PgCharacterRepository, PgZoneSnapshotStore,
};
use api::infrastructure::rules_data::{load_rules, RulesSource};
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::zone_data::parse_zone;
use api::interface::grpc::pb;
use api::zone_runtime::ZoneRuntimeConfig;
use api::Dependencies;
use nightfall_api as api;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use uuid::Uuid;

struct Fixture {
    cfg: ZoneRuntimeConfig,
    pool: sqlx::PgPool,
    repo: Arc<PgCharacterRepository>,
    store: Arc<PgZoneSnapshotStore>,
    log: Arc<JetStreamEventLog>,
    deps: Dependencies,
    zone: ZoneId,
    source: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(path) = &self.cfg.zone_file {
            let _ = std::fs::remove_file(path);
        }
    }
}
async fn fixture() -> Fixture {
    let url = std::env::var("NATS_URL").expect("owned NATS_URL required");
    let pool = common::pg::migrated_pool()
        .await
        .expect("owned DATABASE_URL required");
    let db = connection_from_pool(&pool);
    let zone = ZoneId(u32::try_from(Uuid::now_v7().as_u128() & 0x7fff_ffff).unwrap());
    let source = format!("id='phase3_startup'\nzone_id={}\nname='Phase3 startup'\n[bounds]\nmin=[0,0]\nmax=[256,256]\n[safe_point]\npos=[126,126]\n",zone.0);
    let path = std::env::temp_dir().join(format!("phase3-startup-{}.toml", Uuid::now_v7()));
    std::fs::write(&path, &source).unwrap();
    let cfg = ZoneRuntimeConfig {
        zone_file: Some(path),
        phase3_enabled: Some("1".into()),
        ..Default::default()
    };
    let classes = load_classes(&ClassSource::embedded()).unwrap();
    let rules = Arc::new(cfg.prepare_phase3(classes.clone()).unwrap().unwrap().phase3);
    let repo = Arc::new(
        PgCharacterRepository::new(db.clone())
            .with_classes(classes.registry.clone())
            .with_phase3_rules(rules),
    );
    let mut deps = Dependencies::in_memory();
    deps.characters = repo.clone();
    deps.learning = Some(repo.clone());
    deps.classes = Some(classes);
    let log = Arc::new(
        JetStreamEventLog::connect(async_nats::connect(&url).await.unwrap(), Metrics::detached())
            .await
            .unwrap(),
    );
    let store = Arc::new(PgZoneSnapshotStore::new(db.clone()));
    Fixture {
        cfg,
        pool,
        repo,
        store,
        log,
        deps,
        zone,
        source,
    }
}

// Explicit immutable construction uses the same durable bootstrap/checkpoint/service
// components as the prepared production composition. The normal switch stays fenced
// while Z's independent review is unresolved; this helper asserts no production enablement.
async fn explicit_v5(
    f: &Fixture,
    deps: &Dependencies,
) -> anyhow::Result<api::application::zone_bootstrap::RunningZone> {
    let classes = deps.classes.as_ref().unwrap();
    let bundle = f.cfg.prepare_phase3(classes.clone())?.unwrap();
    let mut checkpoints = api::application::checkpoint::CheckpointService::new(
        deps.characters.clone(),
        deps.audit.clone(),
        Arc::new(deps.metrics.clone()),
    )
    .with_learning(deps.learning.as_ref().unwrap().clone());
    checkpoints
        .recover_indexed(f.log.as_ref(), f.store.as_ref(), f.zone)
        .await?;
    checkpoints = checkpoints.with_durability(f.log.clone(), f.store.clone());
    let running = ZoneBootstrap::new(
        f.log.clone(),
        Some(f.store.clone()),
        deps.clock.clone(),
        Arc::new(deps.metrics.clone()),
    )
    .with_rules(bundle.physical)
    .with_classes(classes.registry.clone(), classes.config_hash.clone())
    .with_phase3_rules(bundle.phase3)
    .start(&bundle.zone, api::application::zone_actor::IntervalTicks::new())
    .await?;
    running.handle().checkpoints.install(checkpoints);
    Ok(running)
}

async fn baseline(f: &Fixture, epoch: u64) -> api::domain::zone::ZoneSnapshot {
    let indexed = f.store.get(f.zone, epoch).await.unwrap().unwrap();
    let snapshot = decode_snapshot(&indexed.snapshot).unwrap();
    let recorded = f.log.read_snapshot(f.zone, epoch).await.unwrap().unwrap();
    assert_eq!(snapshot, recorded.snapshot);
    snapshot
}

#[tokio::test]
async fn fresh_v5_start_and_restart_publish_matching_pg_nats_baselines_and_normal_rpcs() {
    let f = fixture().await;
    for expected in 1..=2 {
        let running = explicit_v5(&f, &f.deps).await.unwrap();
        assert_eq!(running.epoch(), expected);
        let snapshot = baseline(&f, expected).await;
        assert_eq!(snapshot.meta.schema_version, 9);
        assert_eq!(snapshot.meta.digest_version, StateDigestVersion::BinaryV5);
        assert_eq!(snapshot.phase3.as_ref().unwrap().rules.runtime_revision(), 2);
        let repo = f.repo.clone();
        let app = common::TestApp::spawn_with_handle(
            move |deps| {
                deps.characters = repo.clone();
                deps.learning = Some(repo);
            },
            running.handle().clone(),
        )
        .await;
        let mut client = app.game_as(common::account());
        let catalogue = client
            .get_skill_catalog(pb::GetSkillCatalogRequest {})
            .await
            .unwrap()
            .into_inner();
        assert!(!catalogue.skills.is_empty());
        assert_eq!(
            app.anon_grpc()
                .get_skill_catalog(pb::GetSkillCatalogRequest {})
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        drop(app);
        running.shutdown(WatermarkReason::Shutdown).await.unwrap();
        assert!(f.store.unresolved(f.zone).await.unwrap().is_empty());
        assert!(matches!(
            f.log.epoch_status(f.zone, expected).await.unwrap(),
            EpochStatus::Complete(_)
        ));
    }
}

#[tokio::test]
async fn unfinished_legacy_epoch_is_verified_durably_closed_then_replaced_by_v5() {
    let f = fixture().await;
    let classes = f.deps.classes.as_ref().unwrap();
    let bootstrap = ZoneBootstrap::new(
        f.log.clone(),
        Some(f.store.clone()),
        f.deps.clock.clone(),
        Arc::new(Metrics::detached()),
    )
    .with_rules(load_rules(&RulesSource::embedded()).unwrap())
    .with_classes(classes.registry.clone(), classes.config_hash.clone());
    let (ticks, driver) = manual_ticks();
    let legacy = bootstrap
        .start(&parse_zone(&f.source).unwrap(), ticks)
        .await
        .unwrap();
    assert_eq!(baseline(&f, 1).await.meta.schema_version, 8);
    driver.step().await.unwrap();
    drop(driver);
    legacy.handle().stopped().await;
    assert!(matches!(
        f.log.epoch_status(f.zone, 1).await.unwrap(),
        EpochStatus::Incomplete { .. }
    ));
    // Simulate the old process exiting without its graceful shutdown watermark.
    drop(legacy);
    let running = explicit_v5(&f, &f.deps).await.unwrap();
    assert_eq!(running.epoch(), 2);
    assert_eq!(baseline(&f, 2).await.meta.schema_version, 9);
    let watermark = match f.log.epoch_status(f.zone, 1).await.unwrap() {
        EpochStatus::Complete(watermark) => Some(watermark),
        EpochStatus::Missing | EpochStatus::Incomplete { .. } => None,
    }
    .expect("recovered legacy epoch must be durably closed");
    assert_eq!(watermark.records, 1);
    assert_eq!(watermark.last_tick, Some(Tick(0)));
    assert_eq!(watermark.reason, WatermarkReason::EpochEnd);
    assert_eq!(f.store.unresolved(f.zone).await.unwrap().len(), 1);
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
}

struct LostAck {
    inner: Arc<PgCharacterRepository>,
    lose: AtomicBool,
}
#[async_trait::async_trait]
impl CharacterRepository for LostAck {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        self.inner.get(id).await
    }
    async fn list_by_account(&self, id: AccountId) -> anyhow::Result<Vec<Character>> {
        self.inner.list_by_account(id).await
    }
    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fp: &str,
        c: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        self.inner.create_idempotent(key, fp, c).await
    }
    async fn load_for_admission(
        &self,
        id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        self.inner.load_for_admission(id).await
    }
    async fn checkpoint(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        self.inner.checkpoint(cp, events).await
    }
}
#[async_trait::async_trait]
impl LearnReceiptRepository for LostAck {
    async fn learn_receipt_lookup(
        &self,
        a: AccountId,
        key: &IdempotencyKey,
        fp: &str,
    ) -> anyhow::Result<LearnReceiptLookup> {
        self.inner.learn_receipt_lookup(a, key, fp).await
    }
}
#[async_trait::async_trait]
impl LearningCheckpointRepository for LostAck {
    async fn checkpoint_learning(
        &self,
        cp: &CharacterCheckpoint,
        events: &[DomainEvent],
        receipt: &SuccessfulLearnReceipt,
    ) -> Result<LearningCheckpointOutcome, CheckpointError> {
        let result = self.inner.checkpoint_learning(cp, events, receipt).await?;
        if self.lose.swap(false, Ordering::AcqRel) {
            // The actual PostgreSQL transaction committed; its response vanished before
            // the process could expose success. A terminal transport fault models the crash.
            return Err(CheckpointError::Constraint("injected_lost_commit_ack".into()));
        }
        Ok(result)
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Actual paid transaction, vanished ack, restart, immutable replay and outbox count.
async fn lost_paid_checkpoint_ack_recovers_once_before_fresh_v5_admission() {
    let f = fixture().await;
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::now_v7()),
        CharacterName::new("RecoveredLearner").unwrap(),
        Race::Human,
    );
    c.level = 10;
    c.xp = load_rules(&RulesSource::embedded())
        .unwrap()
        .rules
        .xp_to_level(10)
        .unwrap();
    c.class_state.sp = 2000;
    c.position = Position { x: 126.0, y: 128.0 };
    common::pg::insert_account(&f.pool, c.account_id.as_uuid()).await;
    f.repo
        .create_idempotent(&IdempotencyKey::new(), "recovery-seed", &c)
        .await
        .unwrap();
    let fault = Arc::new(LostAck {
        inner: f.repo.clone(),
        lose: AtomicBool::new(true),
    });
    let mut deps = f.deps.clone();
    deps.characters = fault.clone();
    deps.learning = Some(fault);
    let failed = explicit_v5(&f, &deps).await.unwrap();
    let registry = SessionRegistry::with_characters(deps.characters.clone());
    let entity = EntityId::from_uuid(c.id.as_uuid());
    registry
        .admit(
            failed.handle(),
            SessionId::new(),
            SessionGeneration(1),
            &PlayerSpawn {
                entity,
                name: c.name.to_string(),
                pos: Vec2Fixed::from_tiles(126, 128),
                speed: Speed::DEFAULT,
                load: None,
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !failed
            .handle()
            .snapshot()
            .await
            .unwrap()
            .entities
            .iter()
            .any(|e| e.id == entity)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let key = IdempotencyKey::new();
    let reply = failed
        .handle()
        .enqueue_learn_skill(ZoneInput {
            source: CommandSource::Session {
                entity,
                generation: SessionGeneration(1),
            },
            seq: None,
            command: ZoneCommand::LearnSkill {
                entity,
                account: c.account_id,
                request_key: key.as_uuid(),
                skill_id: SkillId(3),
                skill_level: 1,
            },
        })
        .unwrap();
    assert!(reply.await.unwrap().is_err());
    failed.handle().stopped().await;
    assert!(failed.handle().persistence_failed());
    let fp = learn_skill_fingerprint(
        c.id,
        SkillRef {
            id: SkillId(3),
            rank: 1,
        },
    );
    let frozen = match f
        .repo
        .learn_receipt_lookup(c.account_id, &key, &fp)
        .await
        .unwrap()
    {
        LearnReceiptLookup::Known(frozen) => Some(frozen),
        LearnReceiptLookup::Unknown | LearnReceiptLookup::Conflict => None,
    }
    .expect("database transaction really committed before its lost response");
    assert!(f
        .repo
        .load_for_admission(c.id)
        .await
        .unwrap()
        .unwrap()
        .phase3
        .is_some());
    drop(failed);
    let recovered = explicit_v5(&f, &f.deps).await.unwrap();
    assert_eq!(recovered.epoch(), 2);
    assert!(matches!(f.log.epoch_status(f.zone, 1).await.unwrap(), EpochStatus::Complete(_)));
    assert_eq!(
        f.repo
            .learn_receipt_lookup(c.account_id, &key, &fp)
            .await
            .unwrap(),
        LearnReceiptLookup::Known(frozen.clone())
    );
    let repo = f.repo.clone();
    let app = common::TestApp::spawn_with_handle(
        move |deps| {
            deps.characters = repo.clone();
            deps.learning = Some(repo);
        },
        recovered.handle().clone(),
    )
    .await;
    // The old caller is offline. Normal gRPC composition still replays its frozen receipt.
    let response = app
        .game_as(c.account_id.as_uuid())
        .learn_skill(pb::LearnSkillRequest {
            character_id: c.id.to_string(),
            skill_id: 3,
            skill_level: 1,
            idempotency_key: key.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.sp_remaining, frozen.sp_remaining);
    assert_eq!(response.tick, frozen.tick.0);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox WHERE payload->>'type' = 'character_skill_learned'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let saved = f.repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(saved.class_state.sp, frozen.sp_remaining);
    assert!(saved
        .class_state
        .learned_skills
        .iter()
        .any(|s| s.key == "l2.skill.3" && s.level == 1));
    app.sessions_shutdown.cancel();
    drop(app);
    recovered.shutdown(WatermarkReason::Shutdown).await.unwrap();
}
