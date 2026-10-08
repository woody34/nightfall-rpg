//! The replay log against the in-memory adapter and a fault-injecting wrapper (Stories 3.2,
//! 3.4): records in order with contiguous ticks, the gate holds a tick until its record is
//! acknowledged, a dead broker stalls (and then pauses) the zone without releasing anything,
//! a process killed mid-tick leaves an incomplete epoch that replay refuses, the snapshot
//! precedes the first record, duplicates are stored once, records round-trip byte for byte.
//!
//! Broker faults are injected with `FaultyLog` rather than `docker compose pause nats`: the
//! compose NATS is shared by every test binary and parallel session, so pausing it would fail
//! unrelated tests.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod replay_support;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use nightfall_api::application::replay_log::{
    decode_outputs, encode_outputs, open_epoch, AppliedTickRecord, EpochStatus, EventLog,
    GateConfig, NoReplayMetrics, OutputForm, PlayerOutput, ReplayError, SessionAuditWriter,
    SessionInRecord, SessionOutRecord, Watermark, WatermarkReason, ZoneSnapshotStore,
};
use nightfall_api::application::zone_actor::{manual_ticks, ManualTickDriver, TickOutcome};
use nightfall_api::application::zone_bootstrap::{RunningZone, ZoneBootstrap};
use nightfall_api::domain::zone::{
    AppliedCommand, CommandSource, Disposition, EntityId, EntityKind, Fixed, ObserverOutput,
    Ordinal, RejectReason, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneCommand, ZoneEvent,
    ZoneId, ZoneInput, ZoneState,
};
use nightfall_api::infrastructure::eventlog::{InMemoryEventLog, InMemoryZoneSnapshotStore};
use proptest::prelude::*;
use replay_support::{zone_def, FaultyLog, FixedClock};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const ZONE: u32 = 7;

struct Rig {
    mem: Arc<InMemoryEventLog>,
    faulty: Arc<FaultyLog>,
    store: Arc<InMemoryZoneSnapshotStore>,
}

impl Rig {
    fn new() -> Self {
        Self::with_log(InMemoryEventLog::default())
    }

    fn with_log(mem: InMemoryEventLog) -> Self {
        let mem = Arc::new(mem);
        Self {
            faulty: Arc::new(FaultyLog::new(mem.clone())),
            mem,
            store: Arc::new(InMemoryZoneSnapshotStore::default()),
        }
    }

    fn bootstrap(&self, gate: GateConfig) -> ZoneBootstrap {
        ZoneBootstrap::new(
            self.faulty.clone(),
            Some(self.store.clone()),
            Arc::new(FixedClock),
            Arc::new(NoReplayMetrics),
        )
        .with_gate_config(gate)
    }

    async fn start(&self) -> (RunningZone, ManualTickDriver) {
        self.start_with(fast_gate()).await
    }

    async fn start_with(&self, gate: GateConfig) -> (RunningZone, ManualTickDriver) {
        let (ticks, driver) = manual_ticks();
        let zone = self
            .bootstrap(gate)
            .start(&zone_def(ZONE), ticks)
            .await
            .unwrap();
        (zone, driver)
    }
}

fn fast_gate() -> GateConfig {
    GateConfig {
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
        max_stall: Duration::from_secs(60),
    }
}

fn player(n: u128) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: EntityId(Uuid::from_u128(n)),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(40, 40),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
    })
}

async fn records(log: &dyn EventLog, epoch: u64) -> Vec<AppliedTickRecord> {
    let mut stream = log.read_epoch(ZoneId(ZONE), epoch).await.unwrap();
    let mut out = Vec::new();
    while let Some(r) = stream.next().await {
        out.push(r.unwrap());
    }
    out
}

