//! The session actor (Story 4.1): one task per WebSocket that turns client frames into zone
//! inputs and the zone's per-player output into server frames.
//!
//! ```text
//!  socket ──FrameSource──▶ session actor ──ZoneHandle::send_traced──▶ zone actor
//!                            │      ▲                                     │
//!                            │      └──────── broadcast Arc<AppliedTick> ─┘
//!                            ▼
//!                 mpsc(256) outbound ──▶ writer task ──FrameSink──▶ socket
//! ```
//!
//! What the actor owns and enforces (plan §8 items 6, 8, 12; world.proto header):
//!
//! * **Admission.** Before reading a frame it registers with the [`SessionRegistry`], which
//!   queues `SpawnPlayer` (first session of the character) or `ReplaceSession` (a newer
//!   generation takes over a live one, whose socket is then closed with 4409). The session
//!   forwards nothing until it sees its own admission command applied in an `AppliedTick`, so
//!   it never forwards output that belonged to the session it replaced.
//! * **Inbound limits.** Every frame is audited, then: `seq` must strictly increase (else
//!   close 4400); a token bucket of 30 frames/s answers the excess with `RATE_LIMITED`; frames
//!   over 4096 bytes are refused with `INVALID` before decoding; undecodable or non-binary
//!   frames are `INVALID`; a full zone queue is `OVERLOADED`. 60 s without any inbound frame
//!   closes with 4408.
//! * **Ordered output.** For each tick the session sends exactly the zone's ordered output for
//!   its player, encoded one `ServerMessage` per frame, never reordered or merged. Its own
//!   refusals (above) are the only other frames, sent when the frame is refused.
//! * **Backpressure.** The outbound queue holds 256 frames. A queue that stays full for a whole
//!   tick means the client is not reading: the frame is dropped, counted in
//!   `ws_dropped_frames_total`, and the socket is closed with 4429 so the client reconnects and
//!   resynchronises. (Within that tick the session waits: joining a crowded AOI alone yields
//!   more than 256 frames at once.) A session that falls a
//!   whole broadcast buffer behind the zone is closed the same way.
//! * **Leaving.** On any exit the session queues `Despawn` from its own source and generation;
//!   the zone refuses it if a newer session owns the entity, so a stale socket can never remove
//!   its replacement.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use super::ports::SessionAudit;
use super::trace::TraceCarrier;
use super::zone_actor::{ActorStopped, ZoneHandle, ZoneSendError};
use crate::domain::zone::{
    AppliedTick, CommandSource, EntityId, ObserverOutput, PlayerLoad, SessionGeneration, Speed,
    Vec2Fixed, ZoneCommand, ZoneInput, TICK_MS,
};
use crate::domain::SessionId;

/// Largest inbound frame, in bytes (world.proto header). Larger frames are refused before
/// they are decoded.
pub const MAX_FRAME_BYTES: usize = 4096;

/// Inbound frames per second per session (world.proto header), also the burst size.
pub const FRAMES_PER_SECOND: u32 = 30;

/// A session with no inbound frame for this long is closed with 4408.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Outbound frames a session may have queued for its socket. 256 frames is about 2.5 s of a
/// busy AOI at 10 Hz; a client further behind than that is not reading.
pub const OUTBOUND_QUEUE: usize = 256;

/// Concurrent sessions (and upgrades in progress) from one IP address.
pub const MAX_SESSIONS_PER_IP: usize = 10;

/// Most frames the writer hands the socket per flush.
const WRITE_BATCH: usize = 64;

/// Intents a session keeps trace context for while waiting for their response. Responses
/// arrive within a tick or two, so this only bounds a misbehaving zone.
const TRACES_IN_FLIGHT: usize = 64;

/// Per-session resource limits. [`SessionLimits::default`] is the contract in world.proto;
/// tests shorten the timeouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    /// Largest accepted inbound frame.
    pub max_frame_bytes: usize,
    /// Token-bucket rate and burst, in frames per second.
    pub frames_per_second: u32,
    /// Close a session after this long without an inbound frame.
    pub idle_timeout: Duration,
    /// Outbound queue capacity, in frames.
    pub outbound_queue: usize,
    /// How long a full outbound queue may wait for the writer before the client counts as
    /// slow (4429). One tick: joining a crowded AOI produces more than a queue of frames in
    /// one tick, which a reading client drains in microseconds.
    pub outbound_grace: Duration,
    /// Concurrent sessions per client IP (enforced by the `/ws` handler before the upgrade).
    pub max_sessions_per_ip: usize,
    /// How long the writer may take to send the close frame before the socket is dropped.
    pub close_timeout: Duration,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: MAX_FRAME_BYTES,
            frames_per_second: FRAMES_PER_SECOND,
            idle_timeout: IDLE_TIMEOUT,
            outbound_queue: OUTBOUND_QUEUE,
            outbound_grace: Duration::from_millis(TICK_MS),
            max_sessions_per_ip: MAX_SESSIONS_PER_IP,
            close_timeout: Duration::from_secs(1),
        }
    }
}

