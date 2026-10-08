//! The zone actor (Story 3.1): one tokio task that owns a [`ZoneState`], receives
//! [`ZoneInput`]s on a bounded queue, and runs one tick per beat of an injected
//! [`TickSource`].
//!
//! Each tick has three phases (plan §8 #2, #3; architecture.md §2.5):
//!
//! 1. **Draft.** Take this tick's inputs in receive order, at most
//!    [`SESSION_COMMANDS_PER_TICK`] per session (the excess waits, in order, for the next tick),
//!    and assign them ordinals: an [`AppliedTickDraft`](crate::domain::zone::AppliedTickDraft).
//! 2. **Run.** [`ZoneState::run_tick`] applies the draft, moves entities and diffs every
//!    player's AOI into an [`AppliedTick`]. Nothing has left the actor yet.
//! 3. **Gate.** Await the injected [`TickGate`] with the finished record. Story 3.2's
//!    `DurableTickGate` makes this the acknowledged `JetStream` append of the tick's record,
//!    so nothing is released that is not durably logged. Only then is the record broadcast.
//!    If the gate refuses, the record is kept, nothing is broadcast, no later tick is drafted,
//!    and the same record is offered again on the next beat.
//!
//! The gate sees the whole record rather than the draft because the record carries each
//! player's encoded output, which replay compares byte for byte and which only exists after
//! the tick has run. Running before the gate is safe: the state lives only in this task, a
//! snapshot is never taken while a record is unrecorded, and a crash loses the in-memory state
//! together with the unlogged tick, so the log and everything anyone observed always agree.
//!
//! The actor never reads a clock for anything that reaches state or output: tick numbers come
//! from the state, `server_time_ms` from the zone's time origin. The one clock read is a
//! monotonic `Instant` around the tick body that feeds [`TickStats::duration_micros`] only.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::MissedTickBehavior;

use crate::domain::zone::{
    AppliedTick, CommandSource, EntityId, Tick, TickError, ZoneInput, ZoneSnapshot, ZoneState,
    TICK_MS,
};

/// Capacity of the inbound queue. When full, [`ZoneHandle::send`] fails with
/// [`ZoneSendError::Full`] and the session reports `OVERLOADED` to its client (plan §8 #12);
/// the zone itself never blocks or drops silently.
pub const COMMAND_QUEUE: usize = 1024;

/// Most commands one session may have applied per tick (plan §8 #12). Excess is deferred in
/// order to later ticks, so a flooding client slows only itself.
pub const SESSION_COMMANDS_PER_TICK: usize = 8;

/// Most commands the actor holds after taking them off the queue (deferred included). Past
/// this it stops draining, the queue fills, and senders see `Full`.
const PENDING_LIMIT: usize = 1024;

/// Applied ticks kept for slow subscribers: 6.4 s at 10 Hz. A subscriber further behind gets
/// `RecvError::Lagged` and must resynchronise (the session closes and the client reconnects).
const BROADCAST_TICKS: usize = 64;

/// What drives the tick loop. A real 100 ms interval in production; a manual stepper in tests
/// and replay.
pub trait TickSource: Send + 'static {
    /// Resolves when the next tick is due; `false` stops the actor. Must be cancel-safe: the
    /// actor drops the future when a snapshot request arrives first.
    fn next_tick(&mut self) -> impl Future<Output = bool> + Send;

    /// Called after every beat with what happened. Default: nothing.
    fn tick_done(&mut self, _outcome: TickOutcome) {}
}

/// What one beat of the tick source did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// The tick ran, the gate admitted its record, and it was broadcast (if not idle).
    Ran(Tick),
    /// The gate refused the tick's record: nothing was released and no later tick runs until
    /// the same record is admitted.
    Held(Tick),
}

/// Fires every [`TICK_MS`] on tokio's clock.
#[derive(Debug)]
pub struct IntervalTicks {
    interval: tokio::time::Interval,
}