#[tokio::test]
async fn applied_records_are_readable_in_order_with_contiguous_ticks() {
    let rig = Rig::new();
    let (zone, driver) = rig.start().await;
    zone.handle().send(player(1)).unwrap();
    for _ in 0..20 {
        assert!(matches!(driver.step().await.unwrap(), TickOutcome::Ran(_)));
    }
    let epoch = zone.epoch();
    let watermark = zone.shutdown(WatermarkReason::Shutdown).await.unwrap();
    assert_eq!(watermark.last_tick, Some(Tick(19)));
    assert_eq!(watermark.records, 20);

    let all = records(rig.mem.as_ref(), epoch).await;
    let ticks: Vec<u64> = all.iter().map(|r| r.tick.0).collect();
    assert_eq!(ticks, (0..20).collect::<Vec<_>>(), "one record per tick, idle ones included");
    // Tick 0 applied the fixture's two NPC spawns and the player, in that order.
    let first: Vec<u64> = all[0].commands.iter().map(|c| c.ordinal.0).collect();
    assert_eq!(first, vec![0, 1, 2]);
    assert!(matches!(all[0].commands[0].command, ZoneCommand::SpawnNpc { .. }));
    assert_eq!(all[0].server_time_ms, replay_support::ORIGIN_MS);

    let mut opened = open_epoch(rig.mem.as_ref(), ZoneId(ZONE), epoch)
        .await
        .unwrap();
    assert_eq!(opened.watermark, watermark);
    let mut n = 0;
    while let Some(r) = opened.records.next().await {
        assert_eq!(r.unwrap().tick, Tick(n));
        n += 1;
    }
    assert_eq!(n, 20);
}

#[tokio::test]
async fn replaying_the_log_from_the_snapshot_reproduces_every_output_byte() {
    let rig = Rig::new();
    let (zone, driver) = rig.start().await;
    zone.handle().send(player(1)).unwrap();
    zone.handle().send(player(2)).unwrap();
    driver.step().await.unwrap();
    let walk = ZoneCommand::MoveTo {
        entity: EntityId(Uuid::from_u128(1)),
        dest: Vec2Fixed::from_tiles(60, 45),
    };
    zone.handle()
        .send(ZoneInput::session(EntityId(Uuid::from_u128(1)), SessionGeneration(1), 1, walk))
        .unwrap();
    for _ in 0..30 {
        driver.step().await.unwrap();
    }
    let epoch = zone.epoch();
    zone.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let mut opened = open_epoch(rig.mem.as_ref(), ZoneId(ZONE), epoch)
        .await
        .unwrap();
    let mut state = ZoneState::from_snapshot(opened.snapshot.snapshot.clone()).unwrap();
    let mut compared = 0;
    while let Some(record) = opened.records.next().await {
        let record = record.unwrap();
        let draft = nightfall_api::domain::zone::AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        };
        let rerun = AppliedTickRecord::from_applied(ZoneId(ZONE), &state.run_tick(draft).unwrap());
        assert_eq!(rerun.encode(), record.encode(), "tick {:?} diverged", record.tick);
        compared += record.outputs.len();
    }
    assert!(compared > 10, "the walk produced per-player output to compare");
}

/// Re-runs each record of a complete epoch from its snapshot; returns `(recorded, rerun)`
/// pairs, the re-run with its outputs encoded.
async fn rerun_epoch(
    log: &dyn EventLog,
    epoch: u64,
) -> Vec<(AppliedTickRecord, AppliedTickRecord)> {
    let mut opened = open_epoch(log, ZoneId(ZONE), epoch).await.unwrap();
    let mut state = ZoneState::from_snapshot(opened.snapshot.snapshot.clone()).unwrap();
    let mut pairs = Vec::new();
    while let Some(record) = opened.records.next().await {
        let record = record.unwrap();
        let draft = nightfall_api::domain::zone::AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        };
        let rerun = AppliedTickRecord::from_applied(ZoneId(ZONE), &state.run_tick(draft).unwrap());
        pairs.push((record, rerun));
    }
    pairs
}

/// Flips the last bit of one player's output.
fn flip_one_byte(record: &mut AppliedTickRecord, player: usize) {
    let mut bytes = record.outputs[player].bytes.to_vec();
    *bytes.last_mut().unwrap() ^= 1;
    record.outputs[player].bytes = Bytes::from(bytes);
}

