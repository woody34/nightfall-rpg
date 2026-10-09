//! `GET /ws` through real sockets (Stories 4.1-4.3, api-guidelines.md §4 and "Real-time
//! channel"): handshake statuses, close codes, limits, zone join and AOI, MoveTo/StopMove,
//! replacement, ordered output, and the session audit.
//!
//! The zone runs on real 100 ms ticks; every wait is bounded by `common::ws::WAIT`.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::float_cmp, // wire positions are exactly representable tile values
    clippy::similar_names
)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use common::ws::{
    ack, connect, connect_with, despawn_of, entities_in, join, move_of, move_to, rejected,
    seed_player, spawn_of, stop_move, ticket, Received, WAIT,
};
use common::TestApp;
use futures_util::SinkExt as _;
use nightfall_api::domain::zone::{EntityId, ZoneCommand};
use nightfall_api::infrastructure::memory::{AuditedFrame, ManualClock};
use nightfall_api::interface::grpc::pb::{ClientMessage, RejectReason};
use nightfall_api::interface::zone_mapping::encode_observer_outputs;
use prost::Message as _;
use tokio_tungstenite::tungstenite::Message;

const TIME_ORIGIN_MS: i64 = 1_000_000; // TestApp's fixture zone

fn reason(r: RejectReason) -> i32 {
    r.into()
}

// ---- Handshake: HTTP statuses before the upgrade --------------------------------------------

#[tokio::test]
async fn missing_malformed_or_unknown_ticket_is_401() {
    let app = TestApp::spawn().await;
    assert_eq!(connect_with(&app, None).await.err(), Some(401));
    assert_eq!(connect_with(&app, Some("Basic abc")).await.err(), Some(401));
    assert_eq!(connect(&app, "not-a-ticket").await.err(), Some(401));
    let unknown = nightfall_api::domain::PlayTicket::from_bytes([7; 32]).encode();
    assert_eq!(connect(&app, &unknown).await.err(), Some(401));
}

#[tokio::test]
async fn consumed_ticket_is_401() {
    let app = TestApp::spawn().await;
    let p = seed_player(&app, "Aria", 10.0, 10.0);
    let t = ticket(&app, &p).await;
    let first = connect(&app, &t).await.expect("first use admits");
    assert_eq!(connect(&app, &t).await.err(), Some(401));
    drop(first);
}

#[tokio::test]
async fn expired_ticket_is_401() {
    let clock = Arc::new(ManualClock::default());
    let c = clock.clone();
    let app = TestApp::spawn_with(move |deps| deps.clock = c).await;
    let p = seed_player(&app, "Aria", 10.0, 10.0);
    let t = ticket(&app, &p).await;
    clock.advance(chrono::Duration::seconds(61));
    assert_eq!(connect(&app, &t).await.err(), Some(401));
}

#[tokio::test]
async fn superseded_ticket_is_409() {
    let app = TestApp::spawn().await;
    let p = seed_player(&app, "Aria", 10.0, 10.0);
    let older = ticket(&app, &p).await;
    let newer = ticket(&app, &p).await;
    assert_eq!(connect(&app, &older).await.err(), Some(409));
    connect(&app, &newer)
        .await
        .expect("the newest ticket admits");
}

#[tokio::test]
async fn eleventh_concurrent_session_from_one_ip_is_429_and_a_slot_frees_on_close() {
    let app = TestApp::spawn().await;
    let mut open = Vec::new();
    for n in 0..10_u8 {
        let letter = char::from(b'a' + n);
        let p = seed_player(&app, &format!("Player{letter}"), 10.0, 10.0);
        open.push(join(&app, &p).await);
    }
    let eleventh = seed_player(&app, "Eleven", 10.0, 10.0);
    let t = ticket(&app, &eleventh).await;
    assert_eq!(connect(&app, &t).await.err(), Some(429));
    // The refused handshake did not spend the ticket; a freed slot admits it.
    open.pop().unwrap().close().await;
    let mut admitted = None;
    for _ in 0..50 {
        match connect(&app, &t).await {
            Ok(ws) => {
                admitted = Some(ws);
                break;
            },
            Err(429) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(s) => panic!("unexpected status {s}"),
        }
    }
    assert!(admitted.is_some(), "slot was never released");
}

