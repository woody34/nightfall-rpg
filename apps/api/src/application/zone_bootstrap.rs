//! `ZoneBootstrap` (Story 3.4): starts a zone in a new epoch and closes the epoch on shutdown.
//!
//! One epoch per zone run (plan §8 #5): every start is a new epoch, numbered one past the
//! highest epoch the log or the snapshot index knows. Starting writes the epoch-start snapshot
//! to the log (and its row to `zone_snapshots`) before the actor exists, so the snapshot
//! always precedes the first applied record. The zone's starting content (NPCs) enters through
//! `SpawnNpc` commands on the first tick, so it is in the applied log like every other change.
//! [`RunningZone::shutdown`] stops the actor and writes the completion watermark.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::ports::Clock;
use super::replay_log::{
    encode_snapshot, start_epoch, DurableTickGate, EpochProgress, EventLog, GateConfig,
    ReplayLogMetrics, Watermark, WatermarkReason, ZoneSnapshotRow, ZoneSnapshotStore,
};
use super::zone_actor::{TickOutcome, TickSource, ZoneActor, ZoneHandle};
use crate::domain::zone::{
    Speed, Vec2Fixed, ZoneBounds, ZoneCommand, ZoneId, ZoneInput, ZoneSeed, ZoneState,
};

/// How long shutdown waits for the actor to finish its current tick.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// An NPC present when the zone starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcSpawn {
    /// Display name.
    pub name: String,
    /// Where it stands; inside the zone bounds.
    pub pos: Vec2Fixed,
    /// Movement speed.
    pub speed: Speed,
}

/// A zone's starting state, loaded from `packages/data/zones/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneDefinition {
    /// The zone.
    pub zone: ZoneId,
    /// Display name.
    pub name: String,
    /// Walkable area.
    pub bounds: ZoneBounds,
    /// NPCs spawned on the first tick, in this order.
    pub npcs: Vec<NpcSpawn>,
    /// Hash of the definition's source, recorded in every snapshot's provenance.
    pub config_hash: String,
}

/// Starts zones. Holds the ports every epoch needs.
pub struct ZoneBootstrap {
    log: Arc<dyn EventLog>,
    snapshots: Option<Arc<dyn ZoneSnapshotStore>>,
    clock: Arc<dyn Clock>,
    metrics: Arc<dyn ReplayLogMetrics>,
    gate: GateConfig,
}

impl ZoneBootstrap {
    /// `snapshots` is `None` when there is no database (dev only).
    #[must_use]
    pub fn new(
        log: Arc<dyn EventLog>,
        snapshots: Option<Arc<dyn ZoneSnapshotStore>>,
        clock: Arc<dyn Clock>,
        metrics: Arc<dyn ReplayLogMetrics>,
    ) -> Self {
        Self {
            log,
            snapshots,
            clock,
            metrics,
            gate: GateConfig::default(),
        }
    }

    /// Overrides the gate's retry policy.
    #[must_use]
    pub const fn with_gate_config(mut self, gate: GateConfig) -> Self {
        self.gate = gate;
        self
    }

    /// Starts `def` in a new epoch, ticking on `ticks`.
    pub async fn start<T: TickSource>(
        &self,
        def: &ZoneDefinition,
        ticks: T,
    ) -> anyhow::Result<RunningZone> {
        let epoch = self.next_epoch(def.zone).await?;
        let time_origin_ms = self.clock.now().timestamp_millis();
        let state = ZoneState::new(
            ZoneSeed {
                zone: def.zone,
                epoch,
            },
            def.bounds,
            time_origin_ms,
        );
        let mut snapshot = state.snapshot();
        snapshot.meta.config_hash.clone_from(&def.config_hash);

        // Snapshot first: no gate (and so no applied record) exists without this proof.
        let started = start_epoch(self.log.as_ref(), &snapshot)
            .await
            .with_context(|| format!("write snapshot of zone {} epoch {epoch}", def.zone.0))?;
        if let Some(store) = &self.snapshots {
            store
                .insert(&ZoneSnapshotRow {
                    zone: def.zone,
                    epoch,
                    snapshot: encode_snapshot(&snapshot)?,
                    snapshot_seq: started.snapshot_seq(),
                    first_seq: None,
                    time_origin_ms,
                    build_id: snapshot.meta.build_id.clone(),
                    config_hash: snapshot.meta.config_hash.clone(),
                    schema_version: snapshot.meta.schema_version,
                })
                .await
                .context("insert zone_snapshots row")?;
        }

        let stop = CancellationToken::new();
        let gate = DurableTickGate::new(
            &started,
            self.log.clone(),
            self.metrics.clone(),
            self.gate,
            stop.clone(),
        );
        let progress = gate.progress();
        let handle = ZoneActor::spawn_gated(
            state,
            StoppableTicks {
                inner: ticks,
                stop: stop.clone(),
            },
            gate,
        );
        for npc in &def.npcs {
            handle
                .send(ZoneInput::system(ZoneCommand::SpawnNpc {
                    name: npc.name.clone(),
                    pos: npc.pos,
                    speed: npc.speed,
                }))
                .map_err(|e| anyhow::anyhow!("queue starting NPC {}: {e}", npc.name))?;
        }
        if let Some(store) = &self.snapshots {
            tokio::spawn(record_first_seq(
                store.clone(),
                def.zone,
                epoch,
                progress.clone(),
                stop.clone(),
            ));
        }
        tracing::info!(zone = def.zone.0, epoch, time_origin_ms, "zone epoch started");
        Ok(RunningZone {
            handle,
            zone: def.zone,
            epoch,
            progress,
            stop,
            log: self.log.clone(),
        })
    }