#[tokio::test]
async fn an_oversized_tick_is_logged_with_output_digests_and_the_zone_keeps_running() {
    // A broker that takes at most 4 KiB per record. Twenty players spawning on one spot each
    // see every spawn, so tick 0's full record is several times that.
    const LIMIT: usize = 4096;
    let rig = Rig::with_log(InMemoryEventLog::with_max_record_bytes(LIMIT));
    let (zone, driver) = rig.start().await;
    for n in 1..=20 {
        zone.handle().send(player(n)).unwrap();
    }
    for _ in 0..10 {
        assert!(matches!(driver.step().await.unwrap(), TickOutcome::Ran(_)));
    }
    assert_eq!(rig.faulty.attempts.load(Ordering::SeqCst), 10, "no append was retried");
    let epoch = zone.epoch();
    zone.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let pairs = rerun_epoch(rig.mem.as_ref(), epoch).await;
    assert_eq!(pairs.len(), 10);
    let (busy, busy_rerun) = &pairs[0];
    assert!(busy_rerun.encoded_len() > LIMIT, "the full record would not fit");
    assert!(
        rig.mem.append_applied(busy_rerun).await.is_err(),
        "the log refuses it, so without the bound the zone would stall"
    );
    assert_eq!(busy.output_form, OutputForm::Sha256);
    assert_eq!(busy.outputs.len(), 20);
    assert!(busy.outputs.iter().all(|o| o.bytes.len() == 32));
    assert!(busy.encode().len() <= LIMIT);
    assert_eq!(busy.commands, busy_rerun.commands, "commands are always kept in full");
    assert_eq!(pairs[9].0.output_form, OutputForm::Encoded, "quiet ticks keep full outputs");
    for (record, rerun) in &pairs {
        assert!(record.reproduced_by(rerun), "tick {:?} diverged", record.tick);
    }
}

#[tokio::test]
async fn replay_detects_a_one_byte_divergence_in_a_digested_record() {
    let rig = Rig::with_log(InMemoryEventLog::with_max_record_bytes(4096));
    let (zone, driver) = rig.start().await;
    for n in 1..=20 {
        zone.handle().send(player(n)).unwrap();
    }
    driver.step().await.unwrap();
    let epoch = zone.epoch();
    zone.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let (recorded, rerun) = rerun_epoch(rig.mem.as_ref(), epoch).await.remove(0);
    assert_eq!(recorded.output_form, OutputForm::Sha256);
    assert!(recorded.reproduced_by(&rerun));
    for player in [0, 7, 19] {
        let mut diverged = rerun.clone();
        flip_one_byte(&mut diverged, player);
        assert!(!recorded.reproduced_by(&diverged), "player {player}'s changed byte was missed");
    }
    // A changed command or a corrupted stored digest is a divergence too.
    let mut diverged = rerun.clone();
    diverged.commands.pop();
    assert!(!recorded.reproduced_by(&diverged));
    let mut corrupted = recorded.clone();
    flip_one_byte(&mut corrupted, 3);
    assert!(!corrupted.reproduced_by(&rerun));
}

#[test]
fn bounded_keeps_a_record_that_fits_and_digests_one_that_does_not() {
    let mut record = empty_record(1, 2);
    record.outputs = (0..4_u128)
        .map(|n| PlayerOutput {
            entity: EntityId(Uuid::from_u128(n)),
            bytes: Bytes::from(vec![u8::try_from(n).unwrap(); 200]),
        })
        .collect();
    let len = record.encoded_len();
    assert_eq!(len, record.encode().len());
    assert_eq!(record.clone().bounded(len), record);
    let digested = record.clone().bounded(len - 1);
    assert_eq!(digested, record.with_output_digests());
    assert_eq!(digested.output_form, OutputForm::Sha256);
    assert_eq!(digested.with_output_digests(), digested, "digesting twice changes nothing");
    assert!(digested.encoded_len() < 4 * 60);
    // A full record is never "reproduced" by a re-run that differs from it.
    let mut diverged = record.clone();
    flip_one_byte(&mut diverged, 2);
    assert!(record.reproduced_by(&record));
    assert!(!record.reproduced_by(&diverged));
}

#[test]
fn a_record_with_encoded_outputs_keeps_the_original_wire_format() {
    // Field 8 (output_form) is omitted at its default, so records written before digests
    // existed and records written now with full outputs are byte-identical.
    let mut record = empty_record(1, 2);
    assert_eq!(record.encode(), vec![0x08, 0x07, 0x10, 0x01, 0x18, 0x02]);
    record.outputs.push(PlayerOutput {
        entity: EntityId(Uuid::from_u128(1)),
        bytes: Bytes::from_static(&[0xAA]),
    });
    let mut expected = vec![0x08, 0x07, 0x10, 0x01, 0x18, 0x02, 0x3a, 0x15, 0x0a, 0x10];
    expected.extend_from_slice(Uuid::from_u128(1).as_bytes());
    expected.extend_from_slice(&[0x12, 0x01, 0xAA]);
    assert_eq!(record.encode(), expected);
}