// ---- Close codes and limits after the upgrade -----------------------------------------------

#[tokio::test]
async fn seq_regression_closes_with_4400() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    ws.send(&stop_move(5)).await;
    ws.send(&stop_move(4)).await;
    assert_eq!(ws.closed(WAIT).await, Some(4400));
}

#[tokio::test]
async fn oversize_frame_is_rejected_invalid_before_decoding_and_the_session_stays() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    ws.send_raw(vec![0_u8; 4097]).await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    let r = rejected(msgs.last().unwrap()).unwrap();
    assert_eq!((r.seq, r.reason), (0, reason(RejectReason::Invalid)));
    // Still open: the next intent is answered.
    ws.send(&stop_move(1)).await;
    ws.until(|m| ack(m).is_some_and(|a| a.seq == 1)).await;
}

#[tokio::test]
async fn undecodable_and_empty_frames_are_rejected_invalid() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    ws.send_raw(vec![0xff, 0xff, 0xff]).await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    assert_eq!(rejected(msgs.last().unwrap()).unwrap().reason, reason(RejectReason::Invalid));
    ws.send(&ClientMessage {
        seq: 2,
        intent: None,
    })
    .await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    let r = rejected(msgs.last().unwrap()).unwrap();
    assert_eq!((r.seq, r.reason), (2, reason(RejectReason::Invalid)));
}

#[tokio::test]
async fn the_31st_frame_in_a_second_is_rate_limited() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    // Queue all 31 frames and flush them at once, well inside one 33 ms refill.
    for seq in 1..=31 {
        let bytes = stop_move(seq).encode_to_vec();
        ws.socket.feed(Message::Binary(bytes.into())).await.unwrap();
    }
    ws.socket.flush().await.unwrap();
    let mut acked = Vec::new();
    let mut limited = Vec::new();
    while acked.len() + limited.len() < 31 {
        let m = ws.next_msg().await;
        if let Some(a) = ack(&m) {
            acked.push(a.seq);
        } else if let Some(r) = rejected(&m) {
            assert_eq!(r.reason, reason(RejectReason::RateLimited), "{r:?}");
            limited.push(r.seq);
        }
    }
    assert_eq!(limited, vec![31]);
    acked.sort_unstable();
    assert_eq!(acked, (1..=30).collect::<Vec<_>>());
}

#[tokio::test]
async fn an_idle_session_closes_with_4408() {
    let app = TestApp::spawn_with(|deps| {
        deps.session_limits.idle_timeout = Duration::from_millis(400);
    })
    .await;
    let mut ws = join(&app, &seed_player(&app, "Aria", 10.0, 10.0)).await;
    // Activity keeps it open...
    for seq in 1..=3 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        ws.send(&stop_move(seq)).await;
    }
    // ...silence does not.
    let started = tokio::time::Instant::now();
    assert_eq!(ws.closed(Duration::from_secs(3)).await, Some(4408));
    assert!(started.elapsed() >= Duration::from_millis(250));
}

// ---- Zone join and AOI (Story 4.2) ----------------------------------------------------------

#[tokio::test]
async fn players_in_range_see_each_other_spawn_and_move_and_one_outside_sees_neither() {
    let app = TestApp::spawn().await;
    let (a, b, c) = (
        seed_player(&app, "Alpha", 10.0, 10.0),
        seed_player(&app, "Bravo", 12.0, 12.0),
        seed_player(&app, "Charlie", 200.0, 200.0),
    );
    let mut wa = join(&app, &a).await;
    let first = wa.next_msg().await;
    let own = spawn_of(&first).expect("first frame is the player's own spawn");
    assert_eq!(own.entity_id, a.entity_id());
    assert_eq!(own.name, "Alpha");
    assert_eq!(own.session_generation, 1);
    let pos = own.position.unwrap();
    assert_eq!((pos.x, pos.y), (10.0, 10.0));

    let mut wc = join(&app, &c).await;
    let mut wb = join(&app, &b).await;
    // B is told about A (and itself); A is told about B.
    let seen_by_b = wb
        .until(|m| spawn_of(m).is_some_and(|s| s.entity_id == a.entity_id()))
        .await;
    assert!(seen_by_b.iter().all(|m| spawn_of(m).is_some()));
    wa.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == b.entity_id()))
        .await;

    // A walks; B sees the moves.
    wa.send(&move_to(1, 14.0, 10.0)).await;
    wb.until(|m| move_of(m).is_some_and(|mv| mv.entity_id == a.entity_id()))
        .await;
    wa.until(|m| move_of(m).is_some_and(|mv| mv.entity_id == a.entity_id() && mv.speed == 0.0))
        .await;

    // C, 190 tiles away, only ever heard about itself.
    let seen_by_c = wc.drain(Duration::from_millis(500)).await;
    assert_eq!(entities_in(&seen_by_c), vec![c.entity_id()], "{seen_by_c:?}");
}