/// Why a session ended, and the WebSocket close code it is reported with (api-guidelines.md,
/// "Real-time channel"). Codes are only used after the upgrade; everything before it is an
/// HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// The client closed the socket or it broke. Nothing is sent.
    ClientGone,
    /// The server is shutting down: 1001.
    Shutdown,
    /// A `seq` did not increase: 4400.
    SeqRegression,
    /// No inbound frame within the idle timeout: 4408.
    IdleTimeout,
    /// A newer session of the same account took over the character: 4409.
    Replaced,
    /// The client is not reading fast enough (outbound queue full, or a whole broadcast buffer
    /// behind): 4429.
    SlowConsumer,
    /// The zone refused the admission (for example the saved position is outside it): 1011.
    AdmissionRefused,
    /// The zone actor stopped: 1011.
    ZoneUnavailable,
}

impl SessionEnd {
    /// The close code to send, `None` when the socket is already gone.
    #[must_use]
    pub const fn close_code(self) -> Option<u16> {
        match self {
            Self::ClientGone => None,
            Self::Shutdown => Some(1001),
            Self::SeqRegression => Some(4400),
            Self::IdleTimeout => Some(4408),
            Self::Replaced => Some(4409),
            Self::SlowConsumer => Some(4429),
            Self::AdmissionRefused | Self::ZoneUnavailable => Some(1011),
        }
    }

    /// The close reason text. Short, static, safe to show.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::ClientGone => "client gone",
            Self::Shutdown => "server shutting down",
            Self::SeqRegression => "seq must increase",
            Self::IdleTimeout => "idle timeout",
            Self::Replaced => "replaced by a newer session",
            Self::SlowConsumer => "client too slow",
            Self::AdmissionRefused => "admission refused",
            Self::ZoneUnavailable => "zone unavailable",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::ClientGone => "client_gone",
            Self::Shutdown => "shutdown",
            Self::SeqRegression => "seq_regression",
            Self::IdleTimeout => "idle_timeout",
            Self::Replaced => "replaced",
            Self::SlowConsumer => "slow_consumer",
            Self::AdmissionRefused => "admission_refused",
            Self::ZoneUnavailable => "zone_unavailable",
        }
    }
}

/// One frame from the client, as the transport delivers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboundFrame {
    /// A data frame. `binary` is false for text frames, which the protocol does not use.
    Data {
        /// The payload.
        bytes: Bytes,
        /// Binary (true) or text (false).
        binary: bool,
    },
    /// Ping or pong. Counts as activity, carries no intent.
    Control,
    /// The client sent a close frame.
    Close,
}

/// One frame to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundFrame {
    /// An encoded `ServerMessage`.
    Binary(Bytes),
    /// Close the socket with this code and reason.
    Close {
        /// WebSocket close code.
        code: u16,
        /// Close reason.
        reason: &'static str,
    },
}

/// The reading half of a socket.
pub trait FrameSource: Send + 'static {
    /// The next frame; `None` once the socket is closed or broken. Must be cancel-safe: the
    /// actor drops the future when a tick arrives first.
    fn recv(&mut self) -> impl Future<Output = Option<InboundFrame>> + Send;
}

/// The writing half of a socket.
pub trait FrameSink: Send + 'static {
    /// Writes `frames` in order and flushes once: a busy AOI produces hundreds of frames per
    /// tick, and one flush (syscall) each would cap the socket well below that. `Err` means
    /// the socket is gone.
    fn send(
        &mut self,
        frames: Vec<OutboundFrame>,
    ) -> impl Future<Output = Result<(), SinkClosed>> + Send;
}

/// The socket can no longer be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("socket closed")]
pub struct SinkClosed;

