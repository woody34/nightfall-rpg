//! The `JetStream` replay log (Story 3.2) and the `zone_snapshots` index (Story 3.4) against
//! the compose services. Need `NATS_URL` (and `DATABASE_URL` for the index); skip otherwise.
//! Every test uses its own zone id, so runs never see each other's epochs.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stderr
)]

mod common;
mod replay_support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_nats::jetstream::consumer::{pull, DeliverPolicy};
use bytes::Bytes;
use nightfall_api::application::replay_log::{
    encode_snapshot, open_epoch, AppliedTickRecord, EpochStatus, EventLog, GateConfig,
    NoReplayMetrics, OutputForm, Seq, SessionInRecord, WatermarkReason, ZoneSnapshotRow,
    ZoneSnapshotStore,
};
use nightfall_api::application::zone_actor::{manual_ticks, TickOutcome};
use nightfall_api::application::zone_bootstrap::ZoneBootstrap;
use nightfall_api::domain::zone::{
    EntityId, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneCommand, ZoneId, ZoneInput,
};
use nightfall_api::infrastructure::eventlog::{
    InMemoryZoneSnapshotStore, JetStreamEventLog, SESSIONS_STREAM,
};
use nightfall_api::infrastructure::outbox::JetStreamPublisher;
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgZoneSnapshotStore,
};
use nightfall_api::infrastructure::telemetry::Metrics;
use replay_support::{unique_zone, zone_def, FixedClock};
use sea_orm_migration::MigratorTrait;
use tokio_stream::StreamExt;
use uuid::Uuid;

async fn jetstream() -> Option<(async_nats::Client, Arc<JetStreamEventLog>)> {
    let url = std::env::var("NATS_URL").ok()?;
    let client = async_nats::connect(url).await.unwrap();
    // NF_EVENTS must be narrowed to `nightfall.*.*` before NF_ZONES / NF_SESSIONS can exist.
    JetStreamPublisher::connect(client.clone()).await.unwrap();
    let log = JetStreamEventLog::connect(client.clone(), Metrics::detached())
        .await
        .unwrap();
    Some((client, Arc::new(log)))
}

fn bootstrap(log: Arc<JetStreamEventLog>, store: Arc<dyn ZoneSnapshotStore>) -> ZoneBootstrap {
    ZoneBootstrap::new(log, Some(store), Arc::new(FixedClock), Arc::new(NoReplayMetrics))
        .with_gate_config(GateConfig::default())
}

fn empty_record(zone: u32, epoch: u64, tick: u64) -> AppliedTickRecord {
    AppliedTickRecord {
        zone: ZoneId(zone),
        epoch,
        tick: Tick(tick),
        server_time_ms: 0,
        commands: Vec::new(),
        dispositions: Vec::new(),
        outputs: Vec::new(),
        output_form: OutputForm::Encoded,
        events: bytes::Bytes::new(),
        state_digest: bytes::Bytes::from_static(&[0; 32]),
    }
}

async fn read_all(log: &dyn EventLog, zone: u32, epoch: u64) -> Vec<AppliedTickRecord> {
    let mut stream = log.read_epoch(ZoneId(zone), epoch).await.unwrap();
    let mut out = Vec::new();
    while let Some(r) = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .unwrap()
    {
        out.push(r.unwrap());
    }
    out
}

