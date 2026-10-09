//! Live progression projection. Only durably admitted ticks enter this lane; replay verification
//! has no repository. Requests are immutable until acknowledged, including after a lost commit
//! response. The durable applied prefix is the recovery queue.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::time::Instant;
use tokio_stream::StreamExt as _;
use uuid::Uuid;

use super::ports::{
    CharacterCheckpoint, CharacterRepository, CheckpointOutcome, IdempotencyKey, SessionAudit,
};
use super::replay_log::{AppliedTickRecord, EventLog};
use crate::domain::zone::{
    AppliedTick, AppliedTickDraft, CombatRole, EntityId, ProgressionDelta, Tick, ZoneCommand,
    ZoneId, ZoneSnapshot, ZoneState, UNITS_PER_TILE,
};
use crate::domain::{CharacterId, DomainEvent, EventMetadata, Position, SessionId};

/// Checkpoint telemetry, separate from simulation telemetry.
pub trait CheckpointMetrics: Send + Sync {
    /// A failed transaction attempt.
    fn failed(&self);
    /// Age of the pending request; zero after acknowledgement.
    fn lag(&self, age: Duration);
}

struct Player {
    revision: u64,
    last_saved: Tick,
    latest: Option<CharacterCheckpoint>,
    events: Vec<DomainEvent>,
    dirty: bool,
    fenced: bool,
}

/// Serial checkpoint lane for one zone. The actor awaits this lane after log admission, so
/// backpressure retains every transition and DB latency cannot reorder simulation commands.
pub struct CheckpointService {
    repo: Arc<dyn CharacterRepository>,
    audit: Arc<dyn SessionAudit>,
    metrics: Arc<dyn CheckpointMetrics>,
    players: BTreeMap<EntityId, Player>,
    sessions: BTreeMap<EntityId, SessionId>,
    last: Option<(u64, Tick)>,
}

impl CheckpointService {
    /// Builds a lane without starting a task or subscribing to lossy broadcasts.
    pub fn new(
        repo: Arc<dyn CharacterRepository>,
        audit: Arc<dyn SessionAudit>,
        metrics: Arc<dyn CheckpointMetrics>,
    ) -> Self {
        Self {
            repo,
            audit,
            metrics,
            players: BTreeMap::new(),
            sessions: BTreeMap::new(),
            last: None,
        }
    }

    /// Associates acknowledgements with the owning socket. Replacements share the same lane.
    pub fn bind_session(&mut self, entity: EntityId, session: SessionId) {
        self.sessions.insert(entity, session);
    }

    /// Saves the previous admitted state before a lifecycle command removes or replaces it.
    pub async fn flush(&mut self, entity: EntityId) {
        if let Some(player) = self.players.get_mut(&entity) {
            save(
                self.repo.as_ref(),
                self.audit.as_ref(),
                self.metrics.as_ref(),
                self.sessions.get(&entity).copied(),
                player,
            )
            .await;
        }
    }

