//! `SessionAudit` trait dispatch through the bounded writer, decoded stored frames and real WS.
#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;
mod replay_support;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::Bytes;
use nightfall_api::application::replay_log::{NoReplayMetrics, SessionInRecord, SessionOutRecord};
use nightfall_api::application::{Clock, SessionAudit, SessionAuditContext};
use nightfall_api::domain::zone::{Tick, ZoneId};
use nightfall_api::domain::SessionId;
use nightfall_api::infrastructure::eventlog::{
    session_subject, EventLogSessionAudit, InMemoryEventLog,
};
use nightfall_api::infrastructure::memory::ManualClock;
use nightfall_api::interface::grpc::pb::ClientMessage;
use prost::Message as _;

// Independent minimal readers pin the helper's raw-frame tags without depending on codec.rs.
#[derive(prost::Message)]
struct InFrame {
    #[prost(bytes = "bytes", tag = "7")]
    frame: Bytes,
}
#[derive(prost::Message)]
struct OutFrame {
    #[prost(bytes = "bytes", tag = "5")]
    frame: Bytes,
}

fn position(tick: u64) -> SessionAuditContext {
    SessionAuditContext {
        zone: ZoneId(17),
        epoch: 23,
        tick: Tick(tick),
    }
}

#[tokio::test]
async fn trait_dispatch_preserves_raw_bytes_metadata_and_receive_time_through_shutdown() {
    let log = Arc::new(InMemoryEventLog::default());
    let clock = Arc::new(ManualClock::default());
    let (adapter, drain) =
        EventLogSessionAudit::spawn(log.clone(), clock.clone(), Arc::new(NoReplayMetrics), 16);
    let audit: Arc<dyn SessionAudit> = Arc::new(adapter);
    let session = SessionId::new();
    let received = clock.now().timestamp_millis();
    let malformed = Bytes::from_static(b"\xff\x00malformed");
    let inbound = Bytes::from(common::ws::stop_move(7).encode_to_vec());
    let outbound = Bytes::from_static(b"\x0a\x02\x08\x07");
    audit.record_in_context(session, None, &malformed, position(41));
    audit.record_in_context(session, Some(7), &inbound, position(42));
    audit.record_out_context(session, &outbound, position(42));
    // Timestamps must be captured on the socket path, never by the delayed drain.
    clock.advance(chrono::Duration::hours(1));
    drain.shutdown().await;
    let ins = log.session_in();
    assert_eq!(
        ins,
        vec![
            SessionInRecord {
                session: session.as_uuid(),
                seq: 0,
                zone: ZoneId(17),
                epoch: 23,
                tick_seen: Tick(41),
                recv_unix_ms: received,
                frame: malformed.clone()
            },
            SessionInRecord {
                session: session.as_uuid(),
                seq: 7,
                zone: ZoneId(17),
                epoch: 23,
                tick_seen: Tick(42),
                recv_unix_ms: received,
                frame: inbound.clone()
            },
        ]
    );
    let outs = log.session_out();
    assert_eq!(
        outs,
        vec![SessionOutRecord {
            session: session.as_uuid(),
            zone: ZoneId(17),
            epoch: 23,
            tick: Tick(42),
            frame: outbound.clone()
        }]
    );
    assert_eq!(InFrame::decode(ins[0].encode().as_slice()).unwrap().frame, malformed);
    assert_eq!(InFrame::decode(ins[1].encode().as_slice()).unwrap().frame, inbound);
    assert_eq!(OutFrame::decode(outs[0].encode().as_slice()).unwrap().frame, outbound);
    assert_eq!(
        log.subjects()
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>(),
        vec![
            session_subject(session.as_uuid(), "in"),
            session_subject(session.as_uuid(), "in"),
            session_subject(session.as_uuid(), "out"),
        ]
    );
}

