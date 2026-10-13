//! Reproducible eight-session runtime benchmark, using owned Postgres and `JetStream`.
//! Run with `DATABASE_URL`/`NATS_URL` and `cargo test --release --test runtime_perf eight_session_casting_runtime -- --ignored --nocapture`.
//! `PERF_TICKS` defaults to 600 at 10 Hz; `PERF_INTERVAL_MS=0` runs without pacing.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines
)]

#[path = "common/pg.rs"]
mod pg;
use api::application::checkpoint::CheckpointService;
use api::application::replay_log::{AppliedTickRecord, EventLog};
use api::application::session::{PlayerSpawn, SessionCodec, SessionRegistry};
use api::application::use_cases::{CreateCharacter, CreateCharacterInput};
use api::application::zone_actor::{manual_ticks, TickOutcome};
use api::application::zone_bootstrap::ZoneBootstrap;
use api::application::{CharacterRepository, IdempotencyKey};
use api::domain::character_progression::CharacterAppearance;
use api::domain::class::ClassId;
use api::domain::zone::*;
use api::domain::{AccountId, Race, SessionId};
use api::infrastructure::class_data::{load_classes, ClassSource};
use api::infrastructure::eventlog::JetStreamEventLog;
use api::infrastructure::memory::InMemorySessionAudit;
use api::infrastructure::postgres::{PgCharacterRepository, PgZoneSnapshotStore};
use api::infrastructure::rules_data::{load_rules, RulesSource};
use api::infrastructure::skill_data::Phase3DataSource;
use api::infrastructure::telemetry::Metrics;
use api::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};
use api::interface::ws::Phase3Codec;
use api::zone_runtime::prepare_phase3_bootstrap;
use futures_util::StreamExt as _;
use nightfall_api as api;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual release runtime benchmark; requires owned DATABASE_URL and NATS_URL"]
async fn eight_session_casting_runtime() {
    tracing_subscriber::fmt()
        .with_env_filter("nightfall_api::runtime_timing=debug")
        .with_ansi(false)
        .without_time()
        .init();
    // Serialize this reserved benchmark namespace; no production subjects are purged.
    let lock = std::fs::File::create("/tmp/nightfall-runtime-perf.lock").unwrap();
    lock.lock().unwrap();
    let pool = pg::migrated_pool().await.expect("DATABASE_URL required");
    let metrics = Metrics::detached();
    let mut def = parse_zone(TEST_ZONE_TOML).unwrap();
    def.zone = ZoneId(2_147_483_000);
    // A pre-admission fixture keeps all eight casters alive and their target available.
    let mut source = Phase3DataSource::embedded();
    let text = source.profiles.npcs.get_mut("npcs/keltir.toml").unwrap();
    assert!(text.contains("max_hp = 44") && text.contains("attack_speed = 253"));
    *text = text
        .replace("max_hp = 44", "max_hp = 1000000")
        .replace("attack_speed = 253", "attack_speed = 1");
    for npc in &mut def.npc_templates {
        npc.max_hp = 1_000_000;
        npc.attack_speed = 1;
    }
    for slot in &mut def.spawn_slots {
        slot.home = Vec2Fixed::from_tiles(100, 100);
        slot.count = 8;
        slot.respawn_delay_secs = 1;
        slot.respawn_random_secs = 0;
    }
    let prepared = prepare_phase3_bootstrap(
        &source,
        load_rules(&RulesSource::embedded()).unwrap(),
        load_classes(&ClassSource::embedded()).unwrap(),
        def,
    )
    .unwrap();
    let repo = Arc::new(
        PgCharacterRepository::new(pool.clone())
            .with_metrics(metrics.clone())
            .with_classes(prepared.classes.registry.clone())
            .with_phase3_rules(Arc::new(prepared.phase3.clone())),
    );
    let client = async_nats::connect(std::env::var("NATS_URL").expect("NATS_URL required"))
        .await
        .unwrap();
    let log = Arc::new(
        JetStreamEventLog::connect(client.clone(), metrics.clone())
            .await
            .unwrap(),
    );
    if log.latest_epoch(def_zone()).await.unwrap().is_some() {
        let mut stream = async_nats::jetstream::new(client)
            .get_stream("NF_ZONES")
            .await
            .unwrap();
        let dedup = stream.info().await.unwrap().config.duplicate_window;
        stream
            .purge()
            .filter("nightfall.zone.2147483000.>")
            .await
            .unwrap();
        // Purge does not clear the broker's message-id deduplication window.
        tokio::time::sleep(dedup + Duration::from_secs(1)).await;
    }
    let store = Arc::new(PgZoneSnapshotStore::new(pool.clone()));
    let (ticks, driver) = manual_ticks();
    let running = ZoneBootstrap::new(
        log.clone(),
        Some(store.clone()),
        Arc::new(api::infrastructure::memory::ManualClock::default()),
        Arc::new(metrics.clone()),
    )
    .with_rules(prepared.physical)
    .with_classes(prepared.classes.registry.clone(), prepared.classes.config_hash)
    .with_phase3_rules(prepared.phase3)
    .with_telemetry(Arc::new(metrics.clone()))
    .start(&prepared.zone, ticks)
    .await
    .unwrap();
    let zone = running.handle();
    zone.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(metrics.clone()),
        )
        .with_learning(repo.clone())
        .with_durability(log.clone(), store),
    );
    let sampler = pg_wait_sampler().await;
    let registry = SessionRegistry::with_characters(repo.clone());
    let mut players = Vec::new();
    let mut outputs = zone.subscribe();
    for i in 0..8_u8 {
        let account = AccountId::from_uuid(Uuid::from_u128(100 + u128::from(i)));
        pg::insert_account(&pool, account.as_uuid()).await;
        let mut c = CreateCharacter::new(
            Arc::new(api::infrastructure::memory::InMemoryCharacterRepository::default()),
            prepared.classes.registry.clone(),
        )
        .execute(CreateCharacterInput {
            idempotency_key: IdempotencyKey::new(),
            account_id: account,
            name: format!("PerfMage{}", char::from(b'A'.saturating_add(i))),
            race: Race::Human,
            base_class_id: Some(ClassId(10)),
            appearance: CharacterAppearance::default(),
        })
        .await
        .unwrap();
        c.id = api::domain::CharacterId::from_uuid(Uuid::from_u128(200 + u128::from(i)));
        repo.create_idempotent(
            &IdempotencyKey::from_uuid(Uuid::from_u128(300 + u128::from(i))),
            "benchmark-seed",
            &c,
        )
        .await
        .unwrap();
        // Seed only before admission; no live mutation.
        sqlx::query("UPDATE characters SET pos_x=100, pos_y=100 WHERE id=$1")
            .bind(c.id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        let entity = EntityId::from_uuid(c.id.as_uuid());
        registry
            .admit(
                zone,
                SessionId::from_uuid(Uuid::from_u128(400 + u128::from(i))),
                SessionGeneration(1),
                &PlayerSpawn {
                    entity,
                    name: c.name.to_string(),
                    pos: Vec2Fixed::from_tiles(100, 100),
                    speed: Speed::DEFAULT,
                    load: None,
                },
            )
            .await
            .unwrap();
        players.push(entity);
    }
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(0)));
    let initial = zone.snapshot().await.unwrap();
    let mut target = initial
        .entities
        .iter()
        .find(|e| {
            e.combat
                .as_ref()
                .is_some_and(|c| matches!(c.role, CombatRole::Npc { .. }))
        })
        .unwrap()
        .id;
    let ticks = std::env::var("PERF_TICKS").map_or(600, |s| s.parse::<u64>().unwrap());
    assert!(ticks > 0);
    let interval = std::env::var("PERF_INTERVAL_MS").map_or(100, |s| s.parse::<u64>().unwrap());
    let mut durations = Vec::new();
    let mut casts = BTreeMap::<EntityId, usize>::new();
    let mut completions = BTreeMap::<EntityId, usize>::new();
    let mut frames = 0_usize;
    for tick in 1..=ticks {
        let started = Instant::now();
        if tick % 60 == 1 {
            if let Some(npc) = zone.snapshot().await.unwrap().entities.iter().find(|e| {
                !e.targeting.dead
                    && e.combat
                        .as_ref()
                        .is_some_and(|c| matches!(c.role, CombatRole::Npc { .. }))
            }) {
                target = npc.id;
            }
            for entity in &players {
                zone.send(ZoneInput {
                    source: CommandSource::Session {
                        entity: *entity,
                        generation: SessionGeneration(1),
                    },
                    seq: Some(u32::try_from(tick).unwrap()),
                    command: ZoneCommand::CastSkill {
                        entity: *entity,
                        skill_id: api::domain::zone::skill::SkillId(1177),
                        target: Some(target),
                    },
                })
                .unwrap();
            }
        }
        let stepped_at = unix_ms();
        assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(Tick(tick)));
        let micros = zone.stats().borrow().duration_micros;
        println!("TICK n={tick} unix_ms={stepped_at} micros={micros}");
        durations.push(micros);
        let mapping = Instant::now();
        while let Ok(record) = outputs.try_recv() {
            for event in &record.events {
                if let ZoneEvent::SkillCastResult {
                    caster, targets, ..
                } = event
                {
                    if targets.iter().any(|t| {
                        matches!(
                            t.outcome,
                            SkillOutcome::Hit | SkillOutcome::Crit | SkillOutcome::Half
                        )
                    }) {
                        *completions.entry(caster.entity).or_default() += 1;
                    }
                }
                if let ZoneEvent::SkillCastStarted { caster, .. } = event {
                    *casts.entry(caster.entity).or_default() += 1;
                }
            }
            let encoded = AppliedTickRecord::from_applied(def_zone(), &record).encode();
            println!("FINGERPRINT tick={} record={:x}", record.tick.0, Sha256::digest(encoded));
            for entity in &players {
                if let Some(output) = record.outputs.get(entity) {
                    frames = frames
                        .saturating_add(Phase3Codec.outputs(output, record.server_time_ms).len());
                }
            }
        }
        println!("PERF mapping_us={}", mapping.elapsed().as_micros());
        tokio::time::sleep(Duration::from_millis(interval).saturating_sub(started.elapsed())).await;
    }
    durations.sort_unstable();
    println!(
        "PERF ticks={} mean_us={} p50_us={} p99_us={} casts={} frames={}",
        ticks,
        durations.iter().sum::<u64>() / ticks,
        durations.get(durations.len() / 2).unwrap(),
        durations
            .get(durations.len().saturating_mul(99) / 100)
            .unwrap(),
        casts.values().sum::<usize>(),
        frames
    );
    // The actor broadcast omits idle ticks. Read the acknowledged log outside timing.
    let mut records = log.read_epoch(def_zone(), 1).await.unwrap();
    let mut next = 0;
    while let Some(record) = records.next().await {
        let record = record.unwrap();
        assert_eq!(record.tick, Tick(next));
        println!(
            "DURABLE_FINGERPRINT tick={} record={:x}",
            record.tick.0,
            Sha256::digest(record.encode())
        );
        next += 1;
    }
    assert_eq!(next, ticks + 1);
    for player in &players {
        let count = casts.get(player).copied().unwrap_or_default();
        println!("CASTS player={player} successful={count}");
        let completed = completions.get(player).copied().unwrap_or_default();
        println!("COMPLETIONS player={player} successful={completed}");
        assert!(
            completed
                >= usize::try_from(ticks / 60)
                    .unwrap()
                    .saturating_sub(1)
                    .max(1)
        );
        assert!(
            count
                >= usize::try_from(ticks / 60)
                    .unwrap()
                    .saturating_sub(1)
                    .max(1)
        );
    }
    for row in sqlx::query("SELECT key, fingerprint, response FROM idempotency_keys WHERE operation='save_checkpoint' ORDER BY key")
        .fetch_all(&pool).await.unwrap() {
        println!("CHECKPOINT key={} fingerprint={} response={}", row.get::<Uuid, _>("key"), row.get::<String, _>("fingerprint"), row.get::<serde_json::Value, _>("response"));
    }
    assert_eq!(
        zone.snapshot()
            .await
            .unwrap()
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Player && !e.targeting.dead)
            .count(),
        8
    );
    for entity in &players {
        assert!(
            repo.load_for_admission(api::domain::CharacterId::from_uuid(entity.as_uuid()))
                .await
                .unwrap()
                .unwrap()
                .revision
                > 1
        );
    }
    if let Some((stop, task)) = sampler {
        stop.send(()).ok();
        task.await.unwrap();
    }
    println!("{}", metrics.render().unwrap());
    running
        .shutdown(api::application::replay_log::WatermarkReason::Shutdown)
        .await
        .unwrap();
}

