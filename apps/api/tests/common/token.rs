//! Real Postgres/JetStream composition for the token policy acceptance tests.
#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::sync::Arc;

use nightfall_api::application::checkpoint::CheckpointService;
use nightfall_api::application::replay_log::NoReplayMetrics;
use nightfall_api::application::zone_actor::IntervalTicks;
use nightfall_api::application::zone_bootstrap::{RunningZone, ZoneBootstrap, ZoneDefinition};
use nightfall_api::application::{CharacterRepository, IdempotencyKey};
use nightfall_api::domain::zone::ZoneId;
use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
use nightfall_api::infrastructure::class_data::{load_classes, ClassSource};
use nightfall_api::infrastructure::eventlog::JetStreamEventLog;
use nightfall_api::infrastructure::memory::InMemorySessionAudit;
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgZoneSnapshotStore};
use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource};
use nightfall_api::infrastructure::telemetry::Metrics;
use nightfall_api::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};
use sqlx::PgPool;
use uuid::Uuid;

use super::{ws::Player, TestApp};

pub struct Harness {
    pub app: TestApp,
    pub pool: PgPool,
    pub repo: Arc<PgCharacterRepository>,
    pub log: Arc<JetStreamEventLog>,
    pub store: Arc<PgZoneSnapshotStore>,
    pub running: RunningZone,
}

impl Harness {
    pub async fn new() -> Self {
        Self::with_adapter(|repo| repo).await
    }

    pub async fn with_adapter(
        make: impl FnOnce(Arc<PgCharacterRepository>) -> Arc<dyn CharacterRepository>,
    ) -> Self {
        let pool = super::pg::migrated_pool()
            .await
            .expect("real token acceptance requires DATABASE_URL; no skipped pass");
        let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
        let adapter = make(repo.clone());
        let url = std::env::var("NATS_URL")
            .expect("real token acceptance requires NATS_URL; no skipped pass");
        let client = async_nats::connect(url).await.unwrap();
        assert_eq!(client.server_info().max_payload, 1_048_576);
        let log = Arc::new(
            JetStreamEventLog::connect(client, Metrics::detached())
                .await
                .unwrap(),
        );
        let store = Arc::new(PgZoneSnapshotStore::new(pool.clone()));
        let mut def = definition();
        // These tests create only their own NPCs through recorded commands.
        def.npcs.clear();
        def.spawn_slots.clear();
        let classes = load_classes(&ClassSource::embedded()).unwrap();
        let running = ZoneBootstrap::new(
            log.clone(),
            Some(store.clone()),
            Arc::new(nightfall_api::infrastructure::SystemClock),
            Arc::new(NoReplayMetrics),
        )
        .with_rules(load_rules(&RulesSource::embedded()).unwrap())
        .with_classes(classes.registry, classes.config_hash)
        .start(&def, IntervalTicks::new())
        .await
        .unwrap();
        let audit = Arc::new(InMemorySessionAudit::default());
        running.handle().checkpoints.install(
            CheckpointService::new(adapter.clone(), audit, Arc::new(Metrics::detached()))
                .with_durability(log.clone(), store.clone()),
        );
        let app = TestApp::spawn_with_handle(
            move |deps| deps.characters = adapter,
            running.handle().clone(),
        )
        .await;
        Self {
            app,
            pool,
            repo,
            log,
            store,
            running,
        }
    }

    pub async fn seed(&self, character: &Character) -> Player {
        self.repo
            .create_idempotent(&IdempotencyKey::new(), character.name.as_str(), character)
            .await
            .unwrap();
        Player {
            account: character.account_id.as_uuid(),
            character: character.id,
            name: character.name.as_str().into(),
        }
    }
}

pub fn definition() -> ZoneDefinition {
    let mut def = parse_zone(TEST_ZONE_TOML).unwrap();
    // Isolated log subjects: independent of production fixture zone 1.
    let bytes = Uuid::now_v7().into_bytes();
    def.zone = ZoneId((u32::from_le_bytes(bytes[12..16].try_into().unwrap()) & 0x7fff_ffff).max(2));
    def
}

pub fn character(name: &str, level: u32, tokens: [u32; 2], mask: u8) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::now_v7()),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    c.level = level;
    c.xp = super::rules().xp_to_level(level).unwrap();
    c.position = Position { x: 126.0, y: 126.0 };
    c.class_state.token_tier_1_count = tokens[0];
    c.class_state.token_tier_2_count = tokens[1];
    c.class_state.milestone_claimed_mask = mask;
    c
}

pub fn spawned(
    message: &nightfall_api::interface::grpc::pb::ServerMessage,
    player: &Player,
) -> bool {
    super::ws::spawn_of(message).is_some_and(|spawn| spawn.entity_id == player.entity_id())
}

pub fn balances(c: &Character) -> ([u32; 2], u8) {
    (
        [
            c.class_state.token_tier_1_count,
            c.class_state.token_tier_2_count,
        ],
        c.class_state.milestone_claimed_mask,
    )
}

pub async fn grants(pool: &PgPool, character: &Character) -> Vec<serde_json::Value> {
    sqlx::query_scalar("SELECT payload FROM outbox WHERE subject='nightfall.character.token_granted' AND payload->>'character_id'=$1 ORDER BY id")
        .bind(character.id.to_string()).fetch_all(pool).await.unwrap()
}