impl IntervalTicks {
    /// A 100 ms interval. Missed beats are run back to back (`Burst`) so the tick count keeps
    /// pace with wall time after a stall; the simulation itself only sees tick numbers, so
    /// this changes latency, never results.
    #[must_use]
    pub fn new() -> Self {
        let mut interval = tokio::time::interval(Duration::from_millis(TICK_MS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Burst);
        Self { interval }
    }
}

impl Default for IntervalTicks {
    fn default() -> Self {
        Self::new()
    }
}

impl TickSource for IntervalTicks {
    async fn next_tick(&mut self) -> bool {
        self.interval.tick().await;
        true
    }
}

/// Tick source stepped by hand through its [`ManualTickDriver`]. Used by tests and replay.
#[derive(Debug)]
pub struct ManualTicks {
    requests: mpsc::Receiver<oneshot::Sender<TickOutcome>>,
    waiting: Option<oneshot::Sender<TickOutcome>>,
}

/// Steps a [`ManualTicks`] source.
#[derive(Debug, Clone)]
pub struct ManualTickDriver {
    requests: mpsc::Sender<oneshot::Sender<TickOutcome>>,
}

/// A manual tick source and its driver.
#[must_use]
pub fn manual_ticks() -> (ManualTicks, ManualTickDriver) {
    let (tx, rx) = mpsc::channel(1);
    (
        ManualTicks {
            requests: rx,
            waiting: None,
        },
        ManualTickDriver { requests: tx },
    )
}

impl ManualTickDriver {
    /// Runs one beat and waits until its record has been broadcast. Every input sent before
    /// this call is eligible for the tick.
    pub async fn step(&self) -> Result<TickOutcome, ActorStopped> {
        let (tx, rx) = oneshot::channel();
        self.requests.send(tx).await.map_err(|_| ActorStopped)?;
        rx.await.map_err(|_| ActorStopped)
    }
}

impl TickSource for ManualTicks {
    async fn next_tick(&mut self) -> bool {
        // `recv` is cancel-safe; the reply slot is stored only once a request is taken.
        match self.requests.recv().await {
            Some(reply) => {
                self.waiting = Some(reply);
                true
            },
            None => false,
        }
    }

    fn tick_done(&mut self, outcome: TickOutcome) {
        if let Some(reply) = self.waiting.take() {
            // The driver may have stopped waiting; nothing to do then.
            let _ = reply.send(outcome);
        }
    }
}

/// Why a tick was not admitted.
#[derive(Debug, Error)]
#[error("tick gate refused the record: {0}")]
pub struct GateError(pub String);

/// Admission control between running a tick and releasing it (plan §8 #2).
///
/// The actor awaits `admit` with the finished record before broadcasting it or drafting the
/// next tick. `Ok` releases it; `Err` holds it: nothing is broadcast, the zone does not move on,
/// and the same record is offered again on the next beat. `DurableTickGate`
/// (`application::replay_log`) implements this as the acknowledged `JetStream` append of the
/// tick's record, so the log is always ahead of anything observable. A durable gate may also
/// wait (retrying) inside `admit`; the actor simply stalls. The tick budget is 100 ms.
pub trait TickGate: Send + Sync + 'static {
    /// Admits or holds one tick's record.
    fn admit(&self, tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send;

    /// `true` while the zone is degraded (the gate has been unable to admit for longer than
    /// it tolerates). [`ZoneHandle::send`] refuses inputs with [`ZoneSendError::Paused`] then.
    /// Default: never paused.
    fn paused(&self) -> watch::Receiver<bool> {
        watch::channel(false).1
    }
}

/// Admits everything. For tests and replay, where nothing needs to be logged.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenGate;

impl TickGate for OpenGate {
    fn admit(&self, _tick: &AppliedTick) -> impl Future<Output = Result<(), GateError>> + Send {
        std::future::ready(Ok(()))
    }
}

/// Per-tick instrumentation, published on a watch channel for the telemetry epic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickStats {
    /// The tick that ran (or was held).
    pub tick: Tick,
    /// Wall time of the tick body. Metrics only; never reaches state or output.
    pub duration_micros: u64,
    /// Entities in the zone after the tick.
    pub entities: usize,
    /// Commands applied (accepted or rejected) on this tick.
    pub commands_applied: usize,
    /// Commands waiting for a later tick (session budget or a held tick).
    pub commands_deferred: usize,
    /// Ticks held by the gate since the actor started.
    pub gate_holds: u64,
}

/// A command could not be queued. The input is handed back.
#[derive(Debug, Error)]
pub enum ZoneSendError {
    /// The queue is full: tell the client `OVERLOADED` and carry on.
    #[error("zone command queue is full")]
    Full(ZoneInput),
    /// The zone is paused because its replay log is unavailable: tell the client
    /// `OVERLOADED`. Nothing is queued, so nothing is applied unlogged.
    #[error("zone is paused: its replay log is unavailable")]
    Paused(ZoneInput),
    /// The actor has stopped.
    #[error("zone actor has stopped")]
    Closed(ZoneInput),
}