#[tokio::test]
async fn a_disconnect_despawns_the_player_for_its_neighbours() {
    let app = TestApp::spawn().await;
    let (a, b) = (seed_player(&app, "Alpha", 10.0, 10.0), seed_player(&app, "Bravo", 11.0, 11.0));
    let mut wa = join(&app, &a).await;
    let wb = join(&app, &b).await;
    wa.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == b.entity_id()))
        .await;
    wb.close().await;
    wa.until(|m| despawn_of(m).is_some_and(|d| d.entity_id == b.entity_id()))
        .await;
}

// ---- MoveTo / StopMove (Story 4.3) ----------------------------------------------------------

#[tokio::test]
async fn move_to_is_acked_then_moves_at_10_hz_with_tick_derived_time_and_arrives_once() {
    let app = TestApp::spawn().await;
    let a = seed_player(&app, "Alpha", 10.0, 10.0);
    let mut ws = join(&app, &a).await;
    ws.next_msg().await; // own spawn
    ws.send(&move_to(7, 11.0, 10.0)).await; // 1 tile at 5 tiles/s: two ticks
    let msgs = ws
        .until(|m| move_of(m).is_some_and(|mv| mv.speed == 0.0))
        .await;
    let a7 = msgs.iter().find_map(|m| ack(m)).expect("acked");
    assert_eq!(a7.seq, 7);
    let moves: Vec<_> = msgs.iter().filter_map(|m| move_of(m)).collect();
    assert_eq!(moves.len(), 2, "one step, then the arrival: {moves:?}");
    assert_eq!(moves[0].tick, a7.tick, "the effect is stamped with the ack's tick");
    assert_eq!(moves[1].tick, a7.tick + 1);
    for mv in &moves {
        assert_eq!(mv.server_time_ms, TIME_ORIGIN_MS + i64::try_from(mv.tick).unwrap() * 100);
    }
    assert_eq!(moves[0].speed, 5.0);
    assert_eq!(moves[0].destination.unwrap().x, 11.0);
    let end = moves[1].position.unwrap();
    assert_eq!((end.x, end.y), (11.0, 10.0));
    assert_eq!(moves[1].destination.unwrap().x, 0.0, "arrived: destination zero");
    // The ack precedes the tick's events.
    let ack_at = msgs.iter().position(|m| ack(m).is_some()).unwrap();
    let move_at = msgs.iter().position(|m| move_of(m).is_some()).unwrap();
    assert!(ack_at < move_at);
}

#[tokio::test]
async fn move_to_outside_the_zone_is_out_of_bounds() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 250.0, 10.0)).await;
    ws.send(&move_to(1, 260.0, 10.0)).await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    let r = rejected(msgs.last().unwrap()).unwrap();
    assert_eq!((r.seq, r.reason), (1, reason(RejectReason::OutOfBounds)));
    // Not finite: refused at the edge with the same reason.
    ws.send(&move_to(2, f32::NAN, 10.0)).await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    let r = rejected(msgs.last().unwrap()).unwrap();
    assert_eq!((r.seq, r.reason), (2, reason(RejectReason::OutOfBounds)));
}

#[tokio::test]
async fn move_to_beyond_64_tiles_is_too_far() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    ws.send(&move_to(1, 100.0, 100.0)).await;
    let msgs = ws.until(|m| rejected(m).is_some()).await;
    let r = rejected(msgs.last().unwrap()).unwrap();
    assert_eq!((r.seq, r.reason), (1, reason(RejectReason::TooFar)));
}