/// Reasons the session itself refuses an intent, before it reaches the zone. Mirrors the
/// matching `nightfall.v1.RejectReason` values; reasons decided by the zone travel in its
/// output instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReject {
    /// Malformed: oversize, undecodable, not binary, no intent, missing field.
    Invalid,
    /// A coordinate is not a finite tile position.
    OutOfBounds,
    /// Over the frame rate limit.
    RateLimited,
    /// The zone's queue is full.
    Overloaded,
}

/// A decoded `ClientMessage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedIntent {
    /// `ClientMessage.seq`.
    pub seq: u32,
    /// The zone command it asks for, or why it cannot become one (with a detail for logs).
    pub command: Result<ZoneCommand, (SessionReject, &'static str)>,
}

/// The frame is not a `ClientMessage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("frame is not a ClientMessage")]
pub struct Undecodable;

/// The wire format, supplied by the interface layer (`interface::ws::ProstCodec`), so the
/// session logic stays independent of protobuf.
pub trait SessionCodec: Send + Sync + 'static {
    /// Decodes one inbound frame for the session whose player is `entity`.
    fn decode(&self, entity: EntityId, frame: &[u8]) -> Result<DecodedIntent, Undecodable>;

    /// An `IntentRejected` frame. `seq` is 0 when the refused frame had none.
    fn rejected(&self, seq: u32, reason: SessionReject, detail: &str) -> Bytes;

    /// A player's output for one tick, one encoded `ServerMessage` per frame, in order.
    fn outputs(&self, outputs: &[ObserverOutput], server_time_ms: i64) -> Vec<Bytes>;
}

/// Session instruments (`sessions_active`, `ws_frames_total`, `ws_dropped_frames_total`).
/// Implemented by `infrastructure::telemetry::Metrics`.
pub trait SessionMetrics: Send + Sync {
    /// A session started (+1 active).
    fn session_opened(&self);
    /// A session ended (-1 active).
    fn session_closed(&self);
    /// Frames received from clients.
    fn frames_in(&self, n: u64);
    /// Frames written to clients.
    fn frames_out(&self, n: u64);
    /// Frames dropped because a session's outbound queue was full.
    fn frames_dropped(&self, n: u64);
}

/// What a player spawns with: the loaded character, already in zone units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSpawn {
    /// The character id.
    pub entity: EntityId,
    /// Character name.
    pub name: String,
    /// Saved position.
    pub pos: Vec2Fixed,
    /// Movement speed.
    pub speed: Speed,
    /// Loaded combat state; `None` spawns a noncombat player.
    pub load: Option<Box<PlayerLoad>>,
}

/// The zone generation for an account's session generation. Generations start at 1 and only
/// grow, so the conversion is exact.
#[must_use]
pub fn zone_generation(g: crate::domain::SessionGeneration) -> SessionGeneration {
    SessionGeneration(u64::from(g.get().unsigned_abs()))
}

/// Why the registry refused an admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmitError {
    /// A session with the same or a newer generation already owns the character.
    #[error("a newer session owns this character")]
    Stale,
    /// The zone actor has stopped.
    #[error(transparent)]
    ZoneStopped(#[from] ActorStopped),
}

struct Live {
    session: SessionId,
    generation: SessionGeneration,
    replaced: CancellationToken,
}

/// Which session currently owns each player entity (plan §8 #8). Admission and departure take
/// one lock and queue their zone command while holding it, so the zone receives lifecycle
/// commands in the same order the registry decided them: a departing session's `Despawn` is
/// always queued before a later admission's `SpawnPlayer`, and a replacement's
/// `ReplaceSession` before the stale session's `Despawn`.
#[derive(Clone, Default)]
pub struct SessionRegistry {
    live: Arc<tokio::sync::Mutex<BTreeMap<EntityId, Live>>>,
}

impl SessionRegistry {
    /// Registers `session` as the owner of `spawn.entity` at `generation` and queues its
    /// admission: `SpawnPlayer` if nobody owns the entity, `ReplaceSession` (and the old
    /// session's replacement signal) if an older generation does. Returns the token that is
    /// cancelled when this session is itself replaced.
    pub async fn admit(
        &self,
        zone: &ZoneHandle,
        session: SessionId,
        generation: SessionGeneration,
        spawn: &PlayerSpawn,
    ) -> Result<CancellationToken, AdmitError> {
        let mut live = self.live.lock().await;
        let command = match live.get(&spawn.entity) {
            Some(current) if current.generation >= generation => return Err(AdmitError::Stale),
            Some(current) => {
                current.replaced.cancel();
                ZoneCommand::ReplaceSession {
                    entity: spawn.entity,
                    generation,
                }
            },
            None => ZoneCommand::SpawnPlayer {
                entity: spawn.entity,
                name: spawn.name.clone(),
                pos: spawn.pos,
                speed: spawn.speed,
                generation,
                load: spawn.load.clone(),
            },
        };
        zone.send_wait(ZoneInput::system(command)).await?;
        let replaced = CancellationToken::new();
        live.insert(
            spawn.entity,
            Live {
                session,
                generation,
                replaced: replaced.clone(),
            },
        );
        Ok(replaced)
    }