/// The actor is no longer running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("zone actor has stopped")]
pub struct ActorStopped;

/// Cheap, cloneable access to a running zone.
#[derive(Debug, Clone)]
pub struct ZoneHandle {
    commands: mpsc::Sender<ZoneInput>,
    snapshots: mpsc::Sender<oneshot::Sender<ZoneSnapshot>>,
    ticks: broadcast::Sender<Arc<AppliedTick>>,
    stats: watch::Receiver<TickStats>,
    paused: watch::Receiver<bool>,
}

impl ZoneHandle {
    /// Queues an input for the next tick without waiting. Inputs from one handle are applied
    /// in the order sent. Refused with [`ZoneSendError::Paused`] while the gate reports the
    /// zone paused.
    pub fn send(&self, input: ZoneInput) -> Result<(), ZoneSendError> {
        if *self.paused.borrow() {
            return Err(ZoneSendError::Paused(input));
        }
        self.commands.try_send(input).map_err(|e| match e {
            mpsc::error::TrySendError::Full(i) => ZoneSendError::Full(i),
            mpsc::error::TrySendError::Closed(i) => ZoneSendError::Closed(i),
        })
    }

    /// Every non-idle tick's record, one batch per tick, in tick order.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<AppliedTick>> {
        self.ticks.subscribe()
    }

    /// The zone at the next tick boundary with no deferred commands (plan §8 #5). Waits for
    /// such a boundary if commands are currently deferred.
    pub async fn snapshot(&self) -> Result<ZoneSnapshot, ActorStopped> {
        let (tx, rx) = oneshot::channel();
        self.snapshots.send(tx).await.map_err(|_| ActorStopped)?;
        rx.await.map_err(|_| ActorStopped)
    }

    /// Per-tick statistics.
    #[must_use]
    pub fn stats(&self) -> watch::Receiver<TickStats> {
        self.stats.clone()
    }

    /// Whether the zone is currently paused (see [`TickGate::paused`]).
    #[must_use]
    pub fn is_paused(&self) -> bool {
        *self.paused.borrow()
    }

    /// Resolves once the actor task has ended.
    pub async fn stopped(&self) {
        self.commands.closed().await;
    }
}

/// The task that owns a zone. Construct with [`ZoneActor::spawn`].
pub struct ZoneActor<T, G> {
    state: ZoneState,
    ticks: T,
    gate: G,
    commands: mpsc::Receiver<ZoneInput>,
    commands_closed: bool,
    snapshots: mpsc::Receiver<oneshot::Sender<ZoneSnapshot>>,
    waiting_snapshots: Vec<oneshot::Sender<ZoneSnapshot>>,
    pending: VecDeque<ZoneInput>,
    /// A tick that ran but whose record the gate has not admitted yet. While set, no new tick
    /// is drafted and no snapshot is taken.
    unrecorded: Option<Arc<AppliedTick>>,
    out: broadcast::Sender<Arc<AppliedTick>>,
    stats: watch::Sender<TickStats>,
    gate_holds: u64,
}

impl<T: TickSource> ZoneActor<T, OpenGate> {
    /// Starts the actor with no admission gate. Must be called inside a tokio runtime.
    pub fn spawn(state: ZoneState, ticks: T) -> ZoneHandle {
        ZoneActor::spawn_gated(state, ticks, OpenGate)
    }
}

