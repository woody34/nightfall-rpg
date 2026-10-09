//! The replay tool (Story 3.3): the checked-in two-player recording replays with zero
//! divergence; a changed output byte is reported at its tick and session; float- and
//! clock-based test doubles of the zone are caught; a fresh multi-session capture round-trips
//! through the `.nfr` format and replays; the binary's exit codes.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

mod replay_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use nightfall_api::application::replay::{
    verify_epoch, verify_with, Mismatch, ReplayReport, TickRunner, VerifyError, VerifyOptions,
};
use nightfall_api::application::replay_log::{
    open_epoch, EventLog, GateConfig, NoReplayMetrics, WatermarkReason,
};
use nightfall_api::application::zone_actor::manual_ticks;
use nightfall_api::application::zone_bootstrap::ZoneBootstrap;
use nightfall_api::domain::zone::{
    AppliedTick, AppliedTickDraft, EntityId, Fixed, ObserverOutput, RejectReason,
    SessionGeneration, Speed, TickError, Vec2Fixed, ZoneCommand, ZoneEvent, ZoneId, ZoneInput,
    ZoneSnapshot, ZoneState,
};
use nightfall_api::infrastructure::eventlog::{InMemoryEventLog, JetStreamEventLog, Recording};
use nightfall_api::infrastructure::outbox::JetStreamPublisher;
use nightfall_api::infrastructure::telemetry::Metrics;
use replay_support::{unique_zone, zone_def, FixedClock};
use uuid::Uuid;

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/two-players-v4.nfr")
}

fn fixture() -> Recording {
    Recording::read(&fixture_path()).unwrap()
}

async fn replay_with<R: TickRunner>(
    rec: Recording,
    runner: impl FnOnce(ZoneSnapshot) -> R,
    opts: VerifyOptions,
) -> Result<ReplayReport, VerifyError> {
    let (zone, epoch) = (rec.snapshot.seed.zone, rec.snapshot.seed.epoch);
    let log = rec.into_log().await.unwrap();
    let opened = open_epoch(&log, zone, epoch).await.unwrap();
    let r = runner(opened.snapshot.snapshot.clone());
    verify_with(r, opened.records, opts).await
}

async fn replay(rec: Recording) -> Result<ReplayReport, VerifyError> {
    replay_with(rec, |s| ZoneState::from_snapshot(s).unwrap(), VerifyOptions::default()).await
}

fn diverged(
    r: Result<ReplayReport, VerifyError>,
) -> Box<nightfall_api::application::replay::Divergence> {
    match r {
        Err(VerifyError::Diverged(d)) => d,
        other => panic!("expected a divergence, got {other:?}"),
    }
}

/// Flips the last byte of one player's output in the record of `tick`; returns the player.
fn flip_output_byte(rec: &mut Recording, tick: usize, player: usize) -> EntityId {
    let out = &mut rec.records[tick].outputs[player];
    let mut bytes = out.bytes.to_vec();
    *bytes.last_mut().unwrap() ^= 0x01;
    out.bytes = Bytes::from(bytes);
    out.entity
}

/// The index of a record in the middle of the fixture with output for at least two players.
fn busy_tick(rec: &Recording) -> usize {
    let busy: Vec<usize> = (0..rec.records.len())
        .filter(|&i| rec.records[i].outputs.len() >= 2)
        .collect();
    busy[busy.len() / 2]
}

#[tokio::test]
async fn the_recorded_two_player_session_replays_with_zero_divergence() {
    let rec = fixture();
    let records = rec.watermark.records;
    let commands: Vec<_> = rec.records.iter().flat_map(|r| &r.commands).collect();
    let rejected: Vec<_> = rec.records.iter().flat_map(|r| &r.dispositions).collect();
    // What the recording covers (examples/record_session.rs).
    let spawns = commands
        .iter()
        .filter(|c| matches!(c.command, ZoneCommand::SpawnPlayer { .. }))
        .count();
    let despawns = commands
        .iter()
        .filter(|c| matches!(c.command, ZoneCommand::Despawn { .. }))
        .count();
    assert_eq!((spawns, despawns), (2, 2), "both players join and leave");
    assert!(commands
        .iter()
        .any(|c| matches!(c.command, ZoneCommand::StopMove { .. })));
    assert!(
        commands
            .iter()
            .filter(|c| matches!(c.command, ZoneCommand::MoveTo { .. }))
            .count()
            >= 4
    );
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].reason, RejectReason::OutOfBounds);

    let report = replay(rec).await.unwrap();
    assert_eq!(report.ticks, records);
    assert_eq!(report.players, 2);
    assert!(report.bytes_compared > 1000, "{report:?}");
    assert_eq!(report.digest_only, 0);
}