    async fn next_epoch(&self, zone: ZoneId) -> anyhow::Result<u64> {
        let in_log = self.log.latest_epoch(zone).await?;
        let in_store = match &self.snapshots {
            Some(s) => s.latest_epoch(zone).await?,
            None => None,
        };
        let latest = in_log.max(in_store).unwrap_or(0);
        latest
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("zone {} has run out of epochs", zone.0))
    }
}

/// Fills `zone_snapshots.jetstream_first_seq` once the first record is acknowledged. Off the
/// tick path; a failure only costs the index entry (replay can start from the snapshot seq).
async fn record_first_seq(
    store: Arc<dyn ZoneSnapshotStore>,
    zone: ZoneId,
    epoch: u64,
    mut progress: watch::Receiver<EpochProgress>,
    stop: CancellationToken,
) {
    let first = tokio::select! {
        p = progress.wait_for(|p| p.first_seq.is_some()) => p.ok().and_then(|p| p.first_seq),
        () = stop.cancelled() => None,
    };
    if let Some(seq) = first {
        if let Err(e) = store.set_first_seq(zone, epoch, seq).await {
            tracing::warn!(zone = zone.0, epoch, error = %e, "cannot record first log seq");
        }
    }
}

/// A zone running in an epoch.
pub struct RunningZone {
    handle: ZoneHandle,
    zone: ZoneId,
    epoch: u64,
    progress: watch::Receiver<EpochProgress>,
    stop: CancellationToken,
    log: Arc<dyn EventLog>,
}

impl RunningZone {
    /// The zone's handle, for sessions.
    #[must_use]
    pub const fn handle(&self) -> &ZoneHandle {
        &self.handle
    }

    /// The zone.
    #[must_use]
    pub const fn zone(&self) -> ZoneId {
        self.zone
    }

    /// The epoch.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// What has been durably logged so far.
    #[must_use]
    pub fn progress(&self) -> watch::Receiver<EpochProgress> {
        self.progress.clone()
    }

    /// Stops the actor after its current tick and writes the completion watermark naming the
    /// last acknowledged tick. If the actor cannot be stopped or the watermark cannot be
    /// written, the epoch stays `Incomplete`, which replay refuses: that is the safe outcome.
    pub async fn shutdown(self, reason: WatermarkReason) -> anyhow::Result<Watermark> {
        self.stop.cancel();
        tokio::time::timeout(STOP_TIMEOUT, self.handle.stopped())
            .await
            .map_err(|_| anyhow::anyhow!("zone {} did not stop in time", self.zone.0))?;
        let progress = *self.progress.borrow();
        let watermark = Watermark {
            zone: self.zone,
            epoch: self.epoch,
            last_tick: progress.last_tick,
            records: progress.records,
            reason,
        };
        self.log
            .write_watermark(&watermark)
            .await
            .context("write completion watermark")?;
        tracing::info!(
            zone = self.zone.0,
            epoch = self.epoch,
            last_tick = ?watermark.last_tick,
            "zone epoch closed"
        );
        Ok(watermark)
    }
}

/// Ends the tick source when `stop` is cancelled.
struct StoppableTicks<T> {
    inner: T,
    stop: CancellationToken,
}

impl<T: TickSource> TickSource for StoppableTicks<T> {
    fn next_tick(&mut self) -> impl Future<Output = bool> + Send {
        let stop = self.stop.clone();
        let next = self.inner.next_tick();
        async move {
            tokio::select! {
                biased;
                () = stop.cancelled() => false,
                go = next => go,
            }
        }
    }

    fn tick_done(&mut self, outcome: TickOutcome) {
        self.inner.tick_done(outcome);
    }
}
