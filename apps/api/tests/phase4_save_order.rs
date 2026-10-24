//! D162: one tick's owner saves run concurrently by account, and the recorded rows (outbox,
//! item ledger, receipts and checkpoint state) are byte-identical to the serial path.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::print_stdout,
    clippy::arithmetic_side_effects
)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::*;
use api::domain::zone::item_grant::grant_fingerprint;
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterId, CharacterName, Race};
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::postgres::PgCharacterRepository;
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::{class_data, item_data, rules_data};
use nightfall_api as api;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const NAMES: [&str; 8] = [
    "BurstAlpha",
    "BurstBravo",
    "BurstCharlie",
    "BurstDelta",
    "BurstEcho",
    "BurstFoxtrot",
    "BurstGolf",
    "BurstHotel",
];

fn player_load(loaded: api::application::ProgressionState) -> PlayerLoad {
    PlayerLoad {
        phase3: loaded.phase3.or_else(|| Some(Phase3Checkpoint::new())),
        phase4: loaded.phase4,
        progression: Some(Box::new(PlayerProgression {
            identity: loaded.identity,
            class_state: loaded.class_state,
            max_cp: 0,
        })),
        checkpoint_revision: Some(loaded.revision),
        class: loaded.class_profile,
        level: loaded.level,
        xp: loaded.xp,
        hp: loaded.hp,
        mp: loaded.mp,
        alive: loaded.alive,
    }
}

/// The wall time of each burst tick's `admitted`: the admission, then two grant bursts.
type Timings = Vec<Duration>;

/// Owner `i` belongs to account `accounts[i]`; owners are in entity (UUID) order.
async fn burst<R>(repo: Arc<R>, accounts: &[u128], concurrency: usize) -> Timings
where
    R: CharacterRepository
        + ItemAdmissionRepository
        + ItemCheckpointRepository
        + ItemInventoryRepository
        + 'static,
{
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let mut owners = Vec::new();
    for (index, account) in accounts.iter().enumerate() {
        let mut c = Character::create(
            AccountId::from_uuid(Uuid::from_u128(*account)),
            CharacterName::new(NAMES[index]).unwrap(),
            Race::Human,
        );
        c.id = CharacterId::from_uuid(Uuid::from_u128(0xd162_0000 + index as u128));
        repo.create_idempotent(
            &IdempotencyKey::from_uuid(Uuid::from_u128(0xc000 + index as u128)),
            NAMES[index],
            &c,
        )
        .await
        .unwrap();
        owners.push(c);
    }
    let zone = ZoneId(4162);
    let range = repo.reserve_item_range(zone, 1).await.unwrap();
    let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
    let mut state = ZoneState::new_phase4(
        ZoneSeed { zone, epoch: 1 },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        classes.registry,
        (*bundle.skills).clone(),
        bundle.items.clone(),
        ItemInstanceRange::new(range.range_start, range.next_reserved, range.range_end_exclusive)
            .unwrap(),
    )
    .unwrap();
    let mut service = CheckpointService::new(
        repo.clone(),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    )
    .with_items(repo.clone())
    .with_save_concurrency(concurrency);
    let mut timings = Vec::new();
    let mut spawns = Vec::new();
    for (index, c) in owners.iter().enumerate() {
        let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
        spawns.push(ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: EntityId::from_uuid(c.id.as_uuid()),
            name: c.name.as_str().to_owned(),
            pos: Vec2Fixed::from_tiles(100 + i32::try_from(index).unwrap(), 100),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(player_load(loaded))),
        }));
    }
    let admitted = state.run_tick(state.draft(spawns)).unwrap();
    assert!(admitted.dispositions.is_empty());
    let started = Instant::now();
    service
        .admitted(&admitted, &state.snapshot())
        .await
        .unwrap();
    timings.push(started.elapsed());
    for round in 0..2_u128 {
        let grants = owners
            .iter()
            .enumerate()
            .map(|(index, c)| {
                ZoneInput::system(ZoneCommand::MaterializePendingItems(Box::new(
                    ItemGrantCommand {
                        entity: EntityId::from_uuid(c.id.as_uuid()),
                        account: c.account_id,
                        request_id: Uuid::from_u128(0xa000 + round * 0x100 + index as u128),
                        template_id: 57,
                        count: 10,
                        fingerprint: grant_fingerprint(c.id, 57, 10),
                        rules_hash: bundle.items.hash(),
                        paused: None,
                        known: None,
                        conflict: None,
                        enchant: 0,
                    },
                )))
            })
            .collect();
        let tick = state.run_tick(state.draft(grants)).unwrap();
        assert!(tick.dispositions.is_empty());
        let owners_with_items = tick
            .events
            .iter()
            .filter(|event| matches!(event, ZoneEvent::ItemCheckpoint(_)))
            .count();
        assert_eq!(owners_with_items, accounts.len(), "every owner has item facts");
        let started = Instant::now();
        service.admitted(&tick, &state.snapshot()).await.unwrap();
        timings.push(started.elapsed());
    }
    for c in &owners {
        let saved = repo.retained_inventory(c.id).await.unwrap().unwrap();
        assert_eq!(saved.character_revision, 3, "{} saved every burst", c.name.as_str());
    }
    timings
}