#[tokio::test]
async fn stop_move_halts_a_moving_player_and_is_a_silent_ack_when_still() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    ws.next_msg().await; // own spawn
    ws.next_msg().await; // own StatsChanged (owner-only, E2.2)
    ws.send(&stop_move(1)).await;
    let m = ws.next_msg().await;
    assert_eq!(ack(&m).map(|a| a.seq), Some(1));
    assert!(ws.drain(Duration::from_millis(300)).await.is_empty(), "no event when still");

    ws.send(&move_to(2, 40.0, 10.0)).await;
    ws.until(|m| move_of(m).is_some()).await;
    ws.send(&stop_move(3)).await;
    let msgs = ws
        .until(|m| move_of(m).is_some_and(|mv| mv.speed == 0.0))
        .await;
    assert!(msgs.iter().any(|m| ack(m).is_some_and(|a| a.seq == 3)));
    let last = move_of(msgs.last().unwrap()).unwrap();
    let d = last.destination.unwrap();
    assert_eq!((d.x, d.y), (0.0, 0.0));
    assert!(last.position.unwrap().x < 40.0, "stopped short of the destination");
}

// ---- Replacement (plan §8 #8) ---------------------------------------------------------------

#[tokio::test]
async fn a_new_login_closes_the_old_socket_and_its_late_disconnect_cannot_despawn_the_new() {
    let app = TestApp::spawn().await;
    let p = seed_player(&app, "Alpha", 10.0, 10.0);
    let watcher = seed_player(&app, "Watcher", 12.0, 12.0);
    let mut ww = join(&app, &watcher).await;
    let mut old = join(&app, &p).await;
    old.next_msg().await;
    ww.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == p.entity_id()))
        .await;

    let mut new = join(&app, &p).await;
    assert_eq!(old.closed(WAIT).await, Some(4409), "the old socket is closed");
    // The new session sees the world again: itself and the watcher, generation 2.
    let seen = new
        .until(|m| spawn_of(m).is_some_and(|s| s.entity_id == watcher.entity_id()))
        .await;
    let me = seen
        .iter()
        .filter_map(|m| spawn_of(m))
        .find(|s| s.entity_id == p.entity_id())
        .expect("own spawn resent");
    assert_eq!(me.session_generation, 2);

    // The old socket's TCP close happened; give its Despawn time to be applied (and refused).
    drop(old);
    tokio::time::sleep(Duration::from_millis(400)).await;
    let snap = app.zone.snapshot().await.unwrap();
    let e = snap
        .entities
        .iter()
        .find(|e| e.id == EntityId::from_uuid(p.character.as_uuid()))
        .expect("replacement still in the zone");
    assert_eq!(e.generation.0, 2);
    let watched = ww.drain(Duration::from_millis(300)).await;
    assert!(
        !watched.iter().any(|m| despawn_of(m).is_some()),
        "the watcher never saw a despawn: {watched:?}"
    );
    // And the new session still plays.
    new.send(&move_to(1, 11.0, 10.0)).await;
    new.until(|m| ack(m).is_some_and(|a| a.seq == 1)).await;
}

// ---- Ordered output (plan §8 #6) ------------------------------------------------------------

#[tokio::test]
async fn session_output_is_exactly_the_actors_per_player_output_byte_for_byte() {
    let app = TestApp::spawn().await;
    let mut ticks = app.zone.subscribe();
    let (a, b) = (seed_player(&app, "Alpha", 10.0, 10.0), seed_player(&app, "Bravo", 13.0, 10.0));
    let mut wa = join(&app, &a).await;
    let mut wb = join(&app, &b).await;
    wa.send(&move_to(1, 12.0, 12.0)).await;
    wb.send(&move_to(1, 10.0, 14.0)).await;
    wa.send(&move_to(2, 300.0, 0.0)).await; // refused by the zone: in the output too
    wa.send(&stop_move(3)).await;
    // Let everything settle: both arrive or stop well within a second.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let mut got = Vec::new();
    while let Some(Received::Message(_, bytes)) = wa.recv(Duration::from_millis(200)).await {
        got.push(bytes);
    }

    let me = EntityId::from_uuid(a.character.as_uuid());
    let mut expected: Vec<Bytes> = Vec::new();
    let mut admitted = false;
    while let Ok(t) = ticks.try_recv() {
        admitted |= t
            .commands
            .iter()
            .any(|c| matches!(c.command, ZoneCommand::SpawnPlayer { entity, .. } if entity == me));
        if admitted {
            if let Some(out) = t.outputs.get(&me) {
                expected.extend(encode_observer_outputs(out, t.server_time_ms));
            }
        }
    }
    assert!(expected.len() >= 6, "spawns, acks, moves, a rejection: {}", expected.len());
    let expected: Vec<Vec<u8>> = expected.into_iter().map(|b| b.to_vec()).collect();
    assert_eq!(got, expected);
    drop(wb);
}