impl<T: TickSource, G: TickGate> ZoneActor<T, G> {
    /// Starts the actor with an admission gate. Must be called inside a tokio runtime. The
    /// task ends when the tick source stops or every handle has been dropped and the queue
    /// is drained.
    pub fn spawn_gated(state: ZoneState, ticks: T, gate: G) -> ZoneHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_QUEUE);
        let (snap_tx, snap_rx) = mpsc::channel(8);
        let (out_tx, _) = broadcast::channel(BROADCAST_TICKS);
        let (stats_tx, stats_rx) = watch::channel(TickStats::default());
        let paused = gate.paused();
        let actor = Self {
            state,
            ticks,
            gate,
            commands: cmd_rx,
            commands_closed: false,
            snapshots: snap_rx,
            waiting_snapshots: Vec::new(),
            pending: VecDeque::new(),
            unrecorded: None,
            out: out_tx.clone(),
            stats: stats_tx,
            gate_holds: 0,
        };
        tokio::spawn(actor.run());
        ZoneHandle {
            commands: cmd_tx,
            snapshots: snap_tx,
            ticks: out_tx,
            stats: stats_rx,
            paused,
        }
    }

    async fn run(mut self) {
        loop {
            tokio::select! {
                biased;
                Some(reply) = self.snapshots.recv() => {
                    self.waiting_snapshots.push(reply);
                    self.answer_snapshots();
                },
                go = self.ticks.next_tick() => {
                    if !go {
                        break;
                    }
                    let outcome = self.tick().await;
                    self.ticks.tick_done(outcome);
                    self.answer_snapshots();
                    if self.commands_closed && self.pending.is_empty() && self.unrecorded.is_none() {
                        break;
                    }
                },
            }
        }
        tracing::debug!(zone = self.state.seed().zone.0, "zone actor stopped");
    }

    /// Snapshots are only taken at a tick boundary with nothing deferred and nothing
    /// unrecorded.
    fn answer_snapshots(&mut self) {
        if self.pending.is_empty()
            && self.unrecorded.is_none()
            && !self.waiting_snapshots.is_empty()
        {
            let snap = self.state.snapshot();
            for reply in self.waiting_snapshots.drain(..) {
                let _ = reply.send(snap.clone());
            }
        }
    }

    async fn tick(&mut self) -> TickOutcome {
        // Metrics only (see module docs).
        let started = Instant::now();
        self.collect();
        let (record, applied) = match self.unrecorded.take() {
            Some(record) => (record, 0),
            None => match self.run_next() {
                Ok(ran) => ran,
                Err((tick, e)) => {
                    // Unreachable by construction: the draft came from this state just above.
                    tracing::error!(tick = tick.0, error = %e, "draft did not continue the zone");
                    self.publish_stats(tick, started, 0);
                    return TickOutcome::Ran(tick);
                },
            },
        };
        let tick = record.tick;

        if let Err(e) = self.gate.admit(&record).await {
            self.gate_holds = self.gate_holds.saturating_add(1);
            tracing::warn!(tick = tick.0, error = %e, "tick held by gate");
            self.unrecorded = Some(record);
            self.publish_stats(tick, started, 0);
            return TickOutcome::Held(tick);
        }
        if !record.is_idle() {
            // No subscribers is fine: nobody is connected yet.
            let _ = self.out.send(record);
        }
        self.publish_stats(tick, started, applied);
        TickOutcome::Ran(tick)
    }

    /// Drafts and runs the next tick, consuming its inputs from `pending`.
    fn run_next(&mut self) -> Result<(Arc<AppliedTick>, usize), (Tick, TickError)> {
        let admitted = self.select_admitted();
        let inputs: Vec<ZoneInput> = admitted
            .iter()
            .filter_map(|i| self.pending.get(*i).cloned())
            .collect();
        let draft = self.state.draft(inputs);
        let tick = draft.tick;
        let applied = draft.commands.len();
        let mut index = 0_usize;
        let mut next_admitted = admitted.iter().peekable();
        self.pending.retain(|_| {
            let keep = next_admitted.next_if_eq(&&index).is_none();
            index = index.saturating_add(1);
            keep
        });
        let record = self.state.run_tick(draft).map_err(|e| (tick, e))?;
        Ok((Arc::new(record), applied))
    }

    /// Moves queued inputs into `pending`, up to [`PENDING_LIMIT`].
    fn collect(&mut self) {
        while self.pending.len() < PENDING_LIMIT {
            match self.commands.try_recv() {
                Ok(input) => self.pending.push_back(input),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.commands_closed = true;
                    break;
                },
            }
        }
    }

    /// Indices into `pending`, ascending, of the inputs that fit this tick's budgets.
    fn select_admitted(&self) -> Vec<usize> {
        let mut per_session: BTreeMap<EntityId, usize> = BTreeMap::new();
        let mut admitted = Vec::new();
        for (i, input) in self.pending.iter().enumerate() {
            if let CommandSource::Session { entity, .. } = input.source {
                let used = per_session.entry(entity).or_default();
                if *used >= SESSION_COMMANDS_PER_TICK {
                    continue;
                }
                *used = used.saturating_add(1);
            }
            admitted.push(i);
        }
        admitted
    }

    fn publish_stats(&mut self, tick: Tick, started: Instant, applied: usize) {
        let duration = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.stats.send_replace(TickStats {
            tick,
            duration_micros: duration,
            entities: self.state.entity_count(),
            commands_applied: applied,
            commands_deferred: self.pending.len(),
            gate_holds: self.gate_holds,
        });
    }
}

#[cfg(test)]
#[path = "zone_actor_tests.rs"]
mod tests;