    /// Unregisters `session` (if it still owns the entity) and queues its `Despawn` from the
    /// session's own source. The zone refuses that despawn when a newer generation owns the
    /// entity, which is the fence that protects a replacement from its predecessor.
    pub async fn leave(
        &self,
        zone: &ZoneHandle,
        session: SessionId,
        entity: EntityId,
        generation: SessionGeneration,
    ) -> Result<(), ActorStopped> {
        let mut live = self.live.lock().await;
        if live.get(&entity).is_some_and(|l| l.session == session) {
            live.remove(&entity);
        }
        zone.send_wait(ZoneInput {
            source: CommandSource::Session { entity, generation },
            seq: None,
            command: ZoneCommand::Despawn { entity },
        })
        .await
    }

    /// Number of entities with a live session.
    pub async fn len(&self) -> usize {
        self.live.lock().await.len()
    }

    /// `true` when no session is live.
    pub async fn is_empty(&self) -> bool {
        self.live.lock().await.is_empty()
    }
}

/// Everything sessions share: the zone, the registry, the ports and the limits.
pub struct SessionContext {
    /// The zone sessions play in.
    pub zone: ZoneHandle,
    /// Owner of each player entity.
    pub registry: SessionRegistry,
    /// Per-session frame audit.
    pub audit: Arc<dyn SessionAudit>,
    /// Instruments.
    pub metrics: Arc<dyn SessionMetrics>,
    /// Wire format.
    pub codec: Arc<dyn SessionCodec>,
    /// Limits.
    pub limits: SessionLimits,
    /// Cancelled at server shutdown: every session closes with 1001.
    pub shutdown: CancellationToken,
}

/// An admitted socket, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStart {
    /// This socket's id (audit key).
    pub id: SessionId,
    /// The account's session generation from the consumed ticket.
    pub generation: SessionGeneration,
    /// The player to spawn.
    pub player: PlayerSpawn,
}

/// Runs one session until it ends and returns why. Spawns the writer task over `sink`.
pub async fn run_session<R: FrameSource, W: FrameSink>(
    ctx: Arc<SessionContext>,
    start: SessionStart,
    source: R,
    sink: W,
) -> SessionEnd {
    let span = tracing::info_span!(
        "ws.session",
        session_id = %start.id,
        character_id = %start.player.entity,
        generation = start.generation.0,
        end = tracing::field::Empty,
    );
    let session_span = span.clone();
    async move {
        ctx.metrics.session_opened();
        let (out_tx, out_rx) = mpsc::channel(ctx.limits.outbound_queue.max(1));
        let (close_tx, close_rx) = oneshot::channel();
        let writer = tokio::spawn(
            write_loop(sink, out_rx, close_rx, ctx.metrics.clone())
                .instrument(tracing::Span::current()),
        );
        // Subscribe before queueing the admission so its tick cannot be missed.
        let ticks = ctx.zone.subscribe();
        let end = match ctx
            .registry
            .admit(&ctx.zone, start.id, start.generation, &start.player)
            .await
        {
            Ok(replaced) => {
                let mut actor = Actor::new(&ctx, &start, out_tx, session_span);
                let end = actor.run(source, ticks, replaced).await;
                if let Err(e) = ctx
                    .registry
                    .leave(&ctx.zone, start.id, start.player.entity, start.generation)
                    .await
                {
                    tracing::warn!(error = %e, "despawn on disconnect not queued");
                }
                end
            },
            Err(AdmitError::Stale) => SessionEnd::Replaced,
            Err(AdmitError::ZoneStopped(_)) => SessionEnd::ZoneUnavailable,
        };
        tracing::Span::current().record("end", end.label());
        tracing::info!(end = end.label(), "session ended");
        // The writer sends the close frame (dropping anything still queued) and stops.
        let _ = close_tx.send(end);
        if tokio::time::timeout(ctx.limits.close_timeout, writer)
            .await
            .is_err()
        {
            tracing::debug!("socket did not take the close frame in time; dropping it");
        }
        ctx.metrics.session_closed();
        end
    }
    .instrument(span)
    .await
}