#[tokio::test]
async fn applied_records_round_trip_through_jetstream_in_order() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let zone = unique_zone();
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    let (ticks, driver) = manual_ticks();
    let running = bootstrap(log.clone(), store.clone())
        .start(&zone_def(zone), ticks)
        .await
        .unwrap();
    assert_eq!(running.epoch(), 1, "a fresh zone starts at epoch 1");
    let id = EntityId::from_uuid(Uuid::from_u128(1));
    running
        .handle()
        .send(ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: id,
            name: "p1".to_owned(),
            pos: Vec2Fixed::from_tiles(30, 30),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: None,
        }))
        .unwrap();
    for _ in 0..25 {
        assert!(matches!(driver.step().await.unwrap(), TickOutcome::Ran(_)));
    }
    let epoch = running.epoch();
    assert!(matches!(
        log.epoch_status(ZoneId(zone), epoch).await.unwrap(),
        EpochStatus::Incomplete {
            last_tick: Some(Tick(24))
        }
    ));
    let watermark = running.shutdown(WatermarkReason::Shutdown).await.unwrap();
    assert_eq!(
        log.epoch_status(ZoneId(zone), epoch).await.unwrap(),
        EpochStatus::Complete(watermark)
    );

    let records = read_all(log.as_ref(), zone, epoch).await;
    let ticks: Vec<u64> = records.iter().map(|r| r.tick.0).collect();
    assert_eq!(ticks, (0..25).collect::<Vec<_>>());
    assert_eq!(records[0].commands.len(), 3, "two fixture NPCs and the player");
    assert!(!records[0].outputs.is_empty(), "the player saw the NPC spawns");

    let mut opened = open_epoch(log.as_ref(), ZoneId(zone), epoch).await.unwrap();
    let mut n = 0;
    while let Some(r) = opened.records.next().await {
        r.unwrap();
        n += 1;
    }
    assert_eq!(n, 25);
    assert_eq!(log.latest_epoch(ZoneId(zone)).await.unwrap(), Some(epoch));
}

#[tokio::test]
async fn the_snapshot_precedes_the_first_record_in_the_stream() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let zone = unique_zone();
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    let (ticks, driver) = manual_ticks();
    let running = bootstrap(log.clone(), store.clone())
        .start(&zone_def(zone), ticks)
        .await
        .unwrap();
    driver.step().await.unwrap();
    let first = running.progress().borrow().first_seq.unwrap();
    let epoch = running.epoch();
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
    let snap = log
        .read_snapshot(ZoneId(zone), epoch)
        .await
        .unwrap()
        .unwrap();
    assert!(snap.seq < first, "snapshot {:?} before first record {:?}", snap.seq, first);
    assert_eq!(snap.snapshot.tick, Tick(0));
    assert!(snap.snapshot.meta.config_hash.starts_with("sha256:"));
}

#[tokio::test]
async fn a_restart_is_a_new_epoch() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let zone = unique_zone();
    let store = Arc::new(InMemoryZoneSnapshotStore::default());
    let (t1, _d1) = manual_ticks();
    let first = bootstrap(log.clone(), store.clone())
        .start(&zone_def(zone), t1)
        .await
        .unwrap();
    first.shutdown(WatermarkReason::Shutdown).await.unwrap();
    // A fresh index (as after losing the database): the log alone still yields the next epoch.
    let (t2, _d2) = manual_ticks();
    let second = bootstrap(log.clone(), Arc::new(InMemoryZoneSnapshotStore::default()))
        .start(&zone_def(zone), t2)
        .await
        .unwrap();
    assert_eq!(second.epoch(), 2);
}

#[tokio::test]
async fn a_duplicate_publish_of_the_same_tick_is_deduplicated_by_the_broker() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let zone = unique_zone();
    let a = log.append_applied(&empty_record(zone, 1, 0)).await.unwrap();
    let b = log.append_applied(&empty_record(zone, 1, 0)).await.unwrap();
    let c = log.append_applied(&empty_record(zone, 1, 1)).await.unwrap();
    assert_eq!(a, b, "the broker answers the retry with the original sequence");
    assert!(c > a);
    let ticks: Vec<u64> = read_all(log.as_ref(), zone, 1)
        .await
        .iter()
        .map(|r| r.tick.0)
        .collect();
    assert_eq!(ticks, vec![0, 1]);
}

#[tokio::test]
async fn an_epoch_without_a_watermark_is_incomplete_and_refused() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let zone = unique_zone();
    let (ticks, driver) = manual_ticks();
    let running = bootstrap(log.clone(), Arc::new(InMemoryZoneSnapshotStore::default()))
        .start(&zone_def(zone), ticks)
        .await
        .unwrap();
    for _ in 0..3 {
        driver.step().await.unwrap();
    }
    let epoch = running.epoch();
    drop(running); // no watermark, as after a crash
    assert_eq!(
        log.epoch_status(ZoneId(zone), epoch).await.unwrap(),
        EpochStatus::Incomplete {
            last_tick: Some(Tick(2))
        }
    );
    assert!(open_epoch(log.as_ref(), ZoneId(zone), epoch).await.is_err());
    assert_eq!(log.epoch_status(ZoneId(zone), epoch + 1).await.unwrap(), EpochStatus::Missing);
}

