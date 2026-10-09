//! Session actor unit tests: in-memory socket halves, a manually stepped zone, a stub codec.
//! Cases a real socket cannot reproduce deterministically live here (a client that stops
//! reading, a full zone queue); the wire-level matrix is in `tests/ws_session.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use uuid::Uuid;

use super::*;
use crate::application::zone_actor::{manual_ticks, ManualTickDriver, ZoneActor};
use crate::application::zone_registry::fixture_state;
use crate::infrastructure::memory::{AuditedFrame, InMemorySessionAudit};

/// Frames are `[seq bytes (big-endian, 1-4)..., kind]`: kind 1 = `StopMove`, 2 = `MoveTo` far
/// away, anything else undecodable. Output frames are `out:<item>`, rejections
/// `rej:<seq>:<reason>`.
struct StubCodec;

impl SessionCodec for StubCodec {
    fn decode(&self, entity: EntityId, frame: &[u8]) -> Result<DecodedIntent, Undecodable> {
        let (kind, seq_bytes) = frame.split_last().ok_or(Undecodable)?;
        if seq_bytes.is_empty() || seq_bytes.len() > 4 {
            return Err(Undecodable);
        }
        let seq = seq_bytes
            .iter()
            .fold(0_u32, |acc, b| (acc << 8) | u32::from(*b));
        let command = match kind {
            1 => ZoneCommand::StopMove { entity },
            2 => ZoneCommand::MoveTo {
                entity,
                dest: Vec2Fixed::from_tiles(250, 250),
            },
            _ => return Err(Undecodable),
        };
        Ok(DecodedIntent {
            seq,
            command: Ok(command),
        })
    }

    fn rejected(&self, seq: u32, reason: SessionReject, _detail: &str) -> Bytes {
        Bytes::from(format!("rej:{seq}:{reason:?}"))
    }

    fn outputs(&self, outputs: &[ObserverOutput], _server_time_ms: i64) -> Vec<Bytes> {
        outputs
            .iter()
            .map(|o| Bytes::from(format!("out:{o:?}")))
            .collect()
    }
}

#[derive(Default)]
struct CountingMetrics {
    active: AtomicI64,
    dropped: AtomicU64,
    frames_in: AtomicU64,
}

impl SessionMetrics for CountingMetrics {
    fn session_opened(&self) {
        self.active.fetch_add(1, Ordering::SeqCst);
    }
    fn session_closed(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    fn frames_in(&self, n: u64) {
        self.frames_in.fetch_add(n, Ordering::SeqCst);
    }
    fn frames_out(&self, _n: u64) {}
    fn frames_dropped(&self, n: u64) {
        self.dropped.fetch_add(n, Ordering::SeqCst);
    }
}

struct ChannelSource(mpsc::Receiver<InboundFrame>);

impl FrameSource for ChannelSource {
    async fn recv(&mut self) -> Option<InboundFrame> {
        self.0.recv().await
    }
}

/// A sink that forwards frames, or (when `stalled`) never completes a write, like a client
/// that stopped reading.
struct ChannelSink {
    tx: mpsc::UnboundedSender<OutboundFrame>,
    stalled: bool,
}

impl FrameSink for ChannelSink {
    async fn send(&mut self, frames: Vec<OutboundFrame>) -> Result<(), SinkClosed> {
        if self.stalled {
            std::future::pending::<()>().await;
        }
        for f in frames {
            self.tx.send(f).map_err(|_| SinkClosed)?;
        }
        Ok(())
    }
}

struct Harness {
    ctx: Arc<SessionContext>,
    driver: ManualTickDriver,
    audit: Arc<InMemorySessionAudit>,
    metrics: Arc<CountingMetrics>,
}

fn harness(limits: SessionLimits) -> Harness {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(fixture_state(0).unwrap(), ticks);
    let audit = Arc::new(InMemorySessionAudit::default());
    let metrics = Arc::new(CountingMetrics::default());
    let ctx = Arc::new(SessionContext {
        zone,
        registry: SessionRegistry::default(),
        audit: audit.clone(),
        metrics: metrics.clone(),
        codec: Arc::new(StubCodec),
        limits,
        shutdown: CancellationToken::new(),
    });
    Harness {
        ctx,
        driver,
        audit,
        metrics,
    }
}

struct Client {
    id: SessionId,
    tx: mpsc::Sender<InboundFrame>,
    rx: mpsc::UnboundedReceiver<OutboundFrame>,
    task: tokio::task::JoinHandle<SessionEnd>,
}

fn player(n: u128, x: i32) -> PlayerSpawn {
    PlayerSpawn {
        entity: EntityId::from_uuid(Uuid::from_u128(n)),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(x, 10),
        speed: Speed::DEFAULT,
        load: None,
    }
}

fn open(h: &Harness, spawn: PlayerSpawn, generation: u64, stalled: bool) -> Client {
    let (in_tx, in_rx) = mpsc::channel(4096);
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let start = SessionStart {
        id: SessionId::new(),
        generation: SessionGeneration(generation),
        player: spawn,
    };
    let id = start.id;
    let task = tokio::spawn(run_session(
        h.ctx.clone(),
        start,
        ChannelSource(in_rx),
        ChannelSink {
            tx: out_tx,
            stalled,
        },
    ));
    Client {
        id,
        tx: in_tx,
        rx: out_rx,
        task,
    }
}

impl Client {
    async fn send(&self, frame: &[u8]) {
        self.tx
            .send(InboundFrame::Data {
                bytes: Bytes::copy_from_slice(frame),
                binary: true,
            })
            .await
            .unwrap();
    }