#[test]
#[ignore = "offline analysis; set PERF_RECORDING to a verified Phase 3 recording"]
fn recorded_checkpoint_causes() {
    use api::application::replay_log::AppliedTickRecord;
    use api::infrastructure::eventlog::Recording;
    use std::collections::BTreeMap;
    let recording =
        Recording::read(std::path::Path::new(&std::env::var("PERF_RECORDING").unwrap())).unwrap();
    let mut state = ZoneState::from_snapshot(recording.snapshot).unwrap();
    let mut previous = state.snapshot();
    let mut counts = BTreeMap::<&str, usize>::new();
    for record in recording.records {
        let tick = state
            .run_tick(AppliedTickDraft {
                epoch: record.epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })
            .unwrap();
        assert!(record.reproduced_by(&AppliedTickRecord::from_applied(record.zone, &tick)));
        let snapshot = state.snapshot();
        if let Some(phase) = &snapshot.phase3 {
            for (id, side) in &phase.entities {
                if side.player.is_none() {
                    continue;
                }
                let old = previous.phase3.as_ref().and_then(|p| p.entities.get(id));
                let Some(old) = old else {
                    *counts.entry("admission").or_default() += 1;
                    continue;
                };
                let changes = [
                    ("reuse", old.reuse != side.reuse),
                    ("effects", old.effects != side.effects),
                    ("modifier_order", old.modifier_install_order != side.modifier_install_order),
                    ("fractions", old.fractions != side.fractions),
                    (
                        "pvp_death",
                        old.player
                            .as_ref()
                            .map(|p| (&p.pvp, p.death_penalty_level, &p.last_death))
                            != side
                                .player
                                .as_ref()
                                .map(|p| (&p.pvp, p.death_penalty_level, &p.last_death)),
                    ),
                ];
                if changes.iter().any(|(_, changed)| *changed) {
                    *counts.entry("retained_union").or_default() += 1;
                }
                for (name, changed) in changes {
                    if changed {
                        *counts.entry(name).or_default() += 1;
                    }
                }
            }
        }
        previous = snapshot;
    }
    println!("PERF recording checkpoint causes: {counts:?}");
}