#[tokio::test]
async fn full_channel_drops_without_waiting_and_shutdown_flushes_accepted_frames() {
    let log = Arc::new(InMemoryEventLog::default());
    let (adapter, drain) = EventLogSessionAudit::spawn(
        log.clone(),
        Arc::new(ManualClock::default()),
        Arc::new(NoReplayMetrics),
        2,
    );
    let audit: &dyn SessionAudit = &adapter;
    let session = SessionId::new();
    let frame = Bytes::from_static(b"raw");
    // A current-thread runtime cannot run the drain until this synchronous loop yields.
    // All 1000 calls return even though only two frames fit in the channel.
    for seq in 1..=1000 {
        audit.record_in(session, Some(seq), &frame);
    }
    assert_eq!(adapter.dropped(), 998);
    drain.shutdown().await;
    assert_eq!(log.session_in().iter().map(|r| r.seq).collect::<Vec<_>>(), vec![1, 2]);
    audit.record_out(session, &frame);
    assert_eq!(adapter.dropped(), 999, "recording after shutdown also drops without waiting");
}

#[tokio::test]
async fn failed_acknowledgements_count_each_buffered_frame() {
    let log = Arc::new(InMemoryEventLog::default());
    let faulty = Arc::new(replay_support::FaultyLog::new(log.clone()));
    faulty.down.store(true, Ordering::SeqCst);
    let (adapter, drain) = EventLogSessionAudit::spawn(
        faulty,
        Arc::new(ManualClock::default()),
        Arc::new(NoReplayMetrics),
        4,
    );
    let audit: &dyn SessionAudit = &adapter;
    let session = SessionId::new();
    audit.record_in(session, None, &Bytes::from_static(b"in"));
    audit.record_out(session, &Bytes::from_static(b"out"));
    drain.shutdown().await;
    assert_eq!(adapter.dropped(), 2);
    assert!(log.session_in().is_empty());
    assert!(log.session_out().is_empty());
}

#[tokio::test(start_paused = true)]
async fn shutdown_of_a_writer_with_a_stalled_ack_is_bounded_and_closes_the_channel() {
    let log = Arc::new(InMemoryEventLog::default());
    let faulty = Arc::new(replay_support::FaultyLog::new(log.clone()));
    faulty.hang_audit.store(true, Ordering::SeqCst);
    let (adapter, drain) = EventLogSessionAudit::spawn(
        faulty,
        Arc::new(ManualClock::default()),
        Arc::new(NoReplayMetrics),
        4,
    );
    let audit: &dyn SessionAudit = &adapter;
    let session = SessionId::new();
    audit.record_in(session, Some(1), &Bytes::from_static(b"stalled"));
    tokio::task::yield_now().await;
    let started = tokio::time::Instant::now();
    drain.shutdown().await;
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(5));
    tokio::task::yield_now().await;
    audit.record_out(session, &Bytes::from_static(b"after shutdown"));
    assert_eq!(adapter.dropped(), 1, "the aborted worker closes the socket-path channel");
    assert!(log.session_in().is_empty());
    assert!(log.session_out().is_empty());
}