    /// Consumes one live, admitted batch and its end-of-tick state. Duplicate batches are no-ops.
    pub async fn admitted(&mut self, tick: &AppliedTick, snapshot: &ZoneSnapshot) {
        if self
            .last
            .is_some_and(|last| last >= (tick.epoch, tick.tick))
        {
            return;
        }
        for command in &tick.commands {
            if tick
                .dispositions
                .iter()
                .any(|d| d.ordinal == command.ordinal)
            {
                continue;
            }
            if let ZoneCommand::SpawnPlayer {
                entity,
                load: Some(load),
                ..
            } = &command.command
            {
                if let Some(revision) = load.checkpoint_revision {
                    self.players.insert(
                        *entity,
                        Player {
                            revision,
                            last_saved: tick.tick,
                            latest: None,
                            events: Vec::new(),
                            dirty: false,
                            fenced: false,
                        },
                    );
                }
            }
        }
        for entity in &snapshot.entities {
            let Some(player) = self.players.get_mut(&entity.id) else {
                continue;
            };
            let Some(combat) = &entity.combat else {
                continue;
            };
            let CombatRole::Player { xp, .. } = combat.role else {
                continue;
            };
            let mut cp = CharacterCheckpoint {
                character_id: CharacterId::from_uuid(entity.id.as_uuid()),
                revision_seen: player.revision,
                level: combat.sheet.level(),
                xp,
                hp: combat.hp,
                mp: combat.mp,
                alive: !entity.targeting.dead,
                position: position(entity.pos),
                idempotency: (
                    "save_checkpoint".to_owned(),
                    IdempotencyKey::from_uuid(stable_id(
                        entity.id,
                        snapshot.seed.zone,
                        tick.epoch,
                        tick.tick,
                        None,
                    )),
                ),
            };
            // ProgressionDelta is the authoritative persistence fact; the boundary also
            // supplies movement and resource-only changes on ticks without a delta.
            if let Some(delta) = tick.progression().find(|d| d.entity == entity.id) {
                cp.level = delta.level;
                cp.xp = delta.xp;
                cp.hp = delta.hp;
                cp.mp = delta.mp;
                cp.alive = delta.alive;
                cp.position = position(delta.pos);
            }
            let changed = player.latest.as_ref().is_none_or(|old| {
                old.level != cp.level
                    || old.xp != cp.xp
                    || old.hp != cp.hp
                    || old.mp != cp.mp
                    || old.alive != cp.alive
                    || old.position != cp.position
            });
            player.dirty |= changed;
            player.latest = Some(cp);
            let mut immediate = false;
            for delta in tick.progression().filter(|d| d.entity == entity.id) {
                immediate |=
                    delta.died.is_some() || delta.level_before != delta.level || delta.respawned;
                player.events.extend(domain_events(
                    snapshot.seed.zone,
                    tick.epoch,
                    player.revision.saturating_add(1),
                    delta,
                ));
            }
            if immediate || tick.tick.0.saturating_sub(player.last_saved.0) >= 50 {
                save(
                    self.repo.as_ref(),
                    self.audit.as_ref(),
                    self.metrics.as_ref(),
                    self.sessions.get(&entity.id).copied(),
                    player,
                )
                .await;
                player.last_saved = tick.tick;
            }
        }
        self.players.retain(|id, _| {
            let present = snapshot.entities.iter().any(|e| e.id == *id);
            if !present {
                self.sessions.remove(id);
            }
            present
        });
        self.last = Some((tick.epoch, tick.tick));
    }

    /// Recovers only a durable, contiguous, validated prefix before the next epoch/admission.
    /// This does not change `open_epoch` or the verifier's refusal of incomplete epochs.
    pub async fn recover(
        &mut self,
        log: &dyn EventLog,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<()> {
        let stored = log
            .read_snapshot(zone, epoch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("checkpoint recovery snapshot missing"))?;
        let mut state = ZoneState::from_snapshot(stored.snapshot)?;
        let mut records = log.read_epoch(zone, epoch).await?;
        while let Some(record) = records.next().await {
            let record = record?;
            anyhow::ensure!(
                record.zone == zone && record.epoch == epoch,
                "checkpoint prefix crossed epochs"
            );
            let applied = state.run_tick(AppliedTickDraft {
                epoch,
                tick: record.tick,
                commands: record.commands.clone(),
            })?;
            anyhow::ensure!(
                record.reproduced_by(&AppliedTickRecord::from_applied(zone, &applied)),
                "checkpoint prefix diverged at tick {}",
                record.tick.0
            );
            // The previous boundary is saved before the recorded lifecycle transition.
            for command in &applied.commands {
                if let ZoneCommand::Despawn { entity }
                | ZoneCommand::ReplaceSession { entity, .. } = command.command
                {
                    self.flush(entity).await;
                }
            }
            self.admitted(&applied, &state.snapshot()).await;
        }
        // Ordinary progress may roll back; critical facts have already been checkpointed.
        self.players.clear();
        self.sessions.clear();
        self.last = None;
        Ok(())
    }
}

async fn save(
    repo: &dyn CharacterRepository,
    audit: &dyn SessionAudit,
    metrics: &dyn CheckpointMetrics,
    session: Option<SessionId>,
    player: &mut Player,
) {
    if !player.dirty || player.fenced {
        return;
    }
    let Some(cp) = &player.latest else {
        return;
    };
    let started = Instant::now();
    let mut backoff = Duration::from_millis(100);
    loop {
        metrics.lag(started.elapsed());
        match observe_attempt(repo.checkpoint(cp, &player.events), metrics, started).await {
            Ok(CheckpointOutcome::Applied(revision) | CheckpointOutcome::Replayed(revision)) => {
                if let Some(session) = session {
                    audit.record_checkpoint(
                        session,
                        &CheckpointAck {
                            character_id: cp.character_id,
                            key: cp.idempotency.1,
                            revision,
                        },
                    );
                }
                player.revision = revision;
                player.events.clear();
                player.dirty = false;
                metrics.lag(Duration::ZERO);
                return;
            },
            Ok(CheckpointOutcome::Stale) => {
                player.fenced = true;
                metrics.failed();
                metrics.lag(Duration::ZERO);
                tracing::warn!(character = %cp.character_id, "checkpoint fenced by newer revision");
                return;
            },
            Err(e) => {
                metrics.failed();
                tracing::warn!(error = %e, character = %cp.character_id, "checkpoint retained for retry");
                observe_attempt(tokio::time::sleep(backoff), metrics, started).await;
                backoff = backoff.saturating_mul(2).min(Duration::from_secs(5));
            },
        }
    }
}

async fn observe_attempt<T>(
    future: impl std::future::Future<Output = T>,
    metrics: &dyn CheckpointMetrics,
    started: Instant,
) -> T {
    tokio::pin!(future);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut future => return result,
            _ = interval.tick() => metrics.lag(started.elapsed()),
        }
    }
}