/// With `PERF_PG_SAMPLE_MS`, samples every non-idle backend of this database (wait event,
/// statement age, abbreviated statement) and every ungranted lock, on a dedicated
/// connection outside the actor's pool. Read-only; it never takes a lock that the actor needs.
async fn pg_wait_sampler() -> Option<(tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>)>
{
    use sqlx::Connection as _;
    let every = std::env::var("PERF_PG_SAMPLE_MS")
        .ok()?
        .parse::<u64>()
        .unwrap();
    let mut conn = sqlx::PgConnection::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let started = Instant::now();
        loop {
            let at = started.elapsed().as_millis();
            let unix_ms = unix_ms();
            for row in sqlx::query(
                "SELECT pid, state, COALESCE(wait_event_type,'-') AS wt, COALESCE(wait_event,'-') AS we, \
                 (EXTRACT(EPOCH FROM clock_timestamp() - query_start) * 1000)::bigint AS age_ms, \
                 left(regexp_replace(query, '\\s+', ' ', 'g'), 48) AS q \
                 FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() \
                 AND state NOT IN ('idle')",
            )
            .fetch_all(&mut conn)
            .await
            .unwrap()
            {
                println!(
                    "PGSAMPLE t_ms={at} unix_ms={unix_ms} pid={} state={} wait={}:{} age_ms={} q={}",
                    row.get::<i32, _>("pid"),
                    row.get::<String, _>("state"),
                    row.get::<String, _>("wt"),
                    row.get::<String, _>("we"),
                    row.get::<Option<i64>, _>("age_ms").unwrap_or(-1),
                    row.get::<String, _>("q"),
                );
            }
            for row in sqlx::query(
                "SELECT l.pid, l.locktype, l.mode, COALESCE(c.relname, '-') AS rel \
                 FROM pg_locks l LEFT JOIN pg_class c ON c.oid = l.relation WHERE NOT l.granted",
            )
            .fetch_all(&mut conn)
            .await
            .unwrap()
            {
                println!(
                    "PGLOCKWAIT t_ms={at} unix_ms={unix_ms} pid={} type={} mode={} rel={}",
                    row.get::<i32, _>("pid"),
                    row.get::<String, _>("locktype"),
                    row.get::<String, _>("mode"),
                    row.get::<String, _>("rel"),
                );
            }
            tokio::select! {
                _ = &mut stopped => break,
                () = tokio::time::sleep(Duration::from_millis(every)) => {},
            }
        }
    });
    Some((stop, task))
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