    fn frames(&mut self) -> Vec<OutboundFrame> {
        let mut v = Vec::new();
        while let Ok(f) = self.rx.try_recv() {
            v.push(f);
        }
        v
    }
}

/// Lets spawned tasks run until they block.
async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// Steps the zone until the registry has queued every admission and the sessions forwarded
/// their tick.
async fn step(h: &Harness) {
    settle().await;
    h.driver.step().await.unwrap();
    settle().await;
}

#[tokio::test]
async fn nothing_is_forwarded_before_the_admission_tick_and_its_output_after() {
    let h = harness(SessionLimits::default());
    let mut a = open(&h, player(1, 10), 1, false);
    settle().await;
    assert!(a.frames().is_empty(), "nothing before the zone applies the spawn");
    step(&h).await;
    let frames = a.frames();
    assert_eq!(frames.len(), 1, "own spawn: {frames:?}");
    assert_eq!(h.metrics.active.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_client_that_stops_reading_is_closed_with_4429_and_the_drop_is_counted() {
    let limits = SessionLimits {
        outbound_queue: 2,
        frames_per_second: 1000,
        ..SessionLimits::default()
    };
    let h = harness(limits);
    let a = open(&h, player(1, 10), 1, true);
    step(&h).await; // spawn: 1 frame, stuck in the sink
                    // The writer holds the spawn frame forever. Each StopMove is acked: one more frame per
                    // tick, so the queue (2) overflows on the third ack.
    for seq in 1..=3_u8 {
        a.send(&[seq, 1]).await;
        step(&h).await;
    }
    let end = tokio::time::timeout(Duration::from_secs(5), a.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::SlowConsumer);
    assert_eq!(end.close_code(), Some(4429));
    assert_eq!(h.metrics.dropped.load(Ordering::SeqCst), 1);
    assert_eq!(h.metrics.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_tick_burst_larger_than_the_queue_is_delivered_to_a_reading_client() {
    let limits = SessionLimits {
        outbound_queue: 2,
        ..SessionLimits::default()
    };
    let h = harness(limits);
    // Six players join on one tick: each one's first output is six spawns, three times the
    // queue.
    let mut clients: Vec<Client> = (1..=6).map(|n| open(&h, player(n, 10), 1, false)).collect();
    step(&h).await;
    settle().await;
    for c in &mut clients {
        assert_eq!(c.frames().len(), 6);
        assert!(!c.task.is_finished());
    }
    assert_eq!(h.metrics.dropped.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_full_zone_queue_is_answered_with_overloaded_and_the_session_stays() {
    let limits = SessionLimits {
        frames_per_second: 100_000,
        outbound_queue: 4096,
        ..SessionLimits::default()
    };
    let h = harness(limits);
    let mut a = open(&h, player(1, 10), 1, false);
    step(&h).await;
    a.frames();
    // Without a tick nothing drains the zone queue (1024 inputs).
    for seq in 1..=1100_u32 {
        a.tx.send(InboundFrame::Data {
            bytes: Bytes::from(encode_seq(seq)),
            binary: true,
        })
        .await
        .unwrap();
    }
    settle().await;
    let rejected: Vec<_> = a
        .frames()
        .into_iter()
        .filter_map(|f| match f {
            OutboundFrame::Binary(b) => Some(String::from_utf8(b.to_vec()).unwrap()),
            OutboundFrame::Close { .. } => None,
        })
        .collect();
    assert!(!rejected.is_empty(), "some intents must be refused");
    assert!(rejected.iter().all(|r| r.ends_with(":Overloaded")), "{rejected:?}");
    assert!(!a.task.is_finished(), "OVERLOADED does not disconnect");
}

/// A `StopMove` frame with a 4-byte seq.
fn encode_seq(seq: u32) -> Vec<u8> {
    let mut v = seq.to_be_bytes().to_vec();
    v.push(1);
    v
}

#[tokio::test]
async fn seq_regression_ends_the_session_with_4400() {
    let h = harness(SessionLimits::default());
    let mut a = open(&h, player(1, 10), 1, false);
    step(&h).await;
    a.send(&[5, 1]).await;
    a.send(&[5, 1]).await;
    let end = tokio::time::timeout(Duration::from_secs(5), &mut a.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::SeqRegression);
    assert!(a.frames().contains(&OutboundFrame::Close {
        code: 4400,
        reason: "seq must increase"
    }));
}

#[tokio::test]
async fn replacement_closes_the_old_session_and_its_despawn_is_fenced() {
    let h = harness(SessionLimits::default());
    let old = open(&h, player(1, 10), 1, false);
    step(&h).await;
    let mut new = open(&h, player(1, 10), 2, false);
    settle().await;
    // The old socket is told at once, before the zone even applies the replacement.
    let end = tokio::time::timeout(Duration::from_secs(5), old.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::Replaced);
    step(&h).await; // ReplaceSession, then the old session's stale Despawn
    step(&h).await;
    let snap = h.ctx.zone.snapshot().await.unwrap();
    let e = snap
        .entities
        .iter()
        .find(|e| e.id == EntityId::from_uuid(Uuid::from_u128(1)))
        .expect("the replacement is still in the zone");
    assert_eq!(e.generation, SessionGeneration(2));
    assert_eq!(new.frames().len(), 1, "the new session got the AOI (its own spawn)");
    // A stale admission (older generation than the live one) is refused outright.
    let stale = open(&h, player(1, 10), 1, false);
    let end = tokio::time::timeout(Duration::from_secs(5), stale.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::Replaced);
}

#[tokio::test]
async fn a_refused_admission_closes_with_1011() {
    let h = harness(SessionLimits::default());
    let outside = PlayerSpawn {
        pos: Vec2Fixed::from_tiles(-5, 0),
        ..player(1, 0)
    };
    let a = open(&h, outside, 1, false);
    step(&h).await;
    let end = tokio::time::timeout(Duration::from_secs(5), a.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::AdmissionRefused);
    assert_eq!(end.close_code(), Some(1011));
}

#[tokio::test]
async fn audit_records_inbound_and_outbound_frames_in_order() {
    let h = harness(SessionLimits::default());
    let mut a = open(&h, player(1, 10), 1, false);
    step(&h).await;
    a.send(&[1, 1]).await;
    a.send(&[9, 9, 9]).await; // kind 9: undecodable, refused by the session
    step(&h).await;
    let sent: Vec<Bytes> = a
        .frames()
        .into_iter()
        .filter_map(|f| match f {
            OutboundFrame::Binary(b) => Some(b),
            OutboundFrame::Close { .. } => None,
        })
        .collect();
    let audited = h.audit.frames(a.id);
    let ins: Vec<_> = audited
        .iter()
        .filter_map(|f| match f {
            AuditedFrame::In { seq, frame } => Some((*seq, frame.clone())),
            AuditedFrame::Out { .. } | AuditedFrame::Checkpoint(_) => None,
        })
        .collect();
    let outs: Vec<_> = audited
        .iter()
        .filter_map(|f| match f {
            AuditedFrame::Out { frame } => Some(frame.clone()),
            AuditedFrame::In { .. } | AuditedFrame::Checkpoint(_) => None,
        })
        .collect();
    assert_eq!(
        ins,
        vec![
            (Some(1), Bytes::from_static(&[1, 1])),
            (None, Bytes::from_static(&[9, 9, 9]))
        ]
    );
    assert_eq!(outs, sent);
    assert_eq!(h.metrics.frames_in.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn token_bucket_allows_a_burst_then_the_rate() {
    let mut b = TokenBucket::new(30);
    let t0 = Instant::now();
    assert!((0..30).all(|_| b.take(t0)), "a full burst of 30");
    assert!(!b.take(t0), "the 31st in the same instant is refused");
    assert!(!b.take(t0 + Duration::from_millis(32)), "under one frame refilled");
    assert!(b.take(t0 + Duration::from_millis(34)), "one frame per 33.3 ms");
    assert!(!b.take(t0 + Duration::from_millis(34)));
    let later = t0 + Duration::from_secs(10);
    assert!((0..30).all(|_| b.take(later)), "refill caps at the burst");
    assert!(!b.take(later));
}

#[test]
fn close_codes_match_the_contract() {
    let cases = [
        (SessionEnd::ClientGone, None),
        (SessionEnd::Shutdown, Some(1001)),
        (SessionEnd::SeqRegression, Some(4400)),
        (SessionEnd::IdleTimeout, Some(4408)),
        (SessionEnd::Replaced, Some(4409)),
        (SessionEnd::SlowConsumer, Some(4429)),
        (SessionEnd::AdmissionRefused, Some(1011)),
        (SessionEnd::ZoneUnavailable, Some(1011)),
    ];
    for (end, code) in cases {
        assert_eq!(end.close_code(), code, "{end:?}");
    }
}

#[tokio::test]
async fn session_actor_dispatches_raw_frames_with_metadata_to_the_event_log() {
    use crate::application::replay_log::NoReplayMetrics;
    use crate::infrastructure::eventlog::{EventLogSessionAudit, InMemoryEventLog};
    use crate::infrastructure::memory::ManualClock;

    let mut h = harness(SessionLimits::default());
    let log = Arc::new(InMemoryEventLog::default());
    let (adapter, drain) = EventLogSessionAudit::spawn(
        log.clone(),
        Arc::new(ManualClock::default()),
        Arc::new(NoReplayMetrics),
        1024,
    );
    Arc::get_mut(&mut h.ctx).unwrap().audit = Arc::new(adapter);
    let mut client = open(&h, player(1, 10), 1, false);
    step(&h).await;
    let seen = h.ctx.zone.audit_context();
    client.send(&[1, 1]).await;
    client.send(&[9, 9, 9]).await;
    step(&h).await;
    // A duplicate seq is audited before it closes the session, never forwarded.
    client.send(&[1, 1]).await;
    let end = tokio::time::timeout(Duration::from_secs(5), &mut client.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end, SessionEnd::SeqRegression);
    let sent: Vec<Bytes> = client
        .frames()
        .into_iter()
        .filter_map(|f| match f {
            OutboundFrame::Binary(b) => Some(b),
            OutboundFrame::Close { .. } => None,
        })
        .collect();
    drain.shutdown().await;
    let ins = log.session_in();
    assert_eq!(
        ins.iter()
            .map(|r| (r.seq, r.frame.clone()))
            .collect::<Vec<_>>(),
        vec![
            (1, Bytes::from_static(&[1, 1])),
            (0, Bytes::from_static(&[9, 9, 9])),
            (1, Bytes::from_static(&[1, 1])),
        ]
    );
    assert_eq!(ins[0].tick_seen, seen.tick);
    let outs = log.session_out();
    assert_eq!(outs.iter().map(|r| r.frame.clone()).collect::<Vec<_>>(), sent);
    let accepted = outs
        .iter()
        .find(|r| r.frame.starts_with(b"out:Accepted"))
        .unwrap();
    assert_eq!(accepted.tick, seen.tick);
    assert!(ins
        .iter()
        .all(|r| r.session == client.id.as_uuid() && r.zone == seen.zone && r.epoch == seen.epoch));
    assert!(outs
        .iter()
        .all(|r| r.session == client.id.as_uuid() && r.zone == seen.zone && r.epoch == seen.epoch));
}