/// Writes queued frames until told to close, then sends the close frame. The close signal
/// wins over queued frames: on 4429 the queue is full of frames the client will never read.
async fn write_loop<W: FrameSink>(
    mut sink: W,
    mut frames: mpsc::Receiver<Bytes>,
    mut close: oneshot::Receiver<SessionEnd>,
    metrics: Arc<dyn SessionMetrics>,
) {
    let mut batch = Vec::with_capacity(WRITE_BATCH);
    loop {
        tokio::select! {
            biased;
            end = &mut close => {
                if let Ok(end) = end {
                    if let Some(code) = end.close_code() {
                        let reason = end.reason();
                        let _ = sink.send(vec![OutboundFrame::Close { code, reason }]).await;
                    }
                }
                return;
            },
            n = frames.recv_many(&mut batch, WRITE_BATCH) => {
                if n == 0 {
                    return;
                }
                let out = batch.drain(..).map(OutboundFrame::Binary).collect();
                if sink.send(out).await.is_err() {
                    return;
                }
                metrics.frames_out(u64::try_from(n).unwrap_or(u64::MAX));
            },
        }
    }
}

/// Token bucket in milli-frames: `rate` frames per second, burst `rate`. Integer math on
/// tokio's clock, so paused-time tests are exact.
#[derive(Debug)]
struct TokenBucket {
    capacity: u64,
    per_second: u64,
    tokens: u64,
    last: Instant,
}

impl TokenBucket {
    const COST: u64 = 1000;

    fn new(frames_per_second: u32) -> Self {
        let capacity = u64::from(frames_per_second).saturating_mul(Self::COST);
        Self {
            capacity,
            per_second: capacity,
            tokens: capacity,
            last: Instant::now(),
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        let elapsed_ms =
            u64::try_from(now.saturating_duration_since(self.last).as_millis()).unwrap_or(u64::MAX);
        // `per_second` milli-frames per 1000 ms is `per_second / 1000` per ms; multiply first.
        let refill = elapsed_ms.saturating_mul(self.per_second) / 1000;
        if refill > 0 {
            self.tokens = self.tokens.saturating_add(refill).min(self.capacity);
            self.last = now;
        }
        if self.tokens >= Self::COST {
            self.tokens = self.tokens.saturating_sub(Self::COST);
            true
        } else {
            false
        }
    }
}

/// The per-socket state machine, separate from the plumbing in [`run_session`].
struct Actor<'a> {
    ctx: &'a SessionContext,
    id: SessionId,
    entity: EntityId,
    generation: SessionGeneration,
    out: mpsc::Sender<Bytes>,
    bucket: TokenBucket,
    last_seq: Option<u32>,
    /// Whether this session's admission has been applied; nothing is forwarded before.
    admitted: bool,
    /// Trace context of intents awaiting their Ack or rejection, by seq.
    traces: BTreeMap<u32, TraceCarrier>,
    session_span: tracing::Span,
}

impl<'a> Actor<'a> {
    fn new(
        ctx: &'a SessionContext,
        start: &SessionStart,
        out: mpsc::Sender<Bytes>,
        session_span: tracing::Span,
    ) -> Self {
        Self {
            ctx,
            id: start.id,
            entity: start.player.entity,
            generation: start.generation,
            out,
            bucket: TokenBucket::new(ctx.limits.frames_per_second),
            last_seq: None,
            admitted: false,
            traces: BTreeMap::new(),
            session_span,
        }
    }

