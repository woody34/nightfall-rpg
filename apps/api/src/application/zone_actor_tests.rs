//! Unit tests for the zone actor, all driven by `ManualTicks` except the interval smoke test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicBool, Ordering};

use uuid::Uuid;

use super::*;
use crate::domain::zone::{
    Ordinal, RejectReason, SessionGeneration, Speed, Vec2Fixed, ZoneBounds, ZoneCommand, ZoneId,
    ZoneSeed,
};

const GEN: SessionGeneration = SessionGeneration(1);

fn state() -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(3),
            epoch: 1,
        },
        bounds,
        0,
    )
}

fn id(n: u128) -> EntityId {
    EntityId(Uuid::from_u128(n))
}

fn spawn(n: u128) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: id(n),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(10, 10),
        speed: Speed::DEFAULT,
        generation: GEN,
    })
}

fn stop(n: u128, seq: u32) -> ZoneInput {
    ZoneInput::session(id(n), GEN, seq, ZoneCommand::StopMove { entity: id(n) })
}

fn seqs(t: &AppliedTick) -> Vec<Option<u32>> {
    t.commands.iter().map(|c| c.seq).collect()
}

#[tokio::test]
async fn commands_are_applied_in_receive_order_with_ordinals_and_the_tick() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    let mut rx = zone.subscribe();
    zone.send(spawn(1)).unwrap();
    zone.send(spawn(2)).unwrap();
    zone.send(stop(1, 5)).unwrap();
    assert_eq!(driver.step().await, Ok(TickOutcome::Ran(Tick(0))));

    let t = rx.recv().await.unwrap();
    assert_eq!(t.tick, Tick(0));
    assert_eq!(t.epoch, 1);
    let ordinals: Vec<Ordinal> = t.commands.iter().map(|c| c.ordinal).collect();
    assert_eq!(ordinals, vec![Ordinal(0), Ordinal(1), Ordinal(2)]);
    assert_eq!(seqs(&t), vec![None, None, Some(5)]);
    assert_eq!(t.events.len(), 2);
}

#[tokio::test]
async fn a_session_gets_at_most_eight_commands_per_tick_and_the_rest_wait_in_order() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    let mut rx = zone.subscribe();
    zone.send(spawn(1)).unwrap();
    zone.send(spawn(2)).unwrap();
    driver.step().await.unwrap();
    rx.recv().await.unwrap();

    for seq in 0..10 {
        zone.send(stop(1, seq)).unwrap();
    }
    zone.send(stop(2, 100)).unwrap();
    driver.step().await.unwrap();
    let t = rx.recv().await.unwrap();
    let mut want: Vec<Option<u32>> = (0..8).map(Some).collect();
    want.push(Some(100));
    assert_eq!(seqs(&t), want, "the neighbour is not starved by the flood");
    assert_eq!(zone.stats().borrow().commands_deferred, 2);

    driver.step().await.unwrap();
    let t = rx.recv().await.unwrap();
    assert_eq!(seqs(&t), vec![Some(8), Some(9)]);
    assert_eq!(t.commands[0].ordinal, Ordinal(11));
}

#[tokio::test]
async fn a_full_queue_is_reported_not_fatal() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    for _ in 0..COMMAND_QUEUE {
        zone.send(ZoneInput::system(ZoneCommand::Despawn { entity: id(9) }))
            .unwrap();
    }
    let err = zone.send(spawn(1)).unwrap_err();
    assert!(matches!(err, ZoneSendError::Full(ZoneInput { .. })));
    driver.step().await.unwrap();
    zone.send(spawn(1)).unwrap();
}

#[tokio::test]
async fn rejected_commands_reach_the_issuing_session_output() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    let mut rx = zone.subscribe();
    zone.send(spawn(1)).unwrap();
    driver.step().await.unwrap();
    rx.recv().await.unwrap();
    let far = ZoneCommand::MoveTo {
        entity: id(1),
        dest: Vec2Fixed::from_tiles(200, 200),
    };
    zone.send(ZoneInput::session(id(1), GEN, 77, far)).unwrap();
    driver.step().await.unwrap();
    let t = rx.recv().await.unwrap();
    assert_eq!(t.dispositions.len(), 1);
    assert_eq!(t.dispositions[0].reason, RejectReason::TooFar);
    assert_eq!(t.dispositions[0].tick_seen, Tick(1));
    assert!(matches!(
        t.outputs[&id(1)].as_slice(),
        [crate::domain::zone::ObserverOutput::Rejected(d)] if d.seq == Some(77)
    ));
}