#[tokio::test]
async fn selecting_a_session_compares_only_its_outputs_but_applies_every_command() {
    let mut rec = fixture();
    let tick = busy_tick(&rec);
    let other = flip_output_byte(&mut rec, tick, 1);
    let mine = rec.records[tick].outputs[0].entity;
    let opts = VerifyOptions {
        session: Some(mine),
        ..VerifyOptions::default()
    };
    let report = replay_with(rec.clone(), |s| ZoneState::from_snapshot(s).unwrap(), opts)
        .await
        .unwrap();
    assert_eq!(report.players, 1, "the other session's (corrupted) output is not compared");
    let opts = VerifyOptions {
        session: Some(other),
        ..VerifyOptions::default()
    };
    let d = diverged(replay_with(rec, |s| ZoneState::from_snapshot(s).unwrap(), opts).await);
    assert_eq!(d.session(), Some(other));
}

#[tokio::test]
async fn a_changed_output_byte_is_reported_at_its_tick_and_session() {
    let mut rec = fixture();
    let tick = busy_tick(&rec);
    let entity = flip_output_byte(&mut rec, tick, 1);
    let want_tick = rec.records[tick].tick;

    let d = diverged(replay(rec).await);
    assert_eq!(d.tick(), want_tick);
    assert_eq!(d.session(), Some(entity));
    assert_eq!(d.mismatch, Mismatch::Output { entity });
    let report = d.to_string();
    assert!(report.contains(&format!("divergence at tick {}", want_tick.0)), "{report}");
    assert!(report.contains(&entity.to_string()), "{report}");
    assert!(report.contains("produced ("), "{report}");
}

#[tokio::test]
async fn changed_events_or_state_nobody_observed_are_divergences() {
    // Off-AOI facts live only in the record's events; invisible state only in its digest.
    let mut rec = fixture();
    let tick = busy_tick(&rec);
    let mut events = rec.records[tick].events.to_vec();
    let last = events.len() - 1;
    events[last] ^= 0x01;
    rec.records[tick].events = Bytes::from(events);
    let d = diverged(replay(rec).await);
    assert_eq!(d.mismatch, Mismatch::Events);
    assert!(d.to_string().contains("zone events differ"), "{d}");

    let mut rec = fixture();
    let mut digest = rec.records[0].state_digest.to_vec();
    digest[0] ^= 0x01;
    rec.records[0].state_digest = Bytes::from(digest);
    let d = diverged(replay(rec).await);
    assert_eq!(d.mismatch, Mismatch::StateDigest);
    assert_eq!(d.tick(), rec_first_tick());
}

fn rec_first_tick() -> nightfall_api::domain::zone::Tick {
    fixture().records[0].tick
}

/// A zone whose movement is integrated in `f32` (normalised direction times speed, position
/// kept as a float between ticks, rounded for output): the float path the determinism rules
/// forbid. Its state stays the real one; only what players are sent differs.
struct FloatMovement {
    zone: ZoneState,
    pos: BTreeMap<EntityId, (f32, f32)>,
}

#[allow(clippy::float_arithmetic, clippy::suboptimal_flops)]
impl FloatMovement {
    fn new(zone: ZoneState) -> Self {
        Self {
            zone,
            pos: BTreeMap::new(),
        }
    }