#[tokio::test]
async fn the_snapshot_precedes_the_first_applied_record() {
    let rig = Rig::new();
    let (zone, driver) = rig.start().await;
    driver.step().await.unwrap();
    let epoch = zone.epoch();
    let progress = *zone.progress().borrow();
    zone.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let subjects: Vec<String> = rig.mem.subjects().into_iter().map(|(_, s)| s).collect();
    let prefix = format!("nightfall.zone.{ZONE}.{epoch}");
    assert_eq!(
        subjects,
        vec![
            format!("{prefix}.snapshot"),
            format!("{prefix}.applied"),
            format!("{prefix}.watermark")
        ]
    );
    let row = rig.store.get(ZoneId(ZONE), epoch).await.unwrap().unwrap();
    let stored = rig
        .mem
        .read_snapshot(ZoneId(ZONE), epoch)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.snapshot_seq, stored.seq);
    assert!(progress.first_seq.unwrap() > stored.seq);
    assert_eq!(
        row.snapshot,
        nightfall_api::application::replay_log::encode_snapshot(&stored.snapshot).unwrap(),
        "Postgres holds the same bytes as the log"
    );
    assert!(row.config_hash.starts_with("sha256:"));
    assert_eq!(row.time_origin_ms, replay_support::ORIGIN_MS);
    // The first-seq index update runs off the tick path.
    tokio::time::timeout(Duration::from_secs(5), async {
        while rig
            .store
            .get(ZoneId(ZONE), epoch)
            .await
            .unwrap()
            .unwrap()
            .first_seq
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn every_start_is_a_new_epoch() {
    let rig = Rig::new();
    let (first, _d1) = rig.start().await;
    let e1 = first.epoch();
    first.shutdown(WatermarkReason::Shutdown).await.unwrap();
    let (second, _d2) = rig.start().await;
    assert_eq!(second.epoch(), e1 + 1);
}

#[tokio::test]
async fn the_gate_blocks_tick_n_plus_1_until_n_is_acknowledged() {
    let rig = Rig::new();
    rig.faulty.manual_acks.store(true, Ordering::SeqCst);
    let (zone, driver) = rig.start().await;
    let mut rx = zone.handle().subscribe();

    let step = tokio::spawn({
        let driver = driver.clone();
        async move { driver.step().await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!step.is_finished(), "tick 0 waits for its ack");
    assert!(rx.try_recv().is_err(), "nothing is broadcast before the ack");

    rig.faulty.acks.add_permits(1);
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(0)));
    assert_eq!(rx.recv().await.unwrap().tick, Tick(0));

    let step = tokio::spawn({
        let driver = driver.clone();
        async move { driver.step().await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!step.is_finished(), "tick 1 waits for its own ack");
    assert_eq!(
        rig.mem
            .epoch_status(ZoneId(ZONE), zone.epoch())
            .await
            .unwrap(),
        EpochStatus::Incomplete {
            last_tick: Some(Tick(0))
        }
    );
    rig.faulty.acks.add_permits(1);
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(1)));
}

#[tokio::test]
async fn a_dead_broker_stalls_the_zone_without_releasing_anything_until_it_returns() {
    let rig = Rig::new();
    let (zone, driver) = rig.start().await;
    let mut rx = zone.handle().subscribe();
    driver.step().await.unwrap();
    rx.recv().await.unwrap(); // tick 0 spawned the NPCs

    rig.faulty.down.store(true, Ordering::SeqCst);
    zone.handle().send(player(1)).unwrap();
    let step = tokio::spawn({
        let driver = driver.clone();
        async move { driver.step().await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!step.is_finished(), "the actor stalls inside the gate");
    assert!(rx.try_recv().is_err(), "no AppliedTick is broadcast while unrecorded");
    assert!(rig.faulty.attempts.load(Ordering::SeqCst) > 3, "the gate keeps retrying");
    assert!(!zone.handle().is_paused(), "below max_stall the zone is only stalled");

    rig.faulty.down.store(false, Ordering::SeqCst);
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(1)));
    let t = rx.recv().await.unwrap();
    assert_eq!((t.tick, t.commands.len()), (Tick(1), 1));
    driver.step().await.unwrap();
    let ticks: Vec<u64> = records(rig.mem.as_ref(), zone.epoch())
        .await
        .iter()
        .map(|r| r.tick.0)
        .collect();
    assert_eq!(ticks, vec![0, 1, 2], "no gap and no duplicate after the outage");
}

#[tokio::test]
async fn a_long_outage_pauses_the_zone_and_inputs_are_refused_until_it_recovers() {
    let rig = Rig::new();
    let (zone, driver) = rig
        .start_with(GateConfig {
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(5),
            max_stall: Duration::from_millis(30),
        })
        .await;
    rig.faulty.down.store(true, Ordering::SeqCst);
    let step = tokio::spawn({
        let driver = driver.clone();
        async move { driver.step().await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !zone.handle().is_paused() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        zone.handle().send(player(1)),
        Err(nightfall_api::application::zone_actor::ZoneSendError::Paused(_))
    ));

    rig.faulty.down.store(false, Ordering::SeqCst);
    assert_eq!(step.await.unwrap().unwrap(), TickOutcome::Ran(Tick(0)));
    assert!(!zone.handle().is_paused());
    zone.handle().send(player(1)).unwrap();
}

#[tokio::test]
async fn shutdown_while_stalled_leaves_the_epoch_incomplete() {
    let rig = Rig::new();
    let (zone, driver) = rig.start().await;
    driver.step().await.unwrap();
    rig.faulty.down.store(true, Ordering::SeqCst);
    let _stalled = tokio::spawn({
        let driver = driver.clone();
        async move { driver.step().await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let epoch = zone.epoch();
    // The gate gives up on shutdown; the watermark names only acknowledged ticks.
    let watermark = zone.shutdown(WatermarkReason::Shutdown).await.unwrap();
    assert_eq!(watermark.last_tick, Some(Tick(0)));
    let ticks: Vec<u64> = records(rig.mem.as_ref(), epoch)
        .await
        .iter()
        .map(|r| r.tick.0)
        .collect();
    assert_eq!(ticks, vec![0]);
}

#[tokio::test]
async fn a_process_killed_mid_tick_leaves_the_log_at_tick_n_and_replay_refuses_it() {
    let rig = Rig::new();
    rig.faulty.hang_from_tick.store(3, Ordering::SeqCst);
    let faulty = rig.faulty.clone();
    let store = rig.store.clone();
    // The "process": its own runtime on its own thread, dropped while tick 3 is mid-append.
    let epoch = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let epoch = rt.block_on(async {
            let (ticks, driver) = manual_ticks();
            let zone = ZoneBootstrap::new(
                faulty,
                Some(store),
                Arc::new(FixedClock),
                Arc::new(NoReplayMetrics),
            )
            .with_gate_config(fast_gate())
            .start(&zone_def(ZONE), ticks)
            .await
            .unwrap();
            for _ in 0..3 {
                driver.step().await.unwrap();
            }
            tokio::spawn(async move { driver.step().await });
            tokio::time::sleep(Duration::from_millis(20)).await;
            // No graceful shutdown: the process dies with tick 3 unacknowledged.
            zone.epoch()
        });
        drop(rt); // kills the actor mid-tick
        epoch
    })
    .join()
    .unwrap();

    let status = rig.mem.epoch_status(ZoneId(ZONE), epoch).await.unwrap();
    assert_eq!(
        status,
        EpochStatus::Incomplete {
            last_tick: Some(Tick(2))
        }
    );
    let ticks: Vec<u64> = records(rig.mem.as_ref(), epoch)
        .await
        .iter()
        .map(|r| r.tick.0)
        .collect();
    assert_eq!(ticks, vec![0, 1, 2]);
    let refused = open_epoch(rig.mem.as_ref(), ZoneId(ZONE), epoch).await;
    assert!(matches!(
        refused,
        Err(ReplayError::Incomplete {
            last_tick: Some(Tick(2)),
            ..
        })
    ));
}

#[tokio::test]
async fn an_unknown_epoch_is_missing() {
    let log = InMemoryEventLog::default();
    assert_eq!(log.epoch_status(ZoneId(1), 9).await.unwrap(), EpochStatus::Missing);
    assert!(matches!(open_epoch(&log, ZoneId(1), 9).await, Err(ReplayError::Missing { .. })));
}

fn empty_record(epoch: u64, tick: u64) -> AppliedTickRecord {
    AppliedTickRecord {
        zone: ZoneId(ZONE),
        epoch,
        tick: Tick(tick),
        server_time_ms: 0,
        commands: Vec::new(),
        dispositions: Vec::new(),
        outputs: Vec::new(),
        output_form: OutputForm::Encoded,
    }
}

async fn hand_written_epoch(ticks: &[u64], last: Option<u64>) -> InMemoryEventLog {
    let log = InMemoryEventLog::default();
    let bounds = nightfall_api::domain::zone::ZoneBounds::new(
        Vec2Fixed::from_tiles(0, 0),
        Vec2Fixed::from_tiles(8, 8),
    )
    .unwrap();
    let state = ZoneState::new(
        nightfall_api::domain::zone::ZoneSeed {
            zone: ZoneId(ZONE),
            epoch: 1,
        },
        bounds,
        0,
    );
    log.write_snapshot(&state.snapshot()).await.unwrap();
    for t in ticks {
        log.append_applied(&empty_record(1, *t)).await.unwrap();
    }
    log.write_watermark(&Watermark {
        zone: ZoneId(ZONE),
        epoch: 1,
        last_tick: last.map(Tick),
        records: ticks.len() as u64,
        reason: WatermarkReason::EpochEnd,
    })
    .await
    .unwrap();
    log
}

async fn replay_result(log: &InMemoryEventLog) -> Result<u64, ReplayError> {
    let mut opened = open_epoch(log, ZoneId(ZONE), 1).await?;
    let mut n = 0;
    while let Some(r) = opened.records.next().await {
        r?;
        n += 1;
    }
    Ok(n)
}

#[tokio::test]
async fn replay_detects_gaps_and_truncation() {
    assert_eq!(
        replay_result(&hand_written_epoch(&[0, 1, 2], Some(2)).await)
            .await
            .unwrap(),
        3
    );
    assert!(matches!(
        replay_result(&hand_written_epoch(&[0, 2], Some(2)).await).await,
        Err(ReplayError::Gap {
            expected: Tick(1),
            found: Tick(2)
        })
    ));
    assert!(matches!(
        replay_result(&hand_written_epoch(&[0, 1], Some(4)).await).await,
        Err(ReplayError::Truncated { .. })
    ));
    assert_eq!(
        replay_result(&hand_written_epoch(&[], None).await)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_duplicate_append_of_the_same_tick_is_stored_once() {
    let log = InMemoryEventLog::default();
    let a = log.append_applied(&empty_record(1, 0)).await.unwrap();
    let b = log.append_applied(&empty_record(1, 0)).await.unwrap();
    assert_eq!(a, b);
    assert_eq!(records(&log, 1).await.len(), 1);
}

fn audit_in(n: u64) -> SessionInRecord {
    SessionInRecord {
        session: Uuid::from_u128(42),
        seq: n,
        zone: ZoneId(ZONE),
        epoch: 1,
        tick_seen: Tick(n),
        recv_unix_ms: 1_000 + i64::try_from(n).unwrap(),
        frame: Bytes::from(vec![u8::try_from(n % 256).unwrap(); 3]),
    }
}

fn audit_out(n: u64) -> SessionOutRecord {
    SessionOutRecord {
        session: Uuid::from_u128(42),
        zone: ZoneId(ZONE),
        epoch: 1,
        tick: Tick(n),
        frame: Bytes::from_static(b"out"),
    }
}

#[tokio::test]
async fn session_audit_frames_are_logged_in_order() {
    let log = Arc::new(InMemoryEventLog::default());
    let stop = CancellationToken::new();
    let (writer, task) =
        SessionAuditWriter::spawn(log.clone(), Arc::new(NoReplayMetrics), 1024, stop.clone());
    for n in 0..500 {
        assert!(writer.record_in(audit_in(n)));
        assert!(writer.record_out(audit_out(n)));
    }
    stop.cancel();
    task.await.unwrap();
    assert_eq!(writer.dropped(), 0);
    assert_eq!(log.session_in(), (0..500).map(audit_in).collect::<Vec<_>>());
    assert_eq!(log.session_out(), (0..500).map(audit_out).collect::<Vec<_>>());
}

#[tokio::test]
async fn a_full_audit_buffer_or_a_dead_broker_drops_and_counts_frames() {
    let mem = Arc::new(InMemoryEventLog::default());
    let faulty = Arc::new(FaultyLog::new(mem.clone()));
    faulty.down.store(true, Ordering::SeqCst);
    let stop = CancellationToken::new();
    let (writer, task) =
        SessionAuditWriter::spawn(faulty, Arc::new(NoReplayMetrics), 4, stop.clone());
    let accepted = (0..100).filter(|n| writer.record_in(audit_in(*n))).count() as u64;
    assert!(accepted < 100, "the buffer is bounded");
    stop.cancel();
    task.await.unwrap();
    assert_eq!(writer.dropped(), 100, "every frame is counted, buffered or not");
    assert!(mem.session_in().is_empty());
}

// ---- byte-for-byte round trip ---------------------------------------------------------------

fn entity() -> impl Strategy<Value = EntityId> {
    any::<u128>().prop_map(|n| EntityId(Uuid::from_u128(n)))
}

fn point() -> impl Strategy<Value = Vec2Fixed> {
    (any::<i32>(), any::<i32>())
        .prop_map(|(x, y)| Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y)))
}

fn source() -> impl Strategy<Value = CommandSource> {
    prop_oneof![
        Just(CommandSource::System),
        (entity(), any::<u64>()).prop_map(|(entity, g)| CommandSource::Session {
            entity,
            generation: SessionGeneration(g)
        }),
    ]
}

fn command() -> impl Strategy<Value = ZoneCommand> {
    prop_oneof![
        (entity(), ".{0,16}", point(), any::<u32>(), any::<u64>()).prop_map(
            |(entity, name, pos, s, g)| {
                ZoneCommand::SpawnPlayer {
                    entity,
                    name,
                    pos,
                    speed: Speed::from_milli_tiles_per_tick(s),
                    generation: SessionGeneration(g),
                }
            }
        ),
        (".{0,16}", point(), any::<u32>()).prop_map(|(name, pos, s)| ZoneCommand::SpawnNpc {
            name,
            pos,
            speed: Speed::from_milli_tiles_per_tick(s),
        }),
        entity().prop_map(|entity| ZoneCommand::Despawn { entity }),
        (entity(), any::<u64>()).prop_map(|(entity, g)| ZoneCommand::ReplaceSession {
            entity,
            generation: SessionGeneration(g)
        }),
        (entity(), point()).prop_map(|(entity, dest)| ZoneCommand::MoveTo { entity, dest }),
        entity().prop_map(|entity| ZoneCommand::StopMove { entity }),
    ]
}

fn reason() -> impl Strategy<Value = RejectReason> {
    prop_oneof![
        Just(RejectReason::UnknownEntity),
        Just(RejectReason::OutOfBounds),
        Just(RejectReason::TooFar),
        Just(RejectReason::AlreadyExists),
        Just(RejectReason::NotPermitted),
        Just(RejectReason::StaleSession),
        Just(RejectReason::NotAPlayer),
    ]
}

fn disposition() -> impl Strategy<Value = Disposition> {
    (any::<u64>(), source(), any::<Option<u32>>(), any::<u64>(), reason()).prop_map(
        |(o, source, seq, t, reason)| Disposition {
            ordinal: Ordinal(o),
            source,
            seq,
            tick_seen: Tick(t),
            reason,
        },
    )
}

fn output() -> impl Strategy<Value = ObserverOutput> {
    let speed = any::<u32>().prop_map(Speed::from_milli_tiles_per_tick);
    prop_oneof![
        disposition().prop_map(ObserverOutput::Rejected),
        (
            any::<u64>(),
            entity(),
            any::<bool>(),
            ".{0,16}",
            point(),
            proptest::option::of(point()),
            speed.clone(),
            any::<u64>()
        )
            .prop_map(|(t, entity, npc, name, pos, dest, speed, g)| {
                ObserverOutput::Event(ZoneEvent::EntitySpawn {
                    tick: Tick(t),
                    entity,
                    kind: if npc {
                        EntityKind::Npc
                    } else {
                        EntityKind::Player
                    },
                    name,
                    pos,
                    dest,
                    speed,
                    generation: SessionGeneration(g),
                })
            }),
        (any::<u64>(), entity(), point(), proptest::option::of(point()), speed).prop_map(
            |(t, entity, pos, dest, speed)| ObserverOutput::Event(ZoneEvent::EntityMove {
                tick: Tick(t),
                entity,
                pos,
                dest,
                speed,
            })
        ),
        (any::<u64>(), entity()).prop_map(|(t, entity)| ObserverOutput::Event(
            ZoneEvent::EntityDespawn {
                tick: Tick(t),
                entity,
            }
        )),
    ]
}

fn record() -> impl Strategy<Value = (AppliedTickRecord, Vec<Vec<ObserverOutput>>)> {
    let cmd = (any::<u64>(), source(), any::<Option<u32>>(), command()).prop_map(
        |(o, source, seq, command)| AppliedCommand {
            ordinal: Ordinal(o),
            source,
            seq,
            command,
        },
    );
    (
        any::<u32>(),
        any::<u64>(),
        any::<u64>(),
        any::<i64>(),
        proptest::collection::vec(cmd, 0..8),
        proptest::collection::vec(disposition(), 0..4),
        proptest::collection::vec((entity(), proptest::collection::vec(output(), 0..6)), 0..4),
    )
        .prop_map(|(zone, epoch, tick, ms, commands, dispositions, outs)| {
            let outputs = outs
                .iter()
                .map(|(entity, items)| PlayerOutput {
                    entity: *entity,
                    bytes: encode_outputs(items),
                })
                .collect();
            let record = AppliedTickRecord {
                zone: ZoneId(zone),
                epoch,
                tick: Tick(tick),
                server_time_ms: ms,
                commands,
                dispositions,
                outputs,
                output_form: OutputForm::Encoded,
            };
            (record, outs.into_iter().map(|(_, items)| items).collect())
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn applied_tick_records_round_trip_byte_for_byte((record, outputs) in record()) {
        let bytes = record.encode();
        let decoded = AppliedTickRecord::decode(&bytes).unwrap();
        prop_assert_eq!(&decoded, &record);
        prop_assert_eq!(decoded.encode(), bytes);
        for (player, items) in record.outputs.iter().zip(outputs) {
            prop_assert_eq!(decode_outputs(&player.bytes).unwrap(), items.clone());
            prop_assert_eq!(encode_outputs(&items), player.bytes.clone());
        }
    }

    #[test]
    fn digested_records_round_trip_and_match_their_full_form((record, _outputs) in record()) {
        let digested = record.with_output_digests();
        let bytes = digested.encode();
        let decoded = AppliedTickRecord::decode(&bytes).unwrap();
        prop_assert_eq!(&decoded, &digested);
        prop_assert_eq!(decoded.encode(), bytes);
        prop_assert_eq!(decoded.output_form, OutputForm::Sha256);
        prop_assert!(decoded.reproduced_by(&record));
    }

    #[test]
    fn audit_frames_round_trip_byte_for_byte(n in any::<u64>(), frame in proptest::collection::vec(any::<u8>(), 0..64)) {
        let mut r = audit_in(n % 1000);
        r.frame = Bytes::from(frame);
        let bytes = r.encode();
        prop_assert_eq!(SessionInRecord::decode(&bytes).unwrap(), r);
        let o = audit_out(n);
        prop_assert_eq!(SessionOutRecord::decode(&o.encode()).unwrap(), o);
    }
}

#[test]
fn garbage_is_a_codec_error_not_a_panic() {
    assert!(AppliedTickRecord::decode(&[0xff, 0xff, 0xff]).is_err());
    // A command with no kind.
    assert!(AppliedTickRecord::decode(&[0x2a, 0x02, 0x08, 0x01]).is_err());
    // An unknown output form (field 8 = 7).
    let mut bytes = empty_record(1, 2).encode();
    bytes.extend_from_slice(&[0x40, 0x07]);
    assert!(AppliedTickRecord::decode(&bytes).is_err());
    // A digest that is not 32 bytes.
    let mut short = empty_record(1, 2);
    short.output_form = OutputForm::Sha256;
    short.outputs.push(PlayerOutput {
        entity: EntityId(Uuid::from_u128(1)),
        bytes: Bytes::from_static(&[1, 2, 3]),
    });
    assert!(AppliedTickRecord::decode(&short.encode()).is_err());
    // Output bytes in a record flagged as digested (or a digest in a full one).
    let mut full = short.clone();
    full.output_form = OutputForm::Encoded;
    let mut bytes = full.encode();
    bytes.extend_from_slice(&[0x40, 0x01]);
    assert!(AppliedTickRecord::decode(&bytes).is_err());
}