// ---- Audit -----------------------------------------------------------------------------------

#[tokio::test]
async fn the_audit_log_holds_exactly_what_the_client_sent_and_received() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    let sent: Vec<Vec<u8>> = vec![
        move_to(1, 12.0, 10.0).encode_to_vec(),
        vec![0xff, 0xff],
        stop_move(2).encode_to_vec(),
        move_to(3, 10.0, 200.0).encode_to_vec(),
    ];
    for f in &sent {
        ws.send_raw(f.clone()).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut received = Vec::new();
    while let Some(Received::Message(_, bytes)) = ws.recv(Duration::from_millis(200)).await {
        received.push(bytes);
    }
    ws.close().await;

    let sessions = app.audit.sessions();
    assert_eq!(sessions.len(), 1);
    let frames = app.audit.frames(sessions[0]);
    let ins: Vec<(Option<u32>, Vec<u8>)> = frames
        .iter()
        .filter_map(|f| match f {
            AuditedFrame::In { seq, frame } => Some((*seq, frame.to_vec())),
            AuditedFrame::Out { .. } => None,
        })
        .collect();
    let outs: Vec<Vec<u8>> = frames
        .iter()
        .filter_map(|f| match f {
            AuditedFrame::Out { frame } => Some(frame.to_vec()),
            AuditedFrame::In { .. } => None,
        })
        .collect();
    let expected_in: Vec<(Option<u32>, Vec<u8>)> = vec![
        (Some(1), sent[0].clone()),
        (None, sent[1].clone()),
        (Some(2), sent[2].clone()),
        (Some(3), sent[3].clone()),
    ];
    assert_eq!(ins, expected_in);
    assert_eq!(outs, received);
    assert!(received.len() >= 5);
}

// ---- Metrics ---------------------------------------------------------------------------------

/// The value of the series `name` whose labels contain `label` (exporters add their own).
fn metric(text: &str, name: &str, label: &str) -> Option<f64> {
    text.lines()
        .filter(|l| l.starts_with(name) && l.contains(label))
        .find_map(|l| l.rsplit(' ').next()?.parse().ok())
}

async fn eventually(app: &TestApp, name: &str, label: &str, want: f64) {
    for _ in 0..100 {
        let text = app.metrics.render().unwrap();
        if metric(&text, name, label) == Some(want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{name}{{{label}}} never reached {want}:\n{}", app.metrics.render().unwrap());
}

#[tokio::test]
async fn sessions_and_frames_are_counted() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    ws.send(&stop_move(1)).await;
    ws.until(|m| ack(m).is_some()).await;
    eventually(&app, "nightfall_sessions_active", "", 1.0).await;
    eventually(&app, "nightfall_ws_frames_total", "direction=\"in\"", 1.0).await;
    // Own spawn, own StatsChanged, then the ack.
    eventually(&app, "nightfall_ws_frames_total", "direction=\"out\"", 3.0).await;
    eventually(&app, "nightfall_ws_dropped_frames_total", "", 0.0).await;
    ws.close().await;
    eventually(&app, "nightfall_sessions_active", "", 0.0).await;
}

#[tokio::test]
async fn server_shutdown_closes_sessions_with_1001() {
    let app = TestApp::spawn().await;
    let mut ws = join(&app, &seed_player(&app, "Alpha", 10.0, 10.0)).await;
    ws.next_msg().await;
    app.sessions_shutdown.cancel();
    assert_eq!(ws.closed(WAIT).await, Some(1001));
}