    /// One float step from the entity's float position (or its integer one) toward `dest`.
    fn step(
        &mut self,
        id: EntityId,
        before: Vec2Fixed,
        dest: Vec2Fixed,
        speed: Speed,
    ) -> Vec2Fixed {
        let f = |v: Fixed| v.raw() as f32;
        let (x, y) = self
            .pos
            .get(&id)
            .copied()
            .unwrap_or((f(before.x), f(before.y)));
        let (dx, dy) = (f(dest.x) - x, f(dest.y) - y);
        let len = (dx * dx + dy * dy).sqrt();
        let s = speed.milli_tiles_per_tick() as f32;
        let next = if len <= s {
            (f(dest.x), f(dest.y))
        } else {
            (x + dx / len * s, y + dy / len * s)
        };
        self.pos.insert(id, next);
        Vec2Fixed::new(
            Fixed::from_raw(next.0.round() as i32),
            Fixed::from_raw(next.1.round() as i32),
        )
    }
}

impl TickRunner for FloatMovement {
    fn replay_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        let before: BTreeMap<EntityId, Vec2Fixed> =
            self.zone.entities().map(|e| (e.id, e.pos)).collect();
        let mut t = self.zone.run_tick(draft)?;
        let mut moved = BTreeMap::new();
        for ev in &t.events {
            if let ZoneEvent::EntityMove {
                entity,
                pos,
                dest,
                speed,
                ..
            } = ev
            {
                // Still walking: the float step. Arrived or stopped: the exact position.
                let p = if let Some(d) = dest {
                    let start = before.get(entity).copied().unwrap_or(*pos);
                    self.step(*entity, start, *d, *speed)
                } else {
                    self.pos.remove(entity);
                    *pos
                };
                moved.insert(*entity, p);
            }
        }
        for items in t.outputs.values_mut() {
            for item in items {
                if let ObserverOutput::Event(ZoneEvent::EntityMove { entity, pos, .. }) = item {
                    if let Some(p) = moved.get(entity) {
                        *pos = *p;
                    }
                }
            }
        }
        Ok(t)
    }

    fn boundary_snapshot(&self) -> ZoneSnapshot {
        self.zone.snapshot()
    }
}

/// A zone that stamps ticks with the wall clock instead of `time_origin_ms + tick * 100`.
struct WallClockTime(ZoneState);

impl TickRunner for WallClockTime {
    fn replay_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        let mut t = self.0.run_tick(draft)?;
        t.server_time_ms = chrono::Utc::now().timestamp_millis();
        Ok(t)
    }

    fn boundary_snapshot(&self) -> ZoneSnapshot {
        self.0.snapshot()
    }
}

#[tokio::test]
async fn a_float_movement_path_is_caught() {
    let d = diverged(
        replay_with(
            fixture(),
            |s| FloatMovement::new(ZoneState::from_snapshot(s).unwrap()),
            VerifyOptions::default(),
        )
        .await,
    );
    assert!(matches!(d.mismatch, Mismatch::Output { .. }), "{d}");
    let (_, ordinal) = d.first_difference().unwrap();
    assert_eq!(ordinal, None, "an event differs, not an ack");
}

#[tokio::test]
async fn a_wall_clock_timestamp_is_caught_on_the_first_tick() {
    let d = diverged(
        replay_with(
            fixture(),
            |s| WallClockTime(ZoneState::from_snapshot(s).unwrap()),
            VerifyOptions::default(),
        )
        .await,
    );
    assert_eq!(d.tick().0, 0);
    assert!(matches!(d.mismatch, Mismatch::ServerTime { .. }), "{d}");
}

fn spawn(n: u128, x: i32, y: i32) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: EntityId::from_uuid(Uuid::from_u128(n)),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: None,
    })
}

fn intent(n: u128, seq: u32, command: ZoneCommand) -> ZoneInput {
    ZoneInput::session(EntityId::from_uuid(Uuid::from_u128(n)), SessionGeneration(1), seq, command)
}

fn walk(n: u128, x: i32, y: i32) -> ZoneCommand {
    ZoneCommand::MoveTo {
        entity: EntityId::from_uuid(Uuid::from_u128(n)),
        dest: Vec2Fixed::from_tiles(x, y),
    }
}