/// Every recorded row in a stable order, with only the wall-clock columns removed.
async fn recorded(pool: &sqlx::PgPool) -> Vec<(String, Vec<String>)> {
    let tables = [
        ("outbox", "id"),
        ("item_ledger", "id"),
        ("item_mutation_receipts", "account_id,operation,key"),
        ("idempotency_keys", "account_id,operation,key"),
        ("item_instances", "id"),
        ("item_id_ranges", "zone_id,epoch,range_start"),
        ("characters", "id"),
    ];
    let mut out = Vec::new();
    for (table, order) in tables {
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT (to_jsonb(t) - 'created_at' - 'updated_at')::text FROM {table} t ORDER BY {order}"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        out.push((table.to_owned(), rows));
    }
    out
}

async fn postgres_run(
    accounts: &[u128],
    concurrency: usize,
) -> Option<(Vec<(String, Vec<String>)>, Timings)> {
    let pool = common::pg::migrated_pool().await?;
    // Open all pool connections first, so the timings do not include connection setup.
    let warm: Vec<_> = (0..8)
        .map(|_| sqlx::query("SELECT pg_sleep(0.05)").execute(&pool))
        .collect();
    for result in futures_util::future::join_all(warm).await {
        result.unwrap();
    }
    let mut distinct = accounts.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    for account in distinct {
        common::pg::insert_account(&pool, Uuid::from_u128(account)).await;
    }
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
            .unwrap(),
    );
    let timings = burst(repo, accounts, concurrency).await;
    Some((recorded(&pool).await, timings))
}

async fn postgres_contract(label: &str, accounts: &[u128]) {
    let Some((serial, serial_timings)) = postgres_run(accounts, 1).await else {
        return;
    };
    let (concurrent, concurrent_timings) = postgres_run(accounts, 8).await.unwrap();
    for ((table, expected), (_, actual)) in serial.iter().zip(&concurrent) {
        assert!(!expected.is_empty() || table == "item_mutation_receipts", "{table} has rows");
        assert_eq!(expected, actual, "{label}: {table} rows differ from the serial path");
    }
    let outbox = &serial[0].1;
    assert!(outbox.len() >= accounts.len() * 2, "{label}: outbox holds the grant facts");
    println!(
        "D162 {label}: serial admission/grant/grant = {serial_timings:?}; concurrent = {concurrent_timings:?}"
    );
}

#[tokio::test]
async fn postgres_five_owner_burst_records_the_serial_rows() {
    postgres_contract("5 owners / 5 accounts", &[1, 2, 3, 4, 5]).await;
}

#[tokio::test]
async fn postgres_eight_owner_burst_records_the_serial_rows() {
    // Two accounts own two characters each; those owners stay serial in entity order.
    postgres_contract("8 owners / 6 accounts", &[1, 2, 3, 4, 5, 1, 6, 3]).await;
}

#[tokio::test]
async fn memory_eight_owner_burst_records_the_serial_ledger() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let mut ledgers = Vec::new();
    for concurrency in [1, 8] {
        let repo = Arc::new(
            InMemoryCharacterRepository::default()
                .with_phase4_rules(bundle.items.clone(), bundle.skills.clone())
                .unwrap(),
        );
        burst(repo.clone(), &[1, 2, 3, 4, 5, 1, 6, 3], concurrency).await;
        ledgers.push(repo.item_ledger_for_test());
    }
    assert!(!ledgers[0].is_empty());
    assert_eq!(ledgers[0], ledgers[1]);
}