#[tokio::test]
async fn real_socket_records_invalid_frames_and_exact_ack_tick_but_never_upgrade_auth() {
    use common::ws::{ack, connect, rejected, seed_player, spawn_of, ticket, WAIT};

    let log = Arc::new(InMemoryEventLog::default());
    let (adapter, drain) = EventLogSessionAudit::spawn(
        log.clone(),
        Arc::new(ManualClock::default()),
        Arc::new(NoReplayMetrics),
        1024,
    );
    let app = common::TestApp::spawn_with(move |deps| deps.audit = Arc::new(adapter)).await;
    let player = seed_player(&app, "AuditPlayer", 10.0, 10.0);
    let secret = ticket(&app, &player).await;
    assert!(log.session_in().is_empty(), "gRPC ticket issuance must not record auth");
    assert!(connect(&app, "invalid-secret-ticket").await.is_err());
    assert!(log.session_in().is_empty(), "refused upgrades must not record auth");
    let mut ws = connect(&app, &secret).await.unwrap();
    ws.until(|m| spawn_of(m).is_some_and(|s| s.entity_id == player.entity_id()))
        .await;
    let frame = common::ws::stop_move(1);
    ws.send(&frame).await;
    let messages = ws.until(|m| ack(m).is_some_and(|a| a.seq == 1)).await;
    let applied = ack(messages.last().unwrap()).unwrap().tick;
    let malformed = vec![0xff];
    ws.send_raw(malformed.clone()).await;
    ws.until(|m| rejected(m).is_some()).await;
    app.sessions_shutdown.cancel();
    assert_eq!(ws.closed(WAIT).await, Some(1001));
    drain.shutdown().await;

    let ins = log.session_in();
    assert_eq!(ins.len(), 2);
    assert_eq!(ins[0].frame.as_ref(), frame.encode_to_vec());
    assert_eq!(ClientMessage::decode(ins[0].frame.clone()).unwrap().seq, 1);
    assert_eq!(ins[1].seq, 0);
    assert_eq!(ins[1].frame.as_ref(), malformed);
    assert_ne!(ins[0].session, player.character.as_uuid(), "audit key is the socket UUID");
    let position = app.zone.audit_context();
    assert_eq!(ins[0].zone, position.zone);
    assert_eq!(ins[0].epoch, position.epoch);
    assert!(ins[0].tick_seen.0 > 0);
    let outs = log.session_out();
    let ack_record = outs
        .iter()
        .find(|r| {
            let msg =
                nightfall_api::interface::grpc::pb::ServerMessage::decode(r.frame.clone()).unwrap();
            ack(&msg).is_some_and(|a| a.seq == 1)
        })
        .unwrap();
    assert_eq!(ack_record.tick, Tick(applied));
    assert!(outs.iter().all(|r| r.session == ins[0].session
        && r.zone == position.zone
        && r.epoch == position.epoch));
    for bytes in ins
        .iter()
        .map(|r| &r.frame)
        .chain(outs.iter().map(|r| &r.frame))
    {
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!bytes.windows(b"Bearer".len()).any(|w| w == b"Bearer"));
    }
}

#[tokio::test]
async fn audit_position_tracks_restored_idle_and_held_ticks_without_waiting_for_stats() {
    use nightfall_api::application::zone_actor::{
        manual_ticks, GateError, TickGate, TickOutcome, ZoneActor,
    };
    use nightfall_api::domain::zone::{AppliedTick, ZoneState};
    use std::sync::atomic::AtomicBool;

    struct Hold(Arc<AtomicBool>);
    impl TickGate for Hold {
        fn admit(
            &self,
            _tick: &AppliedTick,
        ) -> impl std::future::Future<Output = Result<(), GateError>> + Send {
            std::future::ready(if self.0.load(Ordering::SeqCst) {
                Err(GateError("held for audit test".into()))
            } else {
                Ok(())
            })
        }
    }
    let mut snapshot = common::fixture_zone().snapshot();
    snapshot.tick = Tick(100);
    let state = ZoneState::from_snapshot(snapshot).unwrap();
    let seed = state.seed();
    let next_tick = state.next_tick();
    let held = Arc::new(AtomicBool::new(true));
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated(state, ticks, Hold(held.clone()));
    let expected = SessionAuditContext {
        zone: seed.zone,
        epoch: seed.epoch,
        tick: next_tick,
    };
    assert_eq!(handle.audit_context(), expected);
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(next_tick));
    let advanced = SessionAuditContext {
        tick: Tick(next_tick.0.saturating_add(1)),
        ..expected
    };
    assert_eq!(handle.audit_context(), advanced);
    // Reoffering a held tick does not advance the state or its audit position twice.
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(next_tick));
    assert_eq!(handle.audit_context(), advanced);
    held.store(false, Ordering::SeqCst);
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(next_tick));
    assert_eq!(handle.audit_context(), advanced);
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Ran(advanced.tick));
    assert_eq!(handle.audit_context().tick, Tick(advanced.tick.0.saturating_add(1)));
    drop(driver);
    handle.stopped().await;
}