#[tokio::test]
async fn a_fresh_multi_session_capture_round_trips_through_a_file_and_replays() {
    let log = Arc::new(InMemoryEventLog::default());
    let gate = GateConfig {
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
        max_stall: Duration::from_secs(60),
    };
    let (ticks, driver) = manual_ticks();
    let zone =
        ZoneBootstrap::new(log.clone(), None, Arc::new(FixedClock), Arc::new(NoReplayMetrics))
            .with_gate_config(gate)
            .start(&zone_def(9), ticks)
            .await
            .unwrap();
    let h = zone.handle().clone();
    for n in 1..=3 {
        h.send(spawn(n, 30 + i32::try_from(n).unwrap(), 30))
            .unwrap();
    }
    driver.step().await.unwrap();
    h.send(intent(1, 1, walk(1, 50, 41))).unwrap();
    h.send(intent(2, 1, walk(2, 999, 30))).unwrap(); // out of bounds
    h.send(intent(3, 1, walk(3, 10, 10))).unwrap();
    for _ in 0..5 {
        driver.step().await.unwrap();
    }
    let entity = EntityId::from_uuid(Uuid::from_u128(3));
    h.send(intent(3, 2, ZoneCommand::StopMove { entity }))
        .unwrap();
    for _ in 0..20 {
        driver.step().await.unwrap();
    }
    h.send(ZoneInput::system(ZoneCommand::Despawn { entity }))
        .unwrap();
    driver.step().await.unwrap();
    let epoch = zone.epoch();
    zone.shutdown(WatermarkReason::Shutdown).await.unwrap();

    let rec = Recording::export(log.as_ref(), ZoneId(9), epoch)
        .await
        .unwrap();
    let back = Recording::from_bytes(&rec.to_bytes().unwrap()).unwrap();
    assert_eq!(back, rec);
    let log = back.into_log().await.unwrap();
    let report =
        verify_epoch(open_epoch(&log, ZoneId(9), epoch).await.unwrap(), VerifyOptions::default())
            .await
            .unwrap();
    assert_eq!(report.ticks, 27);
    assert_eq!(report.players, 3);
}

#[test]
fn a_recording_of_another_format_version_is_refused() {
    let mut bytes = std::fs::read(fixture_path()).unwrap();
    bytes[8] = bytes[8].wrapping_add(1);
    let err = Recording::from_bytes(&bytes).unwrap_err().to_string();
    assert!(err.contains("format 2 is not supported"), "{err}");
    assert!(Recording::from_bytes(b"garbage").is_err());
}

// ---- the binary ---------------------------------------------------------------------------