/// Isolates the pre-log epoch fence: the production `recording` UPDATE every
/// `PERF_INTERVAL_MS`, alone on a fresh migrated schema, with no actor, log, checkpoint
/// transaction or other writer. Any stall here cannot come from row-lock contention.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual storage probe; requires owned DATABASE_URL"]
async fn fence_only_latency() {
    use api::application::replay_log::ZoneSnapshotStore as _;
    let pool = pg::migrated_pool().await.expect("DATABASE_URL required");
    sqlx::query("INSERT INTO zone_epochs (zone_id, epoch) VALUES (2147483001, 1)")
        .execute(&pool)
        .await
        .unwrap();
    let store = PgZoneSnapshotStore::new(pool.clone());
    let sampler = pg_wait_sampler().await;
    let ticks = std::env::var("PERF_TICKS").map_or(600, |s| s.parse::<u64>().unwrap());
    let interval = std::env::var("PERF_INTERVAL_MS").map_or(100, |s| s.parse::<u64>().unwrap());
    let mut durations = Vec::new();
    for tick in 1..=ticks {
        let started = Instant::now();
        let at = unix_ms();
        store
            .recording(ZoneId(2_147_483_001), 1, Tick(tick))
            .await
            .unwrap();
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap();
        println!("FENCE unix_ms={at} micros={micros}");
        durations.push(micros);
        tokio::time::sleep(Duration::from_millis(interval).saturating_sub(started.elapsed())).await;
    }
    if let Some((stop, task)) = sampler {
        stop.send(()).ok();
        task.await.unwrap();
    }
    durations.sort_unstable();
    println!(
        "FENCE_ONLY n={} mean_us={} p50_us={} p99_us={} max_us={}",
        ticks,
        durations.iter().sum::<u64>() / ticks,
        durations.get(durations.len() / 2).unwrap(),
        durations
            .get(durations.len().saturating_mul(99) / 100)
            .unwrap(),
        durations.last().unwrap()
    );
}

fn def_zone() -> ZoneId {
    ZoneId(2_147_483_000)
}
