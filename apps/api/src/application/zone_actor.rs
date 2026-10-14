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
//! from the state, `server_time_ms` from the zone's time origin. Monotonic timing feeds only
//! [`TickStats::duration_micros`] and opt-in runtime phase tracing.

#[path = "zone_actor_grants.rs"]
mod grants;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::MissedTickBehavior;

use super::trace::TraceCarrier;
use super::zone_admission::{
    failure, reserve_recipients, AdmissionFailure, AdmissionFailureReason, RecordAdmissionPolicy,
};
use crate::domain::zone::{
    AppliedTick, CommandSource, EntityId, Ordinal, Tick, TickError, ZoneInput, ZoneSnapshot,
    ZoneState, TICK_MS,
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
/// and the same record is offered again on the next beat, unless `terminal_failure` fences
/// the actor permanently. `DurableTickGate`
/// (`application::replay_log`) implements this as the acknowledged `JetStream` append of the
/// tick's record, so the log is always ahead of anything observable. A durable gate may also
/// wait (retrying) inside `admit`; the actor simply stalls. The tick budget is 100 ms.
pub trait TickGate: Send + Sync + 'static {
    /// Maximum encoded record accepted by this gate. Durable adapters must expose their
    /// actual log ceiling before experimental V5 is enabled in production.
    fn max_record_bytes(&self) -> Option<usize> {
        None
    }

    /// Whether admission proves an acknowledged replay-log append.
    fn durable(&self) -> bool {
        false
    }

    /// A permanent record invariant failed. The actor must stop without publishing the
    /// speculative state or a recovery snapshot; retrying the same record cannot help.
    fn terminal_failure(&self) -> bool {
        false
    }

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

/// Synchronous, process-local telemetry subscriber. Called only by the live actor;
/// replay runs the domain directly. Implementations must be cheap and non-blocking.
pub trait TickTelemetry: Send {
    /// One admitted zone-wide batch, independent of observer/session fan-out.
    fn admitted(&mut self, tick: &AppliedTick);
    /// One tick-body measurement, including idle ticks and gate holds.
    fn stats(&mut self, stats: TickStats);
}

/// Creates a separate subscriber for each live zone, seeded before its first tick.
pub trait ZoneTelemetry: Send + Sync {
    /// Seed metadata needed to classify events from a restored initial state.
    fn consumer(&self, initial: &ZoneSnapshot) -> Box<dyn TickTelemetry>;
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
    /// Terminal byte/cardinality admission failure; the input was never queued.
    #[error("ADMISSION_TOO_LARGE")]
    AdmissionTooLarge(ZoneInput),
    /// The actor has stopped.
    #[error("zone actor has stopped")]
    Closed(ZoneInput),
}

/// The actor is no longer running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("zone actor has stopped")]
pub struct ActorStopped;

/// An input on the queue, with the trace it was sent from (plan §8 #18). The carrier never
/// reaches the draft, the state or the log: it only parents the actor's `zone.apply` span.
#[derive(Debug)]
struct Queued {
    input: ZoneInput,
    trace: Option<TraceCarrier>,
    reply: Option<MutationReply>,
}

#[derive(Debug)]
enum MutationReply {
    Grant(oneshot::Sender<Result<super::ports::ItemMutationReceipt, super::AppError>>),
    Transfer(
        oneshot::Sender<
            Result<crate::domain::character_progression::FrozenTransferResult, super::AppError>,
        >,
    ),
    Learn(oneshot::Sender<Result<super::ports::FrozenLearnResult, super::AppError>>),
}

impl MutationReply {
    fn fail(self, error: super::AppError) {
        match self {
            Self::Transfer(reply) => {
                let _ = reply.send(Err(error));
            },
            Self::Learn(reply) => {
                let _ = reply.send(Err(error));
            },
            Self::Grant(reply) => {
                let _ = reply.send(Err(error));
            },
        }
    }
}

/// Cheap, cloneable access to a running zone.
#[derive(Debug, Clone)]
pub struct ZoneHandle {
    phase4_durable: bool,
    phase2: bool,
    phase3: bool,
    admission_policy: Option<RecordAdmissionPolicy>,
    admission_failures: broadcast::Sender<AdmissionFailure>,
    /// Serial persistence lane, installed before admitting sockets.
    pub checkpoints: super::checkpoint::CheckpointLane,
    commands: mpsc::Sender<Queued>,
    snapshots: mpsc::Sender<oneshot::Sender<ZoneSnapshot>>,
    ticks: broadcast::Sender<Arc<AppliedTick>>,
    stats: watch::Receiver<TickStats>,
    paused: watch::Receiver<bool>,
    final_snapshot: Arc<parking_lot::RwLock<Option<ZoneSnapshot>>>,
    persistence_failed: Arc<std::sync::atomic::AtomicBool>,
    audit_seed: crate::domain::zone::ZoneSeed,
    audit_next_tick: watch::Receiver<Tick>,
}

impl ZoneHandle {
    /// Whether authoritative Phase 2 identity is required at admission.
    pub fn has_classes(&self) -> bool {
        self.phase2
    }

    /// Whether this actor was created from an explicitly experimental V5 state.
    pub fn phase3_enabled(&self) -> bool {
        self.phase3
    }

    /// Current zone position for best-effort session audit. The next tick follows state
    /// advancement, including idle ticks and ticks waiting for durable admission.
    pub fn audit_context(&self) -> super::SessionAuditContext {
        super::SessionAuditContext {
            zone: self.audit_seed.zone,
            epoch: self.audit_seed.epoch,
            tick: *self.audit_next_tick.borrow(),
        }
    }

    /// True when the actor stopped with an unresolved persistence fence.
    pub fn persistence_failed(&self) -> bool {
        self.persistence_failed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn inventory_available(&self) -> Result<(), super::AppError> {
        if self.is_paused() || self.commands.is_closed() || self.persistence_failed() {
            return Err(super::AppError::Unavailable("inventory zone is unavailable".into()));
        }
        Ok(())
    }

    pub(crate) fn final_snapshot(&self) -> Option<ZoneSnapshot> {
        self.final_snapshot.read().clone()
    }

    /// Queues an input for the next tick without waiting. Inputs from one handle are applied
    /// in the order sent. Refused with [`ZoneSendError::Paused`] while the gate reports the
    /// zone paused.
    pub fn send(&self, input: ZoneInput) -> Result<(), ZoneSendError> {
        self.enqueue(Queued {
            input,
            trace: None,
            reply: None,
        })
    }

    /// [`Self::send`], continuing the trace in `trace`: the actor records a `zone.apply` span
    /// parented on it when the command is applied, so one intent is one trace from socket to
    /// broadcast.
    pub fn send_traced(&self, input: ZoneInput, trace: TraceCarrier) -> Result<(), ZoneSendError> {
        self.enqueue(Queued {
            input,
            trace: Some(trace),
            reply: None,
        })
    }

    /// Queues an input, waiting for room if the queue is full. For lifecycle commands
    /// (spawn, replace, despawn) that must not be lost to a momentary burst; player intents
    /// use [`Self::send`] and are refused with `OVERLOADED` instead.
    pub async fn send_wait(&self, input: ZoneInput) -> Result<(), ActorStopped> {
        self.send_wait_checked(input)
            .await
            .map_err(|_| ActorStopped)
    }

    /// Lifecycle/system enqueue with typed terminal `RESOURCE_EXHAUSTED` refusal. Subscribe to
    /// admission failures before calling for cardinality failures decided at selection.
    pub async fn send_wait_checked(&self, input: ZoneInput) -> Result<(), super::AppError> {
        self.check_input(&input)
            .map_err(|_| super::AppError::ResourceExhausted("ADMISSION_TOO_LARGE".into()))?;
        self.commands
            .send(Queued {
                input,
                trace: None,
                reply: None,
            })
            .await
            .map_err(|_| super::AppError::Unavailable("zone stopped".into()))
    }

    /// Terminal ingress failures, separate from authoritative tick output. System producers
    /// must subscribe before enqueueing lifecycle work; sessions do so before admission.
    pub fn subscribe_admission_failures(&self) -> broadcast::Receiver<AdmissionFailure> {
        self.admission_failures.subscribe()
    }

    fn check_input(&self, input: &ZoneInput) -> Result<(), AdmissionFailure> {
        if let Some(policy) = self.admission_policy {
            if let Err(reason) = policy.measure(input) {
                let notice = failure(input, policy, reason);
                log_admission_failure(input, &notice);
                return Err(notice);
            }
        }
        Ok(())
    }

    fn enqueue(&self, q: Queued) -> Result<(), ZoneSendError> {
        if self.check_input(&q.input).is_err() {
            return Err(ZoneSendError::AdmissionTooLarge(q.input));
        }
        if *self.paused.borrow() {
            return Err(ZoneSendError::Paused(q.input));
        }
        self.commands.try_send(q).map_err(|e| match e {
            mpsc::error::TrySendError::Full(q) => ZoneSendError::Full(q.input),
            mpsc::error::TrySendError::Closed(q) => ZoneSendError::Closed(q.input),
        })
    }

    /// Enqueues an authenticated transfer; the receiver is completed only after checkpoint.
    pub fn enqueue_class_transfer(
        &self,
        input: ZoneInput,
    ) -> Result<
        oneshot::Receiver<
            Result<crate::domain::character_progression::FrozenTransferResult, super::AppError>,
        >,
        super::AppError,
    > {
        let (reply, receiver) = oneshot::channel();
        self.enqueue(Queued {
            input,
            trace: None,
            reply: Some(MutationReply::Transfer(reply)),
        })
        .map_err(|e| match e {
            ZoneSendError::AdmissionTooLarge(_) => {
                super::AppError::ResourceExhausted("ADMISSION_TOO_LARGE".into())
            },
            ZoneSendError::Full(_) => {
                super::AppError::ResourceExhausted("zone command queue is full".into())
            },
            ZoneSendError::Paused(_) | ZoneSendError::Closed(_) => {
                super::AppError::Unavailable("zone is unavailable".into())
            },
        })?;
        Ok(receiver)
    }

    /// Queues exact-rank learning; success awaits the atomic durable receipt transaction.
    pub fn enqueue_learn_skill(
        &self,
        input: ZoneInput,
    ) -> Result<super::ports::LearnSkillReply, super::AppError> {
        if !self.phase3 {
            return Err(super::AppError::Unavailable("phase 3 runtime unavailable".into()));
        }
        let (reply, receiver) = oneshot::channel();
        self.enqueue(Queued {
            input,
            trace: None,
            reply: Some(MutationReply::Learn(reply)),
        })
        .map_err(|error| match error {
            ZoneSendError::Full(_) | ZoneSendError::AdmissionTooLarge(_) => {
                super::AppError::ResourceExhausted("learning queue unavailable".into())
            },
            ZoneSendError::Paused(_) | ZoneSendError::Closed(_) => {
                super::AppError::Unavailable("zone is unavailable".into())
            },
        })?;
        Ok(receiver)
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
    failed: bool,
    admission_policy: Option<RecordAdmissionPolicy>,
    admission_failures: broadcast::Sender<AdmissionFailure>,
    replies: BTreeMap<Ordinal, MutationReply>,
    audit_next_tick: watch::Sender<Tick>,
    checkpoints: super::checkpoint::CheckpointLane,
    final_snapshot: Arc<parking_lot::RwLock<Option<ZoneSnapshot>>>,
    persistence_failed: Arc<std::sync::atomic::AtomicBool>,
    state: ZoneState,
    ticks: T,
    gate: G,
    commands: mpsc::Receiver<Queued>,
    commands_closed: bool,
    snapshots: mpsc::Receiver<oneshot::Sender<ZoneSnapshot>>,
    waiting_snapshots: Vec<oneshot::Sender<ZoneSnapshot>>,
    pending: VecDeque<Queued>,
    /// A tick that ran but whose record the gate has not admitted yet. While set, no new tick
    /// is drafted and no snapshot is taken.
    unrecorded: Option<Arc<AppliedTick>>,
    out: broadcast::Sender<Arc<AppliedTick>>,
    stats: watch::Sender<TickStats>,
    gate_holds: u64,
    telemetry: Option<(broadcast::Receiver<Arc<AppliedTick>>, Box<dyn TickTelemetry>)>,
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
        Self::spawn_gated_with_telemetry(state, ticks, gate, None)
    }

    /// Starts with one telemetry subscription installed before the actor can run.
    /// The actor drains it synchronously after broadcast, so this subscriber cannot lag
    /// or lose events even when manual ticks run faster than the runtime can schedule tasks.
    pub fn spawn_gated_with_telemetry(
        state: ZoneState,
        ticks: T,
        gate: G,
        telemetry: Option<Box<dyn TickTelemetry>>,
    ) -> ZoneHandle {
        let policy = state
            .phase3_admission_policy()
            .copied()
            .and_then(|policy| RecordAdmissionPolicy::try_from(policy).ok());
        Self::spawn_with_policy(state, ticks, gate, telemetry, policy)
    }

    /// Experimental construction with an explicit validated byte policy. A bad policy is
    /// refused before a task, draft or simulation is created.
    pub fn try_spawn_gated_with_admission(
        state: ZoneState,
        ticks: T,
        gate: G,
        policy: RecordAdmissionPolicy,
    ) -> Result<ZoneHandle, super::replay_log::AdmissionSizingError> {
        let ceiling = gate
            .max_record_bytes()
            .or_else(|| (!gate.durable()).then_some(usize::MAX))
            .ok_or(super::replay_log::AdmissionSizingError::InvalidBudget)?;
        policy.validate(ceiling)?;
        if state.phase3_enabled()
            && state
                .phase3_admission_policy()
                .copied()
                .and_then(|p| RecordAdmissionPolicy::try_from(p).ok())
                != Some(policy)
        {
            return Err(super::replay_log::AdmissionSizingError::InvalidBudget);
        }
        if observer_count(&state) > policy.max_observers {
            return Err(super::replay_log::AdmissionSizingError::InvalidBudget);
        }
        Ok(Self::spawn_with_policy(state, ticks, gate, None, Some(policy)))
    }

    fn spawn_with_policy(
        state: ZoneState,
        ticks: T,
        gate: G,
        telemetry: Option<Box<dyn TickTelemetry>>,
        admission_policy: Option<RecordAdmissionPolicy>,
    ) -> ZoneHandle {
        let recorded = state
            .phase3_admission_policy()
            .copied()
            .and_then(|policy| RecordAdmissionPolicy::try_from(policy).ok());
        let invalid_policy = (state.phase3_enabled()
            && (recorded.is_none() || admission_policy != recorded))
            || admission_policy.is_some_and(|policy| {
                gate.max_record_bytes()
                    .or_else(|| (!gate.durable()).then_some(usize::MAX))
                    .is_none_or(|ceiling| policy.validate(ceiling).is_err())
                    || observer_count(&state) > policy.max_observers
            });
        let (admission_failures, _) = broadcast::channel(COMMAND_QUEUE);
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_QUEUE);
        let (snap_tx, snap_rx) = mpsc::channel(8);
        let (out_tx, _) = broadcast::channel(BROADCAST_TICKS);
        let (stats_tx, stats_rx) = watch::channel(TickStats::default());
        let paused = gate.paused();
        let phase4_durable = !invalid_policy && state.phase4_enabled() && gate.durable();
        let phase2 = state.has_classes();
        let phase3 = state.phase3_enabled();
        let audit_seed = state.seed();
        let (audit_next_tick_tx, audit_next_tick_rx) = watch::channel(state.next_tick());
        let checkpoints = super::checkpoint::CheckpointLane::with_snapshot(state.snapshot());
        let final_snapshot = Arc::default();
        let persistence_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let actor = Self {
            failed: invalid_policy,
            admission_policy,
            admission_failures: admission_failures.clone(),
            replies: BTreeMap::new(),
            audit_next_tick: audit_next_tick_tx,
            final_snapshot: Arc::clone(&final_snapshot),
            persistence_failed: persistence_failed.clone(),
            checkpoints: checkpoints.clone(),
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
            telemetry: telemetry.map(|consumer| (out_tx.subscribe(), consumer)),
        };
        tokio::spawn(actor.run());
        ZoneHandle {
            phase4_durable,
            phase2,
            phase3,
            admission_policy,
            admission_failures,
            audit_seed,
            audit_next_tick: audit_next_tick_rx,
            final_snapshot,
            persistence_failed,
            checkpoints,
            commands: cmd_tx,
            snapshots: snap_tx,
            ticks: out_tx,
            stats: stats_rx,
            paused,
        }
    }

    async fn run(mut self) {
        if self.failed {
            tracing::error!("invalid byte admission policy fences zone before simulation");
            self.persistence_failed
                .store(true, std::sync::atomic::Ordering::Release);
            return;
        }
        loop {
            tokio::select! {
                biased;
                Some(reply) = self.snapshots.recv() => {
                    self.waiting_snapshots.push(reply);
                    self.answer_snapshots().await;
                },
                go = self.ticks.next_tick() => {
                    if !go {
                        break;
                    }
                    let outcome = self.tick().await;
                    self.ticks.tick_done(outcome);
                    if self.failed { break; }
                    self.answer_snapshots().await;
                    if self.commands_closed && self.pending.is_empty() && self.unrecorded.is_none() {
                        break;
                    }
                },
            }
        }
        if self.failed {
            self.persistence_failed
                .store(true, std::sync::atomic::Ordering::Release);
            self.fail_replies();
        }
        if self.unrecorded.is_none() && !self.failed {
            let mut snapshot = self.state.snapshot();
            if let Some(service) = self.checkpoints.service() {
                service.lock().await.snapshot(&mut snapshot);
            }
            *self.final_snapshot.write() = Some(snapshot);
        }
        tracing::debug!(zone = self.state.seed().zone.0, "zone actor stopped");
    }

    /// Snapshots are only taken at a tick boundary with nothing deferred and nothing
    /// unrecorded.
    async fn answer_snapshots(&mut self) {
        if self.pending.is_empty()
            && self.unrecorded.is_none()
            && !self.waiting_snapshots.is_empty()
        {
            let mut snap = self.state.snapshot();
            if let Some(service) = self.checkpoints.service() {
                service.lock().await.snapshot(&mut snap);
            }
            for reply in self.waiting_snapshots.drain(..) {
                let _ = reply.send(snap.clone());
            }
        }
    }

    #[allow(clippy::too_many_lines)] // Keep durability, persistence and release in one ordered barrier.
    async fn tick(&mut self) -> TickOutcome {
        // Metrics only (see module docs).
        let started = Instant::now();
        self.collect();
        let (record, applied) = match self.unrecorded.take() {
            Some(record) => (record, 0),
            None => match self.run_next().await {
                Ok(ran) => ran,
                Err((tick, e)) => {
                    // Unreachable by construction: the draft came from this state just above.
                    tracing::error!(tick = tick.0, error = %e, "draft did not continue the zone");
                    self.failed = true;
                    self.publish_stats(tick, started, 0);
                    return TickOutcome::Held(tick);
                },
            },
        };
        runtime_phase("draft", started);
        let tick = record.tick;
        if record.commands.iter().any(|command| {
            matches!(
                command.command,
                crate::domain::zone::ZoneCommand::Item(_)
                    | crate::domain::zone::ZoneCommand::MaterializePendingItems(_)
            )
        }) || record
            .events
            .iter()
            .any(|event| matches!(event, crate::domain::zone::ZoneEvent::ItemCheckpoint(_)))
        {
            let enabled = match self.checkpoints.service() {
                Some(service) => service.lock().await.items_enabled(),
                None => false,
            };
            if !self.gate.durable() || !enabled {
                tracing::error!("item work requires durable log and typed receipt lane");
                self.failed = true;
                self.fail_replies();
                self.publish_stats(tick, started, 0);
                return TickOutcome::Held(tick);
            }
        }
        let phase_started = Instant::now();
        if record
            .events
            .iter()
            .any(|event| matches!(event, crate::domain::zone::ZoneEvent::SkillLearned { .. }))
        {
            let enabled = match self.checkpoints.service() {
                Some(service) => service.lock().await.learning_enabled(),
                None => false,
            };
            if !self.gate.durable() || !enabled {
                tracing::error!("learning requires durable log and typed receipt lane");
                self.failed = true;
                self.fail_replies();
                self.publish_stats(tick, started, 0);
                return TickOutcome::Held(tick);
            }
        }
        if self.admission_policy.is_some_and(|policy| {
            super::replay_log::AppliedTickRecord::from_applied(self.state.seed().zone, &record)
                .with_output_digests()
                .encoded_len()
                > policy.record_budget
        }) {
            tracing::error!(
                tick = tick.0,
                "digest record exceeds pre-draft byte proof; zone fenced"
            );
            self.failed = true;
            self.fail_replies();
            self.publish_stats(tick, started, 0);
            return TickOutcome::Held(tick);
        }

        if record
            .events
            .iter()
            .any(|event| matches!(event, crate::domain::zone::ZoneEvent::TokensReconciled { .. }))
            && (!self.gate.durable() || self.checkpoints.service().is_none())
        {
            tracing::error!("token reconciliation requires log and checkpoint lanes");
            self.failed = true;
            return TickOutcome::Held(tick);
        }
        runtime_phase("record_budget", phase_started);
        let phase_started = Instant::now();
        if let Some(service) = self.checkpoints.service() {
            service
                .lock()
                .await
                .recording(self.state.seed().zone, record.epoch, record.tick)
                .await;
        }
        runtime_phase("recording_index", phase_started);
        let phase_started = Instant::now();
        if let Err(e) = self.gate.admit(&record).await {
            self.gate_holds = self.gate_holds.saturating_add(1);
            if self.gate.terminal_failure() {
                tracing::error!(tick = tick.0, error = %e, "terminal record failure fences zone before output release");
                self.failed = true;
                self.fail_replies();
            } else {
                tracing::warn!(tick = tick.0, error = %e, "tick held by gate");
            }
            self.unrecorded = Some(record);
            self.publish_stats(tick, started, 0);
            return TickOutcome::Held(tick);
        }
        runtime_phase("durable_gate", phase_started);
        let phase_started = Instant::now();
        if let Some(service) = self.checkpoints.service() {
            let snapshot_started = Instant::now();
            let snapshot = self.state.snapshot();
            runtime_phase("checkpoint_snapshot", snapshot_started);
            if let Err(error) = service.lock().await.admitted(&record, &snapshot).await {
                tracing::error!(%error,"checkpoint failure fences zone before output release");
                self.failed = true;
                self.fail_replies();
                return TickOutcome::Held(tick);
            }
        }
        runtime_phase("checkpoint", phase_started);
        let phase_started = Instant::now();
        self.complete_replies(&record).await;
        if !record.is_idle() {
            // No subscribers is fine: nobody is connected yet.
            let _ = self.out.send(record);
            if let Some((receiver, consumer)) = &mut self.telemetry {
                // Only this actor sends, and we drain before the next send. No lag possible.
                if let Ok(admitted) = receiver.try_recv() {
                    consumer.admitted(&admitted);
                }
            }
        }
        runtime_phase("fanout_telemetry", phase_started);
        self.publish_stats(tick, started, applied);
        TickOutcome::Ran(tick)
    }

    /// Drafts and runs the next tick, consuming its inputs from `pending`.
    #[allow(clippy::too_many_lines)] // Receipt preflight and lifecycle saves must stay before the single draft mutation.
    async fn run_next(&mut self) -> Result<(Arc<AppliedTick>, usize), (Tick, TickError)> {
        let admitted = loop {
            let selected = self.select_admitted();
            if self.preflight_grants(&selected).await? {
                continue;
            }
            if self.preflight_items(&selected).await? {
                continue;
            }
            if self.preflight_item_allocation(&selected).await? {
                continue;
            }
            if self.preflight_learning(&selected).await? {
                continue;
            }
            let transfer = selected.iter().find_map(|i| {
                self.pending.get(*i).and_then(|q| {
                    if let crate::domain::zone::ZoneCommand::ChangeClass {
                        entity,
                        account,
                        request_key,
                        target,
                    } = q.input.command
                    {
                        Some((*i, entity, account, request_key, target))
                    } else {
                        None
                    }
                })
            });
            let Some((index, entity, account, key, target)) = transfer else {
                break selected;
            };
            let live_admitted = self.pending.get(index).is_some_and(|queued| {
                if let CommandSource::Session {
                    entity: source,
                    generation,
                } = queued.input.source
                {
                    source == entity && self.state.class_player(entity, account, generation).is_ok()
                } else {
                    false
                }
            });
            let result = if let Some(service) = self.checkpoints.service() {
                let service = service.lock().await;
                if service.transfer_ready(entity) {
                    let key = super::IdempotencyKey::from_uuid(key);
                    let fingerprint = super::ports::transfer_fingerprint(
                        crate::domain::CharacterId::from_uuid(entity.as_uuid()),
                        target,
                    );
                    match service
                        .mutation_receipt_lookup(account, &key, &fingerprint)
                        .await
                    {
                        Ok(super::ports::MutationReceiptLookup::Unknown) => None,
                        Ok(super::ports::MutationReceiptLookup::Known(result)) => Some(Ok(result)),
                        Ok(super::ports::MutationReceiptLookup::Conflict) => {
                            Some(Err(super::AppError::IdempotencyConflict))
                        },
                        Err(_) => Some(Err(super::AppError::Unavailable(
                            "transfer receipt lookup failed".into(),
                        ))),
                    }
                } else if live_admitted {
                    tracing::error!(%entity, "admitted player checkpoint lane missing or fenced");
                    return Err((self.state.next_tick(), TickError::PersistenceFence));
                } else {
                    Some(Err(super::AppError::Unavailable(
                        "character admission checkpoint unavailable".into(),
                    )))
                }
            } else if live_admitted {
                return Err((self.state.next_tick(), TickError::PersistenceFence));
            } else {
                Some(Err(super::AppError::Unavailable("checkpoint service unavailable".into())))
            };
            if let Some(result) = result {
                if let Some(mut queued) = self.pending.remove(index) {
                    if let Some(reply) = queued.reply.take() {
                        match reply {
                            MutationReply::Transfer(reply) => {
                                let _ = reply.send(result);
                            },
                            other @ (MutationReply::Learn(_) | MutationReply::Grant(_)) => {
                                other.fail(super::AppError::Unavailable(
                                    "invalid RPC envelope".into(),
                                ));
                            },
                        }
                    }
                }
            } else {
                break selected;
            }
        };
        let inputs: Vec<ZoneInput> = admitted
            .iter()
            .filter_map(|i| self.pending.get(*i).map(|q| q.input.clone()))
            .collect();
        let draft = self.state.draft(inputs);
        let tick = draft.tick;
        // Draft order is admitted order, so the n-th command is the n-th admitted input.
        let traces: Vec<(Ordinal, TraceCarrier)> = draft
            .commands
            .iter()
            .zip(&admitted)
            .filter_map(|(c, i)| {
                let trace = self.pending.get(*i)?.trace.clone()?;
                Some((c.ordinal, trace))
            })
            .collect();

        for (command, index) in draft.commands.iter().zip(&admitted) {
            if let Some(reply) = self.pending.get_mut(*index).and_then(|q| q.reply.take()) {
                self.replies.insert(command.ordinal, reply);
            }
        }
        let applied = draft.commands.len();
        let mut index = 0_usize;
        let mut next_admitted = admitted.iter().peekable();
        self.pending.retain(|_| {
            let keep = next_admitted.next_if_eq(&&index).is_none();
            index = index.saturating_add(1);
            keep
        });
        if let Some(service) = self.checkpoints.service() {
            let mut service = service.lock().await;
            for command in &draft.commands {
                if let crate::domain::zone::ZoneCommand::Despawn { entity }
                | crate::domain::zone::ZoneCommand::ReplaceSession { entity, .. } =
                    command.command
                {
                    if let Err(error) = service.flush(entity).await {
                        tracing::error!(%error,"lifecycle checkpoint failure fences zone");
                        self.failed = true;
                        self.fail_replies();
                        return Err((tick, TickError::PersistenceFence));
                    }
                }
            }
        }
        let domain_started = Instant::now();
        let record = self.state.run_tick(draft).map_err(|e| (tick, e))?;
        runtime_phase("domain", domain_started);
        self.audit_next_tick.send_replace(self.state.next_tick());
        trace_applied(&record, &traces);
        Ok((Arc::new(record), applied))
    }

    // Enrich before drafting, then let selection remeasure the complete recorded command.
    async fn preflight_items(&mut self, selected: &[usize]) -> Result<bool, (Tick, TickError)> {
        use super::ports::{ItemMutationOperation, ItemReceiptLookup};
        use crate::domain::zone::{CombatRole, ZoneCommand};
        let fence = (self.state.next_tick(), TickError::PersistenceFence);
        let mut changed = false;
        for index in selected {
            let queued = self.pending.get(*index).ok_or(fence)?;
            let ZoneCommand::Item(command) = &queued.input.command else {
                continue;
            };
            let CommandSource::Session { entity, generation } = queued.input.source else {
                return Err(fence);
            };
            if entity != command.entity {
                return Err(fence);
            }
            let owner = self
                .state
                .entity(entity)
                .and_then(|entity| entity.combat.as_ref())
                .and_then(|combat| match &combat.role {
                    CombatRole::Player {
                        progression: Some(player),
                        ..
                    } => Some(player.identity.account_id),
                    CombatRole::Player {
                        progression: None, ..
                    }
                    | CombatRole::Npc { .. } => None,
                })
                .ok_or(fence)?;
            let mut normalized = command
                .as_ref()
                .clone()
                .with_account(owner)
                .map_err(|_| fence)?;
            let Some(service) = self.checkpoints.service() else {
                return Err(fence);
            };
            let lane = service.lock().await;
            if !self.gate.durable() || !lane.items_enabled() {
                return Err(fence);
            }
            // Stale generations are recorded refusals and cannot access owner receipts.
            if self.state.class_player(entity, owner, generation).is_ok() {
                if !lane.transfer_ready(entity) {
                    return Err(fence);
                }
                let operation = ItemMutationOperation::from_command_kind(&normalized.kind);
                let key = super::IdempotencyKey::from_uuid(normalized.request_id);
                let fingerprint = normalized.fingerprint_hex().map_err(|_| fence)?;
                normalized = match lane
                    .item_receipt_lookup(owner, operation, &key, &fingerprint)
                    .await
                    .map_err(|_| fence)?
                {
                    ItemReceiptLookup::Unknown => {
                        if normalized.has_known_receipt()
                            || normalized.receipt_conflict().map_err(|_| fence)?.is_some()
                        {
                            return Err(fence);
                        }
                        normalized
                    },
                    ItemReceiptLookup::Conflict { stored_fingerprint } => normalized
                        .with_receipt_conflict(stored_fingerprint)
                        .map_err(|_| fence)?,
                    ItemReceiptLookup::Known(receipt) => {
                        receipt.validate().map_err(|_| fence)?;
                        if receipt.account_id != owner
                            || receipt.operation != operation
                            || receipt.key != key.as_uuid()
                            || receipt.character_id.as_uuid() != entity.as_uuid()
                            || receipt.fingerprint != fingerprint
                        {
                            return Err(fence);
                        }
                        normalized
                            .with_known_receipt(
                                receipt.fingerprint,
                                receipt.result.payload().map_err(|_| fence)?,
                            )
                            .map_err(|_| fence)?
                    },
                };
            }
            drop(lane);
            if normalized != **command {
                self.pending.get_mut(*index).ok_or(fence)?.input.command =
                    ZoneCommand::Item(Box::new(normalized));
                changed = true;
            }
        }
        Ok(changed)
    }

    // Recheck account-wide receipt before any generation or eligibility check. One purchase
    // ends selection, so draining a known duplicate cannot reorder another mutation.
    async fn preflight_learning(&mut self, selected: &[usize]) -> Result<bool, (Tick, TickError)> {
        let Some((index, entity, account, request_key, skill)) =
            selected.iter().find_map(|index| {
                let queued = self.pending.get(*index)?;
                if let crate::domain::zone::ZoneCommand::LearnSkill {
                    entity,
                    account,
                    request_key,
                    skill_id,
                    skill_level,
                } = queued.input.command
                {
                    Some((
                        *index,
                        entity,
                        account,
                        request_key,
                        crate::domain::zone::skill::SkillRef {
                            id: skill_id,
                            rank: skill_level,
                        },
                    ))
                } else {
                    None
                }
            })
        else {
            return Ok(false);
        };
        let Some(service) = self.checkpoints.service() else {
            return Err((self.state.next_tick(), TickError::PersistenceFence));
        };
        let lane = service.lock().await;
        if !self.gate.durable() || !lane.learning_enabled() {
            return Err((self.state.next_tick(), TickError::PersistenceFence));
        }
        let key = super::IdempotencyKey::from_uuid(request_key);
        let fp = super::ports::learn_skill_fingerprint(
            crate::domain::CharacterId::from_uuid(entity.as_uuid()),
            skill,
        );
        let result = match lane.learn_receipt_lookup(account, &key, &fp).await {
            Ok(super::ports::LearnReceiptLookup::Known(result)) => Some(Ok(result)),
            Ok(super::ports::LearnReceiptLookup::Conflict) => {
                Some(Err(super::AppError::IdempotencyConflict))
            },
            Err(_) => {
                Some(Err(super::AppError::Unavailable("learning receipt lookup failed".into())))
            },
            Ok(super::ports::LearnReceiptLookup::Unknown) => {
                let validation = self
                    .pending
                    .get(index)
                    .ok_or((self.state.next_tick(), TickError::PersistenceFence))
                    .map(|queued| match queued.input.source {
                        CommandSource::Session {
                            entity: source,
                            generation,
                        } if source == entity => self
                            .state
                            .validate_learn_skill(entity, account, generation, skill)
                            .map_err(class_error),
                        CommandSource::System | CommandSource::Session { .. } => {
                            Err(super::AppError::PermissionDenied(
                                "learning requires authenticated session".into(),
                            ))
                        },
                    })?;
                if validation.is_ok() && !lane.transfer_ready(entity) {
                    return Err((self.state.next_tick(), TickError::PersistenceFence));
                }
                // Gameplay refusals remain deterministic recorded dispositions.
                None
            },
        };
        drop(lane);
        if let Some(result) = result {
            if let Some(mut queued) = self.pending.remove(index) {
                if let Some(reply) = queued.reply.take() {
                    match reply {
                        MutationReply::Learn(reply) => {
                            let _ = reply.send(result);
                        },
                        other @ (MutationReply::Transfer(_) | MutationReply::Grant(_)) => {
                            other.fail(super::AppError::Unavailable("invalid RPC envelope".into()));
                        },
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Moves queued inputs into `pending`, up to [`PENDING_LIMIT`].
    fn collect(&mut self) {
        // Leave room for the recorded control command; it shares all ordinary byte budgets.
        let limit = if self.item_range_exhausted() {
            PENDING_LIMIT.saturating_sub(1)
        } else {
            PENDING_LIMIT
        };
        while self.pending.len() < limit {
            match self.commands.try_recv() {
                Ok(queued) => self.pending.push_back(queued),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.commands_closed = true;
                    break;
                },
            }
        }
    }

    fn item_range_exhausted(&self) -> bool {
        self.state.item_instance_range().is_some_and(|range| {
            let (_, next, end) = range.bounds();
            next == end
        })
    }

    async fn preflight_item_allocation(
        &mut self,
        selected: &[usize],
    ) -> Result<bool, (Tick, TickError)> {
        use crate::domain::zone::{ItemInstanceRange, ZoneCommand};
        if !self.item_range_exhausted()
            || self
                .pending
                .iter()
                .any(|queued| matches!(queued.input.command, ZoneCommand::ExtendItemRange { .. }))
        {
            return Ok(false);
        }
        let fence = (self.state.next_tick(), TickError::PersistenceFence);
        let mut required = false;
        for index in selected {
            let queued = self.pending.get(*index).ok_or(fence)?;
            if let ZoneCommand::MaterializePendingItems(grant) = &queued.input.command {
                required |= grant.known.is_none() && grant.conflict.is_none();
            } else if let ZoneCommand::SpawnPlayer {
                load: Some(load), ..
            } = &queued.input.command
            {
                required |= load
                    .phase4
                    .as_ref()
                    .is_some_and(|items| !items.pending.is_empty());
            } else if let ZoneCommand::Item(command) = &queued.input.command {
                let CommandSource::Session { entity, generation } = queued.input.source else {
                    return Err(fence);
                };
                let account = command.authenticated_account().map_err(|_| fence)?;
                required |= self.state.class_player(entity, account, generation).is_ok()
                    && command.known_result().map_err(|_| fence)?.is_none()
                    && command.receipt_conflict().map_err(|_| fence)?.is_none()
                    && matches!(
                        command.kind,
                        crate::domain::zone::ItemCommandKind::Enchant { .. }
                            | crate::domain::zone::ItemCommandKind::Crystallize { .. }
                            | crate::domain::zone::ItemCommandKind::ExchangeSpecialAbility { .. }
                    );
            }
        }
        if !required {
            return Ok(false);
        }
        if !self.gate.durable() || self.pending.len() >= PENDING_LIMIT {
            return Err(fence);
        }
        let old = self.state.item_instance_range().ok_or(fence)?;
        let service = self.checkpoints.service().ok_or(fence)?;
        let lane = service.lock().await;
        if !lane.items_enabled() {
            return Err(fence);
        }
        let reserved = lane
            .extend_item_range(self.state.seed(), old.bounds().0)
            .await
            .map_err(|_| fence)?;
        reserved.validate().map_err(|_| fence)?;
        if reserved.zone_id != self.state.seed().zone
            || reserved.epoch != self.state.seed().epoch
            || reserved.next_reserved != reserved.range_start
        {
            return Err(fence);
        }
        let range = ItemInstanceRange::new(
            reserved.range_start,
            reserved.next_reserved,
            reserved.range_end_exclusive,
        )
        .map_err(|_| fence)?;
        let mut checked = old;
        checked.extend(range).map_err(|_| fence)?;
        drop(lane);
        self.pending.push_front(Queued {
            input: ZoneInput::system(ZoneCommand::ExtendItemRange { range }),
            trace: None,
            reply: None,
        });
        Ok(true)
    }

    /// Indices into `pending`, ascending, of the inputs that fit this tick's budgets.
    fn select_admitted(&mut self) -> Vec<usize> {
        let mut recipients: BTreeSet<EntityId> = if self.admission_policy.is_some() {
            self.state
                .entities()
                .filter(|e| e.kind == crate::domain::zone::EntityKind::Player)
                .map(|e| e.id)
                .collect()
        } else {
            BTreeSet::new()
        };
        let initial_recipients = recipients.clone();
        let mut sizes = Vec::new();
        let mut terminal = None;
        let mut per_session: BTreeMap<EntityId, usize> = BTreeMap::new();
        let mut admitted = Vec::new();
        let mut touched = BTreeSet::new();
        let checkpoints = self.checkpoints.service().is_some();
        for (i, queued) in self.pending.iter().enumerate() {
            let transfer = matches!(
                queued.input.command,
                crate::domain::zone::ZoneCommand::ChangeClass { .. }
                    | crate::domain::zone::ZoneCommand::LearnSkill { .. }
            );
            if transfer
                && queued
                    .input
                    .command
                    .entity()
                    .is_some_and(|e| touched.contains(&e))
            {
                break;
            }
            // A preceding Respawn/Spawn/intent must reach an admitted boundary before the
            // same player's final save. Otherwise Respawn + disconnect in one draft could
            // remove the entity before its progression delta is emitted.
            if checkpoints {
                if let crate::domain::zone::ZoneCommand::SpawnPlayer { entity, .. }
                | crate::domain::zone::ZoneCommand::Despawn { entity }
                | crate::domain::zone::ZoneCommand::ReplaceSession { entity, .. } =
                    queued.input.command
                {
                    if touched.contains(&entity) {
                        break;
                    }
                }
            }
            if let CommandSource::Session { entity, .. } = queued.input.source {
                let used = per_session.entry(entity).or_default();
                if *used >= SESSION_COMMANDS_PER_TICK {
                    if transfer || self.admission_policy.is_some() {
                        break;
                    }
                    continue;
                }
                *used = used.saturating_add(1);
            }
            if let Some(policy) = self.admission_policy {
                let mut candidate_recipients = recipients.clone();
                reserve_recipients(&mut candidate_recipients, &queued.input);
                let reason = match policy.measure(&queued.input) {
                    Err(reason) => Some(reason),
                    Ok(size) => {
                        let mut alone_recipients = initial_recipients.clone();
                        reserve_recipients(&mut alone_recipients, &queued.input);
                        if alone_recipients.len() > policy.max_observers {
                            Some(AdmissionFailureReason::TooManyRecipients)
                        } else if !policy.fits(&[size], alone_recipients.len()) {
                            Some(AdmissionFailureReason::RecordTooLarge)
                        } else {
                            sizes.push(size);
                            if !policy.fits(&sizes, candidate_recipients.len()) {
                                break;
                            }
                            recipients = candidate_recipients;
                            None
                        }
                    },
                };
                if let Some(reason) = reason {
                    if admitted.is_empty() {
                        terminal = Some((i, policy, reason));
                    }
                    break;
                }
            }
            if let Some(entity) = queued.input.command.entity() {
                touched.insert(entity);
            }
            admitted.push(i);
            if transfer {
                break;
            }
        }
        if let Some((index, policy, reason)) = terminal {
            if let Some(mut queued) = self.pending.remove(index) {
                let notice = failure(&queued.input, policy, reason);
                log_admission_failure(&queued.input, &notice);
                let _ = self.admission_failures.send(notice);
                if let Some(reply) = queued.reply.take() {
                    reply.fail(super::AppError::ResourceExhausted("ADMISSION_TOO_LARGE".into()));
                }
            }
        }
        admitted
    }

    fn fail_replies(&mut self) {
        for queued in &mut self.pending {
            if let Some(reply) = queued.reply.take() {
                reply.fail(super::AppError::Unavailable(
                    "checkpoint fenced; retry the same key after recovery".into(),
                ));
            }
        }
        for (_, reply) in std::mem::take(&mut self.replies) {
            reply.fail(super::AppError::Unavailable(
                "checkpoint fenced; retry the same key after recovery".into(),
            ));
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)] // Search only this private success fact; other facts do not settle RPC replies.
    async fn complete_replies(&mut self, record: &AppliedTick) {
        for command in &record.commands {
            let Some(reply) = self.replies.remove(&command.ordinal) else {
                continue;
            };
            if let MutationReply::Grant(reply) = reply {
                let _ = reply.send(self.committed_grant(record, command).await);
                continue;
            }
            if let MutationReply::Learn(reply) = reply {
                let result = if let Some(d) = record
                    .dispositions
                    .iter()
                    .find(|d| d.ordinal == command.ordinal)
                {
                    Err(class_error(d.reason))
                } else if let crate::domain::zone::ZoneCommand::LearnSkill {
                    entity,
                    account,
                    request_key,
                    ..
                } = command.command
                {
                    record
                        .events
                        .iter()
                        .find_map(|event| match event {
                            crate::domain::zone::ZoneEvent::SkillLearned {
                                entity: who,
                                account: owner,
                                request_key: key,
                                result,
                                ..
                            } if *who == entity && *owner == account && *key == request_key => {
                                Some(Ok(result.as_ref().clone()))
                            },
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            Err(super::AppError::Unavailable(
                                "committed learning fact missing".into(),
                            ))
                        })
                } else {
                    Err(super::AppError::Unavailable("invalid RPC envelope".into()))
                };
                let _ = reply.send(result);
                continue;
            }
            let MutationReply::Transfer(reply) = reply else {
                continue;
            };
            let result = if let Some(d) = record
                .dispositions
                .iter()
                .find(|d| d.ordinal == command.ordinal)
            {
                Err(class_error(d.reason))
            } else if let crate::domain::zone::ZoneCommand::ChangeClass {
                entity,
                account,
                request_key,
                ..
            } = command.command
            {
                let generation = match command.source {
                    CommandSource::Session { generation, .. } => generation,
                    CommandSource::System => crate::domain::zone::SessionGeneration(0),
                };
                self.state
                    .class_player(entity, account, generation)
                    .map_err(class_error)
                    .and_then(|p| {
                        p.class_state
                            .receipt(request_key)
                            .map(|r| r.result.clone())
                            .ok_or_else(|| {
                                super::AppError::Unavailable(
                                    "successful transfer receipt missing".into(),
                                )
                            })
                    })
            } else {
                Err(super::AppError::Unavailable("invalid RPC envelope".into()))
            };
            let _ = reply.send(result);
        }
    }

    fn publish_stats(&mut self, tick: Tick, started: Instant, applied: usize) {
        let duration = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let stats = TickStats {
            tick,
            duration_micros: duration,
            entities: self.state.entity_count(),
            commands_applied: applied,
            commands_deferred: self.pending.len(),
            gate_holds: self.gate_holds,
        };
        self.stats.send_replace(stats);
        if let Some((_, consumer)) = &mut self.telemetry {
            consumer.stats(stats);
        }
    }
}

/// One `zone.apply` span per traced command, parented on the span it was sent from, with the
/// tick and whether it was accepted. The sending session continues the same trace when it
/// delivers the response (`ws.deliver`).
fn trace_applied(record: &AppliedTick, traces: &[(Ordinal, TraceCarrier)]) {
    for (ordinal, trace) in traces {
        let accepted = !record.dispositions.iter().any(|d| d.ordinal == *ordinal);
        let span = tracing::info_span!(
            parent: trace.span(),
            "zone.apply",
            tick = record.tick.0,
            ordinal = ordinal.0,
            accepted,
        );
        span.in_scope(|| tracing::debug!("zone command applied"));
    }
}

#[cfg(test)]
#[path = "zone_actor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "zone_actor_allocator_tests.rs"]
mod allocator_tests;

/// Maps deterministic refusals at the application boundary.
pub(crate) fn class_error(reason: crate::domain::zone::RejectReason) -> super::AppError {
    use crate::domain::zone::RejectReason;
    match reason {
        RejectReason::TransferConflict | RejectReason::ItemOperationConflict => {
            super::AppError::IdempotencyConflict
        },
        RejectReason::NotPermitted => super::AppError::PermissionDenied(reason.detail().to_owned()),
        RejectReason::ItemNotOwned
        | RejectReason::ItemNotUsable
        | RejectReason::InvalidEquipSlot
        | RejectReason::InsufficientItems
        | RejectReason::InventoryFull
        | RejectReason::ItemGradeMismatch
        | RejectReason::ItemReuse
        | RejectReason::InvalidEnchant
        | RejectReason::InvalidRefinement
        | RejectReason::InvalidRecipe
        | RejectReason::ItemCapabilityDeferred
        | RejectReason::CrystallizeLevelTooLow
        | RejectReason::SkillNotLearnable
        | RejectReason::SkillWrongRank
        | RejectReason::SkillLevelRequired
        | RejectReason::SkillItemRequired
        | RejectReason::SkillDeferred
        | RejectReason::InsufficientSp
        | RejectReason::TransferIneligible
        | RejectReason::TransferRequirement
        | RejectReason::InCombat
        | RejectReason::ClassMasterTooFar
        | RejectReason::DeadActor
        | RejectReason::NonAttackableTarget
        | RejectReason::TargetNotInAoi
        | RejectReason::OutOfRange
        | RejectReason::Protected
        | RejectReason::NotYetImplemented
        | RejectReason::UnknownEntity
        | RejectReason::OutOfBounds
        | RejectReason::TooFar
        | RejectReason::AlreadyExists
        | RejectReason::StaleSession
        | RejectReason::NotAPlayer
        | RejectReason::InvalidLoad
        | RejectReason::SkillUnknown
        | RejectReason::SkillNotLearned
        | RejectReason::SkillCooldown
        | RejectReason::InsufficientMp
        | RejectReason::InsufficientHp
        | RejectReason::AlreadyCasting
        | RejectReason::Incapacitated
        | RejectReason::Silenced
        | RejectReason::InvalidTargetType
        | RejectReason::PeaceZone
        | RejectReason::WeaponCondition
        | RejectReason::NoResurrectOffer
        | RejectReason::NotDead => super::AppError::FailedPrecondition(reason.detail().to_owned()),
    }
}

#[cfg(test)]
#[path = "zone_transfer_tests.rs"]
mod transfer_tests;

fn log_admission_failure(input: &ZoneInput, failure: &AdmissionFailure) {
    use sha2::{Digest, Sha256};
    // serde's typed input has deterministic field/map order. Only its hash reaches logs.
    let hash = serde_json::to_vec(input)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .ok();
    tracing::warn!(entity = ?failure.entity, generation = ?failure.generation,
        seq = ?failure.seq, policy_version = failure.policy_version,
        canonical_hash = ?hash, normalized_bytes = failure.normalized_bytes,
        reason = ?failure.reason, "ADMISSION_TOO_LARGE");
}

fn observer_count(state: &ZoneState) -> usize {
    state
        .snapshot()
        .entities
        .iter()
        .filter(|entity| entity.kind == crate::domain::zone::EntityKind::Player)
        .count()
}

/// Opt-in runtime timings; never used by simulation or admission decisions.
pub(crate) fn runtime_phase(phase: &'static str, started: Instant) {
    tracing::debug!(target: "nightfall_api::runtime_timing", phase, micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX), "runtime phase");
}