fn tool(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn tmp(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", Uuid::now_v7().simple()))
}

#[test]
fn the_tool_exits_0_on_the_fixture() {
    let path = fixture_path();
    let (code, stdout, stderr) = tool(&["--source", "file", "--file", path.to_str().unwrap()]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("match."), "{stdout}");
    assert!(stdout.contains("2 players"), "{stdout}");
}

#[test]
fn the_tool_exits_1_and_names_the_tick_and_session_of_a_changed_byte() {
    let mut rec = fixture();
    let tick = busy_tick(&rec);
    let entity = flip_output_byte(&mut rec, tick, 0);
    let want = rec.records[tick].tick.0;
    let file = tmp("mutated.nfr");
    rec.write(&file).unwrap();
    let out = tmp("divergence");

    let (code, stdout, stderr) = tool(&[
        "--source",
        "file",
        "--file",
        file.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.contains(&format!("divergence at tick {want}\n")), "{stdout}");
    assert!(stdout.contains(&format!("session (player entity): {entity}")), "{stdout}");
    for f in [
        "divergence.txt",
        "recorded-output.pb",
        "produced-output.pb",
        "state-before.json",
    ] {
        assert!(out.join(f).exists(), "{f} not dumped");
    }
}

#[test]
fn the_tool_exits_2_on_bad_usage() {
    assert_eq!(tool(&["--zone", "1"]).0, 2);
    assert_eq!(tool(&["--source", "file"]).0, 2);
    assert_eq!(tool(&["--bogus", "x"]).0, 2);
}

#[tokio::test]
async fn the_tool_exits_3_on_an_incomplete_jetstream_epoch() {
    let Ok(url) = std::env::var("NATS_URL") else {
        return;
    };
    let client = async_nats::connect(url.as_str()).await.unwrap();
    JetStreamPublisher::connect(client.clone()).await.unwrap();
    let log = JetStreamEventLog::connect(client, Metrics::detached())
        .await
        .unwrap();
    let def = zone_def(unique_zone());
    let state = ZoneState::new(
        nightfall_api::domain::zone::ZoneSeed {
            zone: def.zone,
            epoch: 1,
        },
        def.bounds,
        replay_support::ORIGIN_MS,
    );
    log.write_snapshot(&state.snapshot()).await.unwrap();
    let zone = def.zone.0.to_string();

    let (code, _, stderr) = tool(&["--zone", &zone, "--latest", "--nats", &url]);
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("incomplete"), "{stderr}");
    let out = tmp("incomplete.nfr");
    let (code, _, _) = tool(&[
        "export",
        "--zone",
        &zone,
        "--epoch",
        "1",
        "--nats",
        &url,
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 3, "export refuses it too");
    assert!(!out.exists());
}

// ---- Phase 1 E6.1: real-socket fight recording ---------------------------------------------

fn fight_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sessions/two-players-fight-v2.nfr")
}

fn fight() -> Recording {
    Recording::read(&fight_path()).unwrap()
}

fn events(r: &nightfall_api::application::replay_log::AppliedTickRecord) -> Vec<ZoneEvent> {
    nightfall_api::application::replay_log::decode_events(&r.events).unwrap()
}

/// A real tick boundary before a damaging swing lands, with both players present. Keeping
/// the in-flight swing, hate, RNG, scheduler and AOI is essential to the suffix replay.
fn mid_fight() -> Recording {
    let mut rec = fight();
    let index = rec
        .records
        .iter()
        .position(|r| {
            events(r)
                .iter()
                .any(|e| matches!(e, ZoneEvent::AttackResult { damage, .. } if *damage > 0))
        })
        .unwrap();
    let mut zone = ZoneState::from_snapshot(rec.snapshot.clone()).unwrap();
    for r in &rec.records[..index] {
        zone.run_tick(AppliedTickDraft {
            epoch: r.epoch,
            tick: r.tick,
            commands: r.commands.clone(),
        })
        .unwrap();
    }
    rec.snapshot = zone.snapshot();
    assert_eq!(
        rec.snapshot
            .entities
            .iter()
            .filter(|e| e.kind == nightfall_api::domain::zone::EntityKind::Player)
            .count(),
        2
    );
    assert!(rec
        .snapshot
        .entities
        .iter()
        .any(|e| e.combat.as_ref().is_some_and(|c| c.swing.is_some())));
    assert!(!rec.snapshot.hate.is_empty());
    rec.records.drain(..index);
    rec.watermark.records = rec.records.len() as u64;
    rec
}

#[tokio::test]
async fn fight_replays_from_epoch_and_mid_fight_in_bytes_and_digests() {
    for rec in [fight(), mid_fight()] {
        let count = rec.watermark.records;
        let roundtrip = Recording::from_bytes(&rec.to_bytes().unwrap()).unwrap();
        assert_eq!(roundtrip, rec);
        let report = replay(roundtrip).await.unwrap();
        assert_eq!(report.ticks, count);
        assert_eq!(report.players, 2);
        assert!(report.bytes_compared > 1000);
        assert_eq!(report.digest_only, 0);
        let mut digested = rec;
        digested.records = digested
            .records
            .iter()
            .map(nightfall_api::application::replay_log::AppliedTickRecord::with_output_digests)
            .collect();
        let report = replay(digested).await.unwrap();
        assert_eq!(report.ticks, count);
        assert!(report.digest_only > 0);
        assert_eq!(report.bytes_compared, 0);
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One chronological lifecycle audit of the captured session.
fn fight_covers_combat_lifecycle_and_codec_facts() {
    use nightfall_api::application::replay_log::{
        decode_outputs, encode_events, encode_outputs, AppliedTickRecord,
    };
    use nightfall_api::domain::zone::{AttackOutcome, CombatRole, EntityKind, Intention};
    let rec = fight();
    let all: Vec<_> = rec.records.iter().flat_map(events).collect();
    let commands: Vec<_> = rec.records.iter().flat_map(|r| &r.commands).collect();
    let players: Vec<_> = commands
        .iter()
        .filter_map(|c| {
            if let ZoneCommand::SpawnPlayer { entity, .. } = c.command {
                Some(entity)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(players.len(), 2);
    assert_eq!(
        commands
            .iter()
            .filter(|c| matches!(c.command, ZoneCommand::Despawn { .. }))
            .count(),
        2
    );
    for player in &players {
        assert!(all
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if attacker == player)));
    }
    assert!(commands
        .iter()
        .any(|c| matches!(c.command, ZoneCommand::SetTarget { .. })));
    assert!(commands
        .iter()
        .any(|c| matches!(c.command, ZoneCommand::StopAttack { .. })));
    assert!(commands
        .iter()
        .any(|c| matches!(c.command, ZoneCommand::StopMove { .. })));
    let refused: Vec<_> = rec.records.iter().flat_map(|r| &r.dispositions).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].reason, RejectReason::OutOfBounds);
    assert!(all.iter().any(|e| matches!(
        e,
        ZoneEvent::NpcIntentionChanged {
            to: Intention::ReturnHome,
            ..
        }
    )));
    let killer = all
        .iter()
        .find_map(|e| {
            if let ZoneEvent::LevelUp {
                entity, level: 2, ..
            } = e
            {
                Some(*entity)
            } else {
                None
            }
        })
        .unwrap();
    assert!(all.iter().any(|e| matches!(e, ZoneEvent::XpGained { entity, amount: 68, total: 68, .. } if *entity == killer)));
    assert!(all.iter().any(|e| matches!(e, ZoneEvent::StatsChanged { entity, hp: 0, level: 1, xp, .. } if *entity == killer && *xp < 68)));
    let mut zone = ZoneState::from_snapshot(rec.snapshot.clone()).unwrap();
    let (mut chase, mut social, mut respawn, mut disconnect, mut decay, mut npc_respawn) =
        (false, false, false, false, false, false);
    let mut corpse = None;
    for r in &rec.records {
        // The fight itself is the codec fixture: every command, public/private event,
        // internal AI/hate fact and owner-only output must round-trip canonically.
        assert_eq!(AppliedTickRecord::decode(&r.encode()).unwrap(), *r);
        assert_eq!(encode_events(&events(r)), r.events);
        for out in &r.outputs {
            assert_eq!(encode_outputs(&decode_outputs(&out.bytes).unwrap()), out.bytes);
        }
        let before = zone.snapshot();
        for c in &r.commands {
            if let ZoneCommand::Despawn { entity } = c.command {
                disconnect |= before
                    .entities
                    .iter()
                    .any(|e| e.id == entity && e.combat.as_ref().is_some_and(|c| c.auto_attack));
            }
        }
        zone.run_tick(AppliedTickDraft {
            epoch: r.epoch,
            tick: r.tick,
            commands: r.commands.clone(),
        })
        .unwrap();
        let after = zone.snapshot();
        chase |= after.entities.iter().any(|e| {
            e.kind == EntityKind::Player
                && e.dest.is_some()
                && e.combat.as_ref().is_some_and(|c| c.auto_attack)
        });
        // A clan helper enters Attack outside its own think phase without taking damage;
        // it cannot have acquired the target through its independent proximity scan.
        social |= events(r).iter().any(|e| {
            if let ZoneEvent::NpcIntentionChanged {
                entity,
                to: Intention::Attack,
                ..
            } = e
            {
                r.tick.0 % 10 != nightfall_api::domain::zone::NpcAi::think_phase(*entity)
                    && after.hate.iter().any(|h| {
                        h.npc == *entity
                            && h.ledger
                                .iter()
                                .any(|(_, row)| row.hate > 0 && row.damage == 0)
                    })
            } else {
                false
            }
        });
        for e in events(r) {
            if let ZoneEvent::EntityDied {
                entity,
                killer: Some(k),
                ..
            } = e
            {
                if k == killer {
                    corpse = Some(entity);
                }
            }
            if let ZoneEvent::EntityRespawned {
                entity, position, ..
            } = e
            {
                if entity == killer {
                    assert_eq!(position, Vec2Fixed::from_tiles(126, 126));
                    let c = after
                        .entities
                        .iter()
                        .find(|e| e.id == entity)
                        .unwrap()
                        .combat
                        .as_ref()
                        .unwrap();
                    assert!(c.protected_until.is_some_and(|t| t > r.tick));
                    assert!(matches!(c.role, CombatRole::Player { xp, .. } if xp < 68));
                    respawn = true;
                }
            }
        }
        if let Some(id) = corpse {
            decay |= !after.entities.iter().any(|e| e.id == id);
        }
        npc_respawn |= after.entities.iter().any(|e| {
            e.kind == EntityKind::Npc
                && e.combat
                    .as_ref()
                    .is_some_and(|c| c.incarnation > 1 && c.hp > 0)
        });
    }
    assert!(chase && social && respawn && disconnect && decay && npc_respawn,
        "chase={chase} social={social} respawn={respawn} disconnect={disconnect} decay={decay} npc_respawn={npc_respawn}");
    for outcome in [AttackOutcome::Hit, AttackOutcome::Miss, AttackOutcome::Crit] {
        assert!(
            all.iter()
                .any(|e| matches!(e, ZoneEvent::AttackResult { outcome: o, .. } if *o == outcome)),
            "missing {outcome:?}"
        );
    }
}

fn tool_diverges(rec: &Recording, tick: nightfall_api::domain::zone::Tick) {
    let file = tmp("fight-mutation.nfr");
    rec.write(&file).unwrap();
    let (code, stdout, stderr) = tool(&["--source", "file", "--file", file.to_str().unwrap()]);
    std::fs::remove_file(file).unwrap();
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stdout.contains(&format!("divergence at tick {}\n", tick.0)), "{stdout}{stderr}");
}

#[tokio::test]
async fn fight_changed_damage_coefficient_diverges_at_the_impact_tick() {
    let mut rec = mid_fight();
    rec.snapshot
        .rules
        .as_mut()
        .unwrap()
        .constants
        .damage_coefficient *= 2;
    let tick = rec.records[0].tick;
    let d = diverged(replay(rec.clone()).await);
    assert_eq!(d.tick(), tick);
    assert_eq!(
        d.mismatch,
        Mismatch::Events,
        "must change combat facts, not merely the rules digest"
    );
    tool_diverges(&rec, tick);
}

#[tokio::test]
async fn fight_skipped_rng_draw_diverges_at_the_impact_tick() {
    let mut rec = mid_fight();
    // Restore as if the preceding u32 draw had been skipped; retain every recorded command.
    rec.snapshot.rng.word_pos -= 1;
    let tick = rec.records[0].tick;
    assert_eq!(diverged(replay(rec.clone()).await).tick(), tick);
    tool_diverges(&rec, tick);
}

#[tokio::test]
async fn fight_dropped_ai_intention_diverges_at_its_tick() {
    let mut rec = fight();
    let r = rec
        .records
        .iter_mut()
        .find(|r| {
            events(r)
                .iter()
                .any(|e| matches!(e, ZoneEvent::NpcIntentionChanged { .. }))
        })
        .unwrap();
    let mut facts = events(r);
    let i = facts
        .iter()
        .position(|e| matches!(e, ZoneEvent::NpcIntentionChanged { .. }))
        .unwrap();
    facts.remove(i);
    r.events = nightfall_api::application::replay_log::encode_events(&facts);
    let tick = r.tick;
    let d = diverged(replay(rec.clone()).await);
    assert_eq!(d.tick(), tick);
    assert_eq!(d.mismatch, Mismatch::Events);
    tool_diverges(&rec, tick);
}

#[tokio::test]
async fn fight_altered_off_aoi_fact_diverges_without_any_player_output() {
    let mut rec = fight();
    let r = rec
        .records
        .iter_mut()
        .find(|r| {
            r.outputs.is_empty()
                && events(r).iter().any(|e| {
                    matches!(
                        e,
                        ZoneEvent::EntitySpawn {
                            combat: Some(_),
                            ..
                        }
                    )
                })
        })
        .unwrap();
    let mut facts = events(r);
    let e = facts
        .iter_mut()
        .find(|e| {
            matches!(
                e,
                ZoneEvent::EntitySpawn {
                    combat: Some(_),
                    ..
                }
            )
        })
        .unwrap();
    if let ZoneEvent::EntitySpawn {
        combat: Some(view), ..
    } = e
    {
        view.hp -= 1;
    }
    r.events = nightfall_api::application::replay_log::encode_events(&facts);
    let tick = r.tick;
    let d = diverged(replay(rec.clone()).await);
    assert_eq!(d.tick(), tick);
    assert_eq!(d.mismatch, Mismatch::Events);
    tool_diverges(&rec, tick);
}

#[test]
fn fight_tool_replays_epoch_and_mid_fight_and_refuses_gaps_and_truncation() {
    for rec in [fight(), mid_fight()] {
        let file = tmp("fight-suffix.nfr");
        rec.write(&file).unwrap();
        let (code, stdout, stderr) = tool(&["--source", "file", "--file", file.to_str().unwrap()]);
        assert_eq!(code, 0, "{stdout}{stderr}");
        for index in [rec.records.len() / 2, rec.records.len() - 1] {
            let mut broken = rec.clone();
            broken.records.remove(index);
            broken.write(&file).unwrap();
            let (code, stdout, stderr) =
                tool(&["--source", "file", "--file", file.to_str().unwrap()]);
            assert_eq!(code, 2, "{stdout}{stderr}");
            assert!(
                stderr.contains("expected the record") || stderr.contains("log ends before tick"),
                "{stderr}"
            );
        }
        std::fs::remove_file(file).unwrap();
    }
}

#[tokio::test]
async fn fight_copy_without_completion_watermark_is_refused() {
    use prost::Message as _;
    use std::io::{Read as _, Write as _};
    #[derive(prost::Message)]
    struct Envelope {
        #[prost(bytes = "vec", tag = "1")]
        snapshot: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        watermark: Vec<u8>,
        #[prost(bytes = "vec", repeated, tag = "3")]
        records: Vec<Vec<u8>>,
    }
    use nightfall_api::application::replay_log::ReplayError;
    let rec = fight();
    let log = InMemoryEventLog::default();
    log.write_snapshot(&rec.snapshot).await.unwrap();
    for r in &rec.records {
        log.append_applied(r).await.unwrap();
    }
    assert!(matches!(
        open_epoch(&log, rec.snapshot.seed.zone, rec.snapshot.seed.epoch).await,
        Err(ReplayError::Incomplete { .. })
    ));
    assert!(matches!(
        Recording::export(&log, rec.snapshot.seed.zone, rec.snapshot.seed.epoch).await,
        Err(ReplayError::Incomplete { .. })
    ));

    // Strip field 2 from the actual .nfr protobuf envelope, retaining snapshot and records.
    let bytes = rec.to_bytes().unwrap();
    let mut body = Vec::new();
    flate2::read::ZlibDecoder::new(&bytes[12..])
        .read_to_end(&mut body)
        .unwrap();
    let mut envelope = Envelope::decode(body.as_slice()).unwrap();
    envelope.watermark.clear();
    let mut z = flate2::write::ZlibEncoder::new(bytes[..12].to_vec(), flate2::Compression::fast());
    z.write_all(&envelope.encode_to_vec()).unwrap();
    let file = tmp("fight-incomplete.nfr");
    std::fs::write(&file, z.finish().unwrap()).unwrap();
    let (code, stdout, stderr) = tool(&["--source", "file", "--file", file.to_str().unwrap()]);
    assert_eq!(code, 2, "malformed file must be refused: {stdout}{stderr}");
    assert!(stderr.contains("cannot decode replay-log record"), "{stderr}");
    std::fs::remove_file(file).unwrap();
}