    async fn run<R: FrameSource>(
        &mut self,
        mut source: R,
        mut ticks: broadcast::Receiver<Arc<AppliedTick>>,
        replaced: CancellationToken,
    ) -> SessionEnd {
        let idle = tokio::time::sleep(self.ctx.limits.idle_timeout);
        tokio::pin!(idle);
        loop {
            let step = tokio::select! {
                biased;
                () = self.ctx.shutdown.cancelled() => Err(SessionEnd::Shutdown),
                () = replaced.cancelled() => Err(SessionEnd::Replaced),
                () = &mut idle => Err(SessionEnd::IdleTimeout),
                tick = ticks.recv() => match tick {
                    Ok(t) => self.on_tick(&t).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed_ticks = n, "session fell behind the zone");
                        Err(SessionEnd::SlowConsumer)
                    },
                    Err(broadcast::error::RecvError::Closed) => Err(SessionEnd::ZoneUnavailable),
                },
                frame = source.recv() => match frame {
                    None | Some(InboundFrame::Close) => Err(SessionEnd::ClientGone),
                    Some(InboundFrame::Control) => {
                        idle.as_mut().reset(self.idle_deadline());
                        Ok(())
                    },
                    Some(InboundFrame::Data { bytes, binary }) => {
                        idle.as_mut().reset(self.idle_deadline());
                        self.on_frame(&bytes, binary).await
                    },
                },
            };
            if let Err(end) = step {
                return end;
            }
        }
    }

    /// When the session times out if nothing else arrives.
    fn idle_deadline(&self) -> Instant {
        let now = Instant::now();
        // Overflow needs a timeout of centuries; treat it as "now" rather than panic.
        now.checked_add(self.ctx.limits.idle_timeout).unwrap_or(now)
    }

    /// One inbound data frame.
    async fn on_frame(&mut self, bytes: &Bytes, binary: bool) -> Result<(), SessionEnd> {
        self.ctx.metrics.frames_in(1);
        let decoded = (binary && bytes.len() <= self.ctx.limits.max_frame_bytes)
            .then(|| self.ctx.codec.decode(self.entity, bytes).ok())
            .flatten();
        let seq = decoded.as_ref().map(|d| d.seq);
        self.ctx.audit.record_in(self.id, seq, bytes);

        if let Some(seq) = seq {
            if self.last_seq.is_some_and(|last| seq <= last) {
                tracing::info!(seq, last = self.last_seq, "seq regression");
                return Err(SessionEnd::SeqRegression);
            }
            self.last_seq = Some(seq);
        }
        let reply_seq = seq.unwrap_or(0);
        if !self.bucket.take(Instant::now()) {
            return self
                .reject(reply_seq, SessionReject::RateLimited, "over 30 frames per second")
                .await;
        }
        if bytes.len() > self.ctx.limits.max_frame_bytes {
            return self
                .reject(0, SessionReject::Invalid, "frame larger than 4096 bytes")
                .await;
        }
        if !binary {
            return self
                .reject(0, SessionReject::Invalid, "frames must be binary")
                .await;
        }
        let Some(intent) = decoded else {
            return self
                .reject(0, SessionReject::Invalid, "frame is not a ClientMessage")
                .await;
        };
        let command = match intent.command {
            Ok(c) => c,
            Err((reason, detail)) => return self.reject(intent.seq, reason, detail).await,
        };
        self.forward(intent.seq, command).await
    }

    /// Sends an intent to the zone inside its own trace (`ws.frame` -> `zone.apply` ->
    /// `ws.deliver`).
    async fn forward(&mut self, seq: u32, command: ZoneCommand) -> Result<(), SessionEnd> {
        let frame_span = tracing::info_span!(parent: None, "ws.frame", session_id = %self.id, seq);
        frame_span.follows_from(&self.session_span);
        let carrier = frame_span.in_scope(TraceCarrier::current);
        let input = ZoneInput::session(self.entity, self.generation, seq, command);
        match self.ctx.zone.send_traced(input, carrier.clone()) {
            Ok(()) => {
                if self.traces.len() >= TRACES_IN_FLIGHT {
                    self.traces.pop_first();
                }
                self.traces.insert(seq, carrier);
                Ok(())
            },
            Err(ZoneSendError::Full(_)) => {
                frame_span.in_scope(|| tracing::warn!(seq, "zone queue full"));
                self.reject(seq, SessionReject::Overloaded, "zone queue is full")
                    .await
            },
            Err(ZoneSendError::Paused(_)) => {
                frame_span.in_scope(|| tracing::warn!(seq, "zone paused"));
                self.reject(seq, SessionReject::Overloaded, "zone is paused")
                    .await
            },
            Err(ZoneSendError::Closed(_)) => Err(SessionEnd::ZoneUnavailable),
        }
    }

    async fn reject(
        &self,
        seq: u32,
        reason: SessionReject,
        detail: &str,
    ) -> Result<(), SessionEnd> {
        let frame = self.ctx.codec.rejected(seq, reason, detail);
        self.enqueue(&frame, self.grace_deadline()).await
    }

    /// One applied tick: forward this player's output once the session is admitted.
    async fn on_tick(&mut self, tick: &AppliedTick) -> Result<(), SessionEnd> {
        if !self.admitted {
            match self.admission_outcome(tick) {
                None => return Ok(()),
                Some(false) => return Err(SessionEnd::AdmissionRefused),
                Some(true) => self.admitted = true,
            }
        } else if self.replaced_in(tick) {
            // Output on this tick already belongs to the replacement.
            return Err(SessionEnd::Replaced);
        }
        let Some(outputs) = tick.outputs.get(&self.entity) else {
            return Ok(());
        };
        self.trace_responses(tick, outputs);
        // One grace period for the whole tick, not per frame.
        let deadline = self.grace_deadline();
        for frame in self.ctx.codec.outputs(outputs, tick.server_time_ms) {
            self.enqueue(&frame, deadline).await?;
        }
        Ok(())
    }

    /// `Some(accepted)` if this tick applied this session's own admission command.
    fn admission_outcome(&self, tick: &AppliedTick) -> Option<bool> {
        let mine = tick.commands.iter().find(|c| {
            c.source == CommandSource::System
                && match c.command {
                    ZoneCommand::SpawnPlayer {
                        entity, generation, ..
                    }
                    | ZoneCommand::ReplaceSession { entity, generation } => {
                        entity == self.entity && generation == self.generation
                    },
                    ZoneCommand::SpawnNpc { .. }
                    | ZoneCommand::Despawn { .. }
                    | ZoneCommand::MoveTo { .. }
                    | ZoneCommand::StopMove { .. }
                    | ZoneCommand::SetTarget { .. }
                    | ZoneCommand::Attack { .. }
                    | ZoneCommand::StopAttack { .. }
                    | ZoneCommand::Respawn { .. }
                    | ZoneCommand::AddAggro { .. } => false,
                }
        })?;
        Some(!tick.dispositions.iter().any(|d| d.ordinal == mine.ordinal))
    }

    /// Whether a newer generation took over this session's entity on `tick`.
    fn replaced_in(&self, tick: &AppliedTick) -> bool {
        tick.commands.iter().any(|c| {
            matches!(
                c.command,
                ZoneCommand::ReplaceSession { entity, generation }
                    if entity == self.entity && generation > self.generation
            ) && !tick.dispositions.iter().any(|d| d.ordinal == c.ordinal)
        })
    }

    /// Closes the trace of every intent answered on this tick with a `ws.deliver` span.
    fn trace_responses(&mut self, tick: &AppliedTick, outputs: &[ObserverOutput]) {
        for o in outputs {
            let seq = match o {
                ObserverOutput::Accepted { seq, .. } => Some(*seq),
                ObserverOutput::Rejected(d) => d.seq,
                ObserverOutput::Event(_) => None,
            };
            let Some(seq) = seq else { continue };
            if let Some(carrier) = self.traces.remove(&seq) {
                let span = tracing::info_span!(
                    parent: carrier.span(),
                    "ws.deliver",
                    seq,
                    tick = tick.tick.0,
                );
                span.in_scope(|| tracing::debug!("intent response queued for the socket"));
            }
            // Anything older was answered on a tick this session did not forward.
            self.traces.retain(|s, _| *s > seq);
        }
    }

    fn grace_deadline(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.ctx.limits.outbound_grace)
            .unwrap_or(now)
    }

    /// Queues one frame for the socket and audits it. A full queue waits for the writer until
    /// `deadline`; past it the frame is dropped and the session ends (4429).
    async fn enqueue(&self, frame: &Bytes, deadline: Instant) -> Result<(), SessionEnd> {
        let permit = match self.out.try_reserve() {
            Ok(p) => p,
            Err(mpsc::error::TrySendError::Closed(())) => return Err(SessionEnd::ClientGone),
            Err(mpsc::error::TrySendError::Full(())) => {
                match tokio::time::timeout_at(deadline, self.out.reserve()).await {
                    Ok(Ok(p)) => p,
                    Ok(Err(_)) => return Err(SessionEnd::ClientGone),
                    Err(_) => {
                        self.ctx.metrics.frames_dropped(1);
                        tracing::warn!("outbound queue full; closing slow client");
                        return Err(SessionEnd::SlowConsumer);
                    },
                }
            },
        };
        permit.send(frame.clone());
        self.ctx.audit.record_out(self.id, frame);
        Ok(())
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