/// Saved checkpoint acknowledgement, audit-only; never sent as a wire frame.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointAck {
    /// Saved character.
    pub character_id: CharacterId,
    /// Stable checkpoint key.
    pub key: IdempotencyKey,
    /// Committed revision.
    pub revision: u64,
}

fn position(pos: crate::domain::zone::Vec2Fixed) -> Position {
    #[allow(clippy::cast_possible_truncation)]
    let tiles = |raw: i32| (f64::from(raw) / f64::from(UNITS_PER_TILE)) as f32;
    Position {
        x: tiles(pos.x.raw()),
        y: tiles(pos.y.raw()),
    }
}

fn stable_id(entity: EntityId, zone: ZoneId, epoch: u64, tick: Tick, ordinal: Option<u64>) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(b"nightfall.checkpoint.v1");
    hash.update(entity.as_uuid().as_bytes());
    hash.update(zone.0.to_be_bytes());
    hash.update(epoch.to_be_bytes());
    hash.update(tick.0.to_be_bytes());
    if let Some(n) = ordinal {
        hash.update(n.to_be_bytes());
    }
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    for (out, byte) in bytes.iter_mut().zip(digest.iter()) {
        *out = *byte;
    }
    Uuid::from_bytes(bytes)
}

/// Maps each transition once, retaining every intermediate level and the death penalty.
pub fn domain_events(
    zone: ZoneId,
    epoch: u64,
    revision: u64,
    delta: &ProgressionDelta,
) -> Vec<DomainEvent> {
    let character_id = CharacterId::from_uuid(delta.entity.as_uuid());
    let metadata = |ordinal| EventMetadata {
        event_id: stable_id(delta.entity, zone, epoch, delta.tick, Some(ordinal)),
        sequence: (revision, ordinal),
    };
    let mut events: Vec<_> = delta
        .levels_gained
        .iter()
        .enumerate()
        .map(|(ordinal, level)| DomainEvent::CharacterLeveled {
            metadata: metadata(ordinal as u64),
            character_id,
            level: *level,
        })
        .collect();
    if let Some(death) = &delta.died {
        events.push(DomainEvent::CharacterDied {
            metadata: metadata(events.len() as u64),
            character_id,
            killer: death
                .killer_template
                .clone()
                .or_else(|| death.killer.map(|e| e.to_string()))
                .unwrap_or_default(),
            level: delta.level,
            xp: delta.xp,
            xp_lost: death.xp_lost,
        });
    }
    events
}

/// Late-bound lane: installed by the composition root before socket admission.
#[derive(Clone, Default)]
pub struct CheckpointLane(
    Arc<parking_lot::RwLock<Option<Arc<tokio::sync::Mutex<CheckpointService>>>>>,
);

impl std::fmt::Debug for CheckpointLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointLane").finish_non_exhaustive()
    }
}

impl CheckpointLane {
    /// Installs exactly once; bootstrap may already have installed a recovered lane.
    pub fn install(&self, service: CheckpointService) {
        self.0
            .write()
            .get_or_insert_with(|| Arc::new(tokio::sync::Mutex::new(service)));
    }
    /// The installed lane, if persistence is enabled.
    pub fn service(&self) -> Option<Arc<tokio::sync::Mutex<CheckpointService>>> {
        self.0.read().clone()
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