#[tokio::test]
async fn snapshots_wait_for_a_boundary_without_deferred_commands() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    zone.send(spawn(1)).unwrap();
    driver.step().await.unwrap();
    for seq in 0..12 {
        zone.send(stop(1, seq)).unwrap();
    }
    driver.step().await.unwrap();
    assert_eq!(zone.stats().borrow().commands_deferred, 4);

    let pending = tokio::spawn({
        let zone = zone.clone();
        async move { zone.snapshot().await }
    });
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    driver.step().await.unwrap();
    let snap = pending.await.unwrap().unwrap();
    assert_eq!(snap.tick, Tick(3));
    assert_eq!(snap.next_ordinal, Ordinal(13));
    assert_eq!(ZoneState::from_snapshot(snap.clone()).unwrap().snapshot(), snap);
}

/// Refuses while `closed` is set.
struct SwitchGate(Arc<AtomicBool>);

impl TickGate for SwitchGate {
    fn admit(&self, _tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send {
        std::future::ready(if self.0.load(Ordering::SeqCst) {
            Err(GateError("log unavailable".to_owned()))
        } else {
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_held_tick_releases_nothing_and_offers_the_same_record_again() {
    let closed = Arc::new(AtomicBool::new(true));
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state(), ticks, SwitchGate(closed.clone()));
    let mut rx = zone.subscribe();
    zone.send(spawn(1)).unwrap();
    assert_eq!(driver.step().await, Ok(TickOutcome::Held(Tick(0))));
    assert_eq!(driver.step().await, Ok(TickOutcome::Held(Tick(0))));
    assert_eq!(zone.stats().borrow().gate_holds, 2);
    assert!(rx.try_recv().is_err());
    zone.send(spawn(2)).unwrap();

    closed.store(false, Ordering::SeqCst);
    assert_eq!(driver.step().await, Ok(TickOutcome::Ran(Tick(0))));
    let t = rx.recv().await.unwrap();
    assert_eq!(t.commands.len(), 1, "inputs sent while held wait for the next tick");
    assert_eq!(t.commands[0].ordinal, Ordinal(0));
    assert_eq!(driver.step().await, Ok(TickOutcome::Ran(Tick(1))));
    assert_eq!(rx.recv().await.unwrap().commands[0].ordinal, Ordinal(1));
}

#[tokio::test]
async fn no_snapshot_is_taken_while_a_record_is_unrecorded() {
    let closed = Arc::new(AtomicBool::new(true));
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state(), ticks, SwitchGate(closed.clone()));
    zone.send(spawn(1)).unwrap();
    driver.step().await.unwrap();
    let pending = tokio::spawn({
        let zone = zone.clone();
        async move { zone.snapshot().await }
    });
    tokio::task::yield_now().await;
    driver.step().await.unwrap();
    assert!(!pending.is_finished());
    closed.store(false, Ordering::SeqCst);
    driver.step().await.unwrap();
    let snap = pending.await.unwrap().unwrap();
    assert_eq!((snap.tick, snap.entities.len()), (Tick(1), 1));
}

/// Reports the zone paused.
struct PausedGate(watch::Sender<bool>);

impl TickGate for PausedGate {
    fn admit(&self, _tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send {
        std::future::ready(Ok(()))
    }

    fn paused(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

#[tokio::test]
async fn a_paused_zone_refuses_inputs() {
    let (ticks, _driver) = manual_ticks();
    let (tx, _) = watch::channel(false);
    let zone = ZoneActor::spawn_gated(state(), ticks, PausedGate(tx.clone()));
    zone.send(spawn(1)).unwrap();
    tx.send_replace(true);
    assert!(zone.is_paused());
    assert!(matches!(zone.send(spawn(2)), Err(ZoneSendError::Paused(_))));
    tx.send_replace(false);
    zone.send(spawn(2)).unwrap();
}

#[tokio::test]
async fn stats_are_published_every_tick() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    let mut stats = zone.stats();
    zone.send(spawn(1)).unwrap();
    driver.step().await.unwrap();
    assert!(stats.has_changed().unwrap());
    let s = *stats.borrow_and_update();
    assert_eq!((s.tick, s.entities, s.commands_applied), (Tick(0), 1, 1));
    driver.step().await.unwrap();
    assert_eq!(stats.borrow_and_update().tick, Tick(1));
}

#[tokio::test]
async fn the_actor_stops_when_every_handle_is_dropped() {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state(), ticks);
    zone.send(spawn(1)).unwrap();
    drop(zone);
    assert_eq!(driver.step().await, Ok(TickOutcome::Ran(Tick(0))));
    assert_eq!(driver.step().await, Err(ActorStopped));
}

#[tokio::test]
async fn the_interval_source_ticks_on_its_own() {
    let zone = ZoneActor::spawn(state(), IntervalTicks::new());
    let mut stats = zone.stats();
    tokio::time::timeout(Duration::from_secs(5), async {
        while stats.borrow_and_update().tick < Tick(2) {
            stats.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