#[tokio::test]
async fn session_audit_frames_land_in_nf_sessions_in_order() {
    let Some((client, log)) = jetstream().await else {
        return;
    };
    let session = Uuid::now_v7();
    let frames: Vec<SessionInRecord> = (0..50)
        .map(|n| SessionInRecord {
            session,
            seq: n,
            zone: ZoneId(1),
            epoch: 1,
            tick_seen: Tick(n),
            recv_unix_ms: 1,
            frame: Bytes::from(vec![1, 2, 3]),
        })
        .collect();
    assert_eq!(log.append_session_in(&frames).await.unwrap(), 0);

    let js = async_nats::jetstream::new(client);
    let stream = js.get_stream(SESSIONS_STREAM).await.unwrap();
    let consumer = stream
        .create_consumer(pull::OrderedConfig {
            filter_subject: format!("nightfall.session.{}.in", session.simple()),
            deliver_policy: DeliverPolicy::All,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut messages = consumer.messages().await.unwrap();
    for want in &frames {
        let m = tokio::time::timeout(Duration::from_secs(5), messages.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(&SessionInRecord::decode(&m.payload).unwrap(), want);
    }
}

#[tokio::test]
async fn retention_is_seven_days_on_both_streams() {
    let Some((client, _)) = jetstream().await else {
        return;
    };
    let js = async_nats::jetstream::new(client);
    for name in ["NF_ZONES", "NF_SESSIONS"] {
        let mut s = js.get_stream(name).await.unwrap();
        let info = s.info().await.unwrap();
        assert_eq!(info.config.max_age, Duration::from_hours(7 * 24), "{name}");
    }
}

#[tokio::test]
async fn the_zone_snapshots_row_holds_the_log_bytes_and_first_seq() {
    let Some((_, log)) = jetstream().await else {
        return;
    };
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    let store: Arc<dyn ZoneSnapshotStore> = Arc::new(PgZoneSnapshotStore::new(db));
    let zone = unique_zone();
    let (ticks, driver) = manual_ticks();
    let running = bootstrap(log.clone(), store.clone())
        .start(&zone_def(zone), ticks)
        .await
        .unwrap();
    driver.step().await.unwrap();
    let epoch = running.epoch();
    let first = running.progress().borrow().first_seq.unwrap();
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let snap = log
        .read_snapshot(ZoneId(zone), epoch)
        .await
        .unwrap()
        .unwrap();
    let row = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = store.get(ZoneId(zone), epoch).await.unwrap().unwrap();
            if row.first_seq.is_some() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(row.snapshot, encode_snapshot(&snap.snapshot).unwrap());
    assert_eq!(row.snapshot_seq, snap.seq);
    assert_eq!(row.first_seq, Some(first));
    assert_eq!(row.time_origin_ms, replay_support::ORIGIN_MS);
    assert_eq!(store.latest_epoch(ZoneId(zone)).await.unwrap(), Some(epoch));
}

#[tokio::test]
async fn zone_snapshot_rows_insert_once_and_record_the_first_seq() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, None).await.unwrap();
    let store = PgZoneSnapshotStore::new(db);
    let row = ZoneSnapshotRow {
        zone: ZoneId(3),
        epoch: 9,
        snapshot: vec![1, 2, 3],
        snapshot_seq: Seq(10),
        first_seq: None,
        time_origin_ms: 5,
        build_id: "test".to_owned(),
        config_hash: "sha256:x".to_owned(),
        schema_version: 1,
    };
    store.insert(&row).await.unwrap();
    let mut other = row.clone();
    other.snapshot = vec![9];
    store.insert(&other).await.unwrap();
    assert_eq!(
        store.get(ZoneId(3), 9).await.unwrap(),
        Some(row.clone()),
        "second insert is a no-op"
    );
    store.set_first_seq(ZoneId(3), 9, Seq(11)).await.unwrap();
    assert_eq!(store.get(ZoneId(3), 9).await.unwrap().unwrap().first_seq, Some(Seq(11)));
    assert_eq!(store.latest_epoch(ZoneId(3)).await.unwrap(), Some(9));
    assert_eq!(store.latest_epoch(ZoneId(4)).await.unwrap(), None);
    assert_eq!(store.get(ZoneId(3), 8).await.unwrap(), None);
}

/// A record over the broker's `max_payload` (about 1.3 MB of output against the 1 MiB
/// default) fails to publish every time; bounded, it is stored with output digests, reads
/// back, and replay still tells a one-byte change apart.
#[tokio::test]
async fn an_oversized_record_is_stored_with_output_digests() {
    use nightfall_api::application::replay_log::PlayerOutput;
    use nightfall_api::infrastructure::eventlog::HEADER_ROOM;

    let Some((client, log)) = jetstream().await else {
        return;
    };
    let limit = log.max_record_bytes();
    assert_eq!(limit, client.max_payload() - HEADER_ROOM);
    let zone = unique_zone();
    let mut record = empty_record(zone, 1, 0);
    record.outputs = (0..40_u8)
        .map(|n| PlayerOutput {
            entity: EntityId::from_uuid(Uuid::from_u128(u128::from(n))),
            bytes: Bytes::from(vec![n; 32 * 1024]),
        })
        .collect();
    assert!(record.encoded_len() > client.max_payload());
    assert!(log.append_applied(&record).await.is_err(), "the broker refuses it");

    let bounded = record.clone().bounded(limit);
    assert_eq!(bounded.output_form, OutputForm::Sha256);
    log.append_applied(&bounded).await.unwrap();
    let stored = read_all(log.as_ref(), zone, 1).await;
    assert_eq!(stored, vec![bounded]);
    assert!(stored[0].reproduced_by(&record));
    let mut diverged = record.clone();
    let mut bytes = diverged.outputs[17].bytes.to_vec();
    bytes[1000] ^= 1;
    diverged.outputs[17].bytes = Bytes::from(bytes);
    assert!(!stored[0].reproduced_by(&diverged));
}

/// Measures the acknowledged append latency of one tick's record on this machine, for an idle
/// tick and for a busy one (50 players each seeing 20 moves). Run with
/// `cargo test --release --test replay_log_jetstream -- --ignored --nocapture ack_latency`.
#[tokio::test]
#[ignore = "benchmark: prints latency percentiles"]
async fn ack_latency() {
    use nightfall_api::application::replay_log::{encode_outputs, PlayerOutput};
    use nightfall_api::domain::zone::{ObserverOutput, ZoneEvent};

    let Some((_, log)) = jetstream().await else {
        return;
    };
    let moves: Vec<ObserverOutput> = (0..20_u128)
        .map(|n| {
            ObserverOutput::Event(ZoneEvent::EntityMove {
                tick: Tick(1),
                entity: EntityId::from_uuid(Uuid::from_u128(n)),
                pos: Vec2Fixed::from_tiles(10, 10),
                dest: Some(Vec2Fixed::from_tiles(20, 20)),
                speed: Speed::DEFAULT,
            })
        })
        .collect();
    let busy_outputs: Vec<PlayerOutput> = (0..50_u128)
        .map(|n| PlayerOutput {
            entity: EntityId::from_uuid(Uuid::from_u128(n)),
            bytes: encode_outputs(&moves),
        })
        .collect();
    for (label, outputs) in [("idle", Vec::new()), ("busy", busy_outputs)] {
        let zone = unique_zone();
        let mut samples = Vec::new();
        let mut size = 0;
        for tick in 0..2_000 {
            let mut record = empty_record(zone, 1, tick);
            record.outputs.clone_from(&outputs);
            size = record.encode().len();
            let started = Instant::now();
            log.append_applied(&record).await.unwrap();
            samples.push(started.elapsed());
        }
        samples.sort();
        let pct = |p: usize| samples[(samples.len() * p / 100).min(samples.len() - 1)];
        eprintln!(
            "{label} tick ({size} B) ack latency over {} appends: p50 {:?}, p90 {:?}, p99 {:?}, max {:?}",
            samples.len(),
            pct(50),
            pct(90),
            pct(99),
            samples[samples.len() - 1]
        );
    }
}
