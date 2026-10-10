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
    /// Actual newly committed token; replayed acknowledgements do not re-emit metrics.
    fn token_granted(&self, _tier: u8, _source: crate::domain::character_progression::TokenSource) {
    }
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
    durability: Option<(Arc<dyn EventLog>, Arc<dyn super::replay_log::ZoneSnapshotStore>)>,
    refreshed: Instant,
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
            durability: None,
            refreshed: Instant::now(),
        }
    }

    /// Enables durable discovery and periodic recovery baselines in the production lane.
    #[must_use]
    pub fn with_durability(
        mut self,
        log: Arc<dyn EventLog>,
        store: Arc<dyn super::replay_log::ZoneSnapshotStore>,
    ) -> Self {
        self.durability = Some((log, store));
        self
    }

    /// Records intent before the log gate, so even a missing final record fails closed.
    pub async fn recording(&self, zone: ZoneId, epoch: u64, tick: Tick) {
        self.index_tick(zone, epoch, tick, false).await;
    }

    async fn index_tick(&self, zone: ZoneId, epoch: u64, tick: Tick, completed: bool) {
        let Some((_, store)) = &self.durability else {
            return;
        };
        loop {
            let result = if completed {
                store.checkpointed(zone, epoch, tick).await
            } else {
                store.recording(zone, epoch, tick).await
            };
            match result {
                Ok(()) => return,
                Err(error) => {
                    self.metrics.failed();
                    tracing::error!(zone = zone.0, epoch, error = %error, "epoch index update retained for retry");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                },
            }
        }
    }

    async fn refresh(&mut self, snapshot: &ZoneSnapshot) -> anyhow::Result<()> {
        let Some((log, store)) = &self.durability else {
            return Ok(());
        };
        let mut snapshot = snapshot.clone();
        self.snapshot(&mut snapshot);
        let seq = log.write_recovery_snapshot(&snapshot).await?;
        store
            .insert(&super::replay_log::ZoneSnapshotRow {
                zone: snapshot.seed.zone,
                epoch: snapshot.seed.epoch,
                snapshot: super::replay_log::encode_snapshot(&snapshot)?,
                snapshot_seq: seq,
                first_seq: None,
                time_origin_ms: snapshot.time_origin_ms,
                build_id: snapshot.meta.build_id.clone(),
                config_hash: snapshot.meta.config_hash.clone(),
                schema_version: snapshot.meta.schema_version,
            })
            .await?;
        self.refreshed = Instant::now();
        Ok(())
    }

    /// Saves every remaining player and a fresh baseline before marking the epoch resolved.
    pub async fn shutdown(&mut self, snapshot: &ZoneSnapshot) -> anyhow::Result<()> {
        let players: Vec<_> = self.players.keys().copied().collect();
        for entity in players {
            self.flush(entity).await?;
        }
        self.refresh(snapshot).await?;
        if let Some((_, store)) = &self.durability {
            store.close(snapshot.seed.zone, snapshot.seed.epoch).await?;
        }
        Ok(())
    }

    /// Recovers every unresolved durable-index epoch before a new zone can admit players.
    /// Missing or noncontiguous history is an error even if stream discovery sees no epoch.
    pub async fn recover_indexed(
        &mut self,
        log: &dyn EventLog,
        store: &dyn super::replay_log::ZoneSnapshotStore,
        zone: ZoneId,
    ) -> anyhow::Result<()> {
        for indexed in store.unresolved(zone).await? {
            let result = self
                .recover_through(log, zone, indexed.epoch, indexed.last_recorded_tick)
                .await;
            if let Err(error) = result {
                tracing::error!(zone = zone.0, epoch = indexed.epoch, error = %error, "zone admission refused: unresolved checkpoint recovery history");
                return Err(error);
            }
            if let Some(tick) = indexed.last_recorded_tick {
                store.checkpointed(zone, indexed.epoch, tick).await?;
            }
            store.close(zone, indexed.epoch).await?;
        }
        Ok(())
    }

    /// Captures every lane at a boundary after all in-flight requests have completed.
    pub fn snapshot(&self, snapshot: &mut ZoneSnapshot) {
        snapshot.checkpoints = self
            .players
            .iter()
            .map(|(entity, p)| crate::domain::zone::CheckpointSnapshot {
                entity: *entity,
                revision: p.revision,
                last_saved: p.last_saved,
                dirty: p.dirty,
                fenced: p.fenced,
                events: p.events.clone(),
                latest: p.latest.as_ref().map(|cp| {
                    crate::domain::zone::CheckpointRequestSnapshot {
                        class_state: cp.class_state.clone(),
                        revision_seen: cp.revision_seen,
                        level: cp.level,
                        xp: cp.xp,
                        hp: cp.hp,
                        mp: cp.mp,
                        alive: cp.alive,
                        position_bits: [cp.position.x.to_bits(), cp.position.y.to_bits()],
                        key: cp.idempotency.1.as_uuid(),
                    }
                }),
            })
            .collect();
    }

    /// Restores revision fences, cadence and pending facts before accepting another tick.
    pub fn restore(&mut self, snapshot: &ZoneSnapshot) {
        self.players = snapshot
            .checkpoints
            .iter()
            .map(|p| {
                (
                    p.entity,
                    Player {
                        revision: p.revision,
                        last_saved: p.last_saved,
                        dirty: p.dirty,
                        fenced: p.fenced,
                        events: p.events.clone(),
                        latest: p.latest.as_ref().map(|cp| CharacterCheckpoint {
                            class_state: cp.class_state.clone(),
                            character_id: CharacterId::from_uuid(p.entity.as_uuid()),
                            revision_seen: cp.revision_seen,
                            level: cp.level,
                            xp: cp.xp,
                            hp: cp.hp,
                            mp: cp.mp,
                            alive: cp.alive,
                            position: Position {
                                x: f32::from_bits(cp.position_bits[0]),
                                y: f32::from_bits(cp.position_bits[1]),
                            },
                            idempotency: (
                                "save_checkpoint".to_owned(),
                                IdempotencyKey::from_uuid(cp.key),
                            ),
                        }),
                    },
                )
            })
            .collect();
        self.last = snapshot
            .tick
            .0
            .checked_sub(1)
            .map(|tick| (snapshot.seed.epoch, Tick(tick)));
    }

    /// Rechecks account-wide successful receipts before a new actor mutation is drafted.
    pub async fn mutation_receipt_lookup(
        &self,
        account: crate::domain::AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<super::ports::MutationReceiptLookup> {
        self.repo
            .mutation_receipt_lookup(account, key, fingerprint)
            .await
    }

    /// A new transfer requires a committed revision lane, even before its command is run.
    pub fn transfer_ready(&self, entity: EntityId) -> bool {
        self.players.get(&entity).is_some_and(|p| !p.fenced)
    }

    /// Associates acknowledgements with the owning socket. Replacements share the same lane.
    pub fn bind_session(&mut self, entity: EntityId, session: SessionId) {
        self.sessions.insert(entity, session);
    }

    /// Saves the previous admitted state before a lifecycle command removes or replaces it.
    pub async fn flush(&mut self, entity: EntityId) -> anyhow::Result<()> {
        if let Some(player) = self.players.get_mut(&entity) {
            save(
                self.repo.as_ref(),
                self.audit.as_ref(),
                self.metrics.as_ref(),
                self.sessions.get(&entity).copied(),
                player,
            )
            .await?;
        }
        Ok(())
    }

    /// Consumes one live, admitted batch and its end-of-tick state. Duplicate batches are no-ops.
    #[allow(clippy::too_many_lines)] // One ordered projection/save barrier; never split into detached side effects.
    pub async fn admitted(
        &mut self,
        tick: &AppliedTick,
        snapshot: &ZoneSnapshot,
    ) -> anyhow::Result<()> {
        if self
            .last
            .is_some_and(|last| last >= (tick.epoch, tick.tick))
        {
            return Ok(());
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
        for event in &tick.events {
            if let crate::domain::zone::ZoneEvent::ClassTransfer { entity, .. }
            | crate::domain::zone::ZoneEvent::TokensReconciled { entity, .. } = event
            {
                anyhow::ensure!(
                    self.players.get(entity).is_some_and(|p| !p.fenced),
                    "class transfer checkpoint lane missing or fenced"
                );
                anyhow::ensure!(
                    snapshot.entities.iter().any(|e| e.id == *entity),
                    "class transfer entity removed before persistence"
                );
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
                class_state: match &combat.role {
                    CombatRole::Player { progression, .. } => {
                        progression.as_ref().map(|p| p.class_state.clone())
                    },
                    CombatRole::Npc { .. } => None,
                },
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
                old.class_state != cp.class_state
                    || old.level != cp.level
                    || old.xp != cp.xp
                    || old.hp != cp.hp
                    || old.mp != cp.mp
                    || old.alive != cp.alive
                    || old.position != cp.position
            });
            player.dirty |= changed;
            player.latest = Some(cp);
            let mut immediate = tick.events.iter().any(|event| matches!(event,
                crate::domain::zone::ZoneEvent::TokensReconciled { entity: who, .. } if *who == entity.id));
            for event in &tick.events {
                if let crate::domain::zone::ZoneEvent::TokensReconciled {
                    entity: who,
                    tick: at,
                    adjustment,
                    source,
                } = event
                {
                    if *who == entity.id {
                        for (tier, bit) in [(1_u8, 1), (2, 2)] {
                            if adjustment.granted_mask & bit != 0 {
                                let ordinal = player.events.len() as u64;
                                player.events.push(DomainEvent::CharacterTokenGranted {
                                    metadata: EventMetadata {
                                        event_id: stable_id(
                                            entity.id,
                                            snapshot.seed.zone,
                                            tick.epoch,
                                            *at,
                                            Some(ordinal),
                                        ),
                                        sequence: (player.revision.saturating_add(1), ordinal),
                                    },
                                    character_id: CharacterId::from_uuid(entity.id.as_uuid()),
                                    tier,
                                    source: *source,
                                });
                            }
                        }
                    }
                }
                if let crate::domain::zone::ZoneEvent::ClassTransfer {
                    tick: at,
                    entity: who,
                    old_class_id,
                    receipt,
                } = event
                {
                    if *who != entity.id {
                        continue;
                    }
                    anyhow::ensure!(
                        player
                            .latest
                            .as_ref()
                            .and_then(|cp| cp.class_state.as_ref())
                            .and_then(|s| s.receipt(receipt.key))
                            == Some(receipt.as_ref()),
                        "class transfer receipt missing from checkpoint"
                    );
                    immediate = true;
                    let ordinal = player.events.len() as u64;
                    player.events.push(DomainEvent::CharacterClassChanged {
                        metadata: EventMetadata {
                            event_id: stable_id(
                                entity.id,
                                snapshot.seed.zone,
                                tick.epoch,
                                *at,
                                Some(ordinal),
                            ),
                            sequence: (player.revision.saturating_add(1), ordinal),
                        },
                        character_id: CharacterId::from_uuid(entity.id.as_uuid()),
                        old_class_id: *old_class_id,
                        new_class_id: receipt.target_class_id,
                        tick: at.0,
                        request_key: receipt.key,
                    });
                }
            }
            for delta in tick.progression().filter(|d| d.entity == entity.id) {
                immediate |=
                    delta.died.is_some() || delta.level_before != delta.level || delta.respawned;
                let mut facts = domain_events(
                    snapshot.seed.zone,
                    tick.epoch,
                    player.revision.saturating_add(1),
                    delta,
                );
                if snapshot.classes.is_some() {
                    let offset = player.events.len() as u64;
                    for (n, event) in facts.iter_mut().enumerate() {
                        let metadata = match event {
                            DomainEvent::CharacterLeveled { metadata, .. }
                            | DomainEvent::CharacterDied { metadata, .. }
                            | DomainEvent::CharacterClassChanged { metadata, .. }
                            | DomainEvent::CharacterTokenGranted { metadata, .. } => metadata,
                            DomainEvent::CharacterCreated { .. } => continue,
                        };
                        let ordinal = offset.saturating_add(n as u64);
                        metadata.event_id = stable_id(
                            entity.id,
                            snapshot.seed.zone,
                            tick.epoch,
                            delta.tick,
                            Some(ordinal),
                        );
                        metadata.sequence.1 = ordinal;
                    }
                }
                player.events.extend(facts);
            }
            if immediate || tick.tick.0.saturating_sub(player.last_saved.0) >= 50 {
                save(
                    self.repo.as_ref(),
                    self.audit.as_ref(),
                    self.metrics.as_ref(),
                    self.sessions.get(&entity.id).copied(),
                    player,
                )
                .await?;
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
        self.index_tick(snapshot.seed.zone, tick.epoch, tick.tick, true)
            .await;
        self.refresh_if_due(snapshot).await;
        Ok(())
    }

    async fn refresh_if_due(&mut self, snapshot: &ZoneSnapshot) {
        // 23 hours leaves an hour of margin beneath the 24-hour baseline requirement.
        if self.refreshed.elapsed() >= Duration::from_hours(23) {
            while let Err(error) = self.refresh(snapshot).await {
                self.metrics.failed();
                tracing::error!(error = %error, "recovery baseline refresh retained for retry");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    /// Recovers only a durable, contiguous, validated prefix before the next epoch/admission.
    /// This does not change `open_epoch` or the verifier's refusal of incomplete epochs.
    pub async fn recover(
        &mut self,
        log: &dyn EventLog,
        zone: ZoneId,
        epoch: u64,
    ) -> anyhow::Result<()> {
        self.recover_through(log, zone, epoch, None).await
    }

    async fn recover_through(
        &mut self,
        log: &dyn EventLog,
        zone: ZoneId,
        epoch: u64,
        required: Option<Tick>,
    ) -> anyhow::Result<()> {
        let stored = log
            .read_recovery_snapshot(zone, epoch)
            .await?
            .ok_or_else(|| anyhow::anyhow!("checkpoint recovery snapshot missing"))?;
        anyhow::ensure!(
            stored.snapshot.meta.schema_version >= 5
                || stored
                    .snapshot
                    .entities
                    .iter()
                    .all(|e| e.kind != crate::domain::zone::EntityKind::Player),
            "legacy mid-fight snapshot lacks checkpoint lanes"
        );
        self.restore(&stored.snapshot);
        let first_tick = stored.snapshot.tick;
        let mut state = ZoneState::from_snapshot(stored.snapshot)?;
        let mut records = log.read_epoch(zone, epoch).await?;
        while let Some(record) = records.next().await {
            let record = record?;
            if record.tick < first_tick {
                continue;
            }
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
                    self.flush(entity).await?;
                }
            }
            self.admitted(&applied, &state.snapshot()).await?;
        }
        anyhow::ensure!(
            required.is_none_or(|tick| state.snapshot().tick > tick),
            "checkpoint recovery history ends before indexed tick {required:?}"
        );
        // A baseline can itself carry pending critical facts, even without a later record.
        let pending: Vec<_> = self
            .players
            .iter()
            .filter(|(_, p)| !p.events.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for entity in pending {
            self.flush(entity).await?;
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
) -> anyhow::Result<()> {
    anyhow::ensure!(!player.fenced, "checkpoint lane is fenced");
    if !player.dirty {
        return Ok(());
    }
    let cp = player
        .latest
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("dirty checkpoint has no request"))?;
    let started = Instant::now();
    let mut backoff = Duration::from_millis(100);
    loop {
        metrics.lag(started.elapsed());
        match observe_attempt(repo.checkpoint(cp, &player.events), metrics, started).await {
            Ok(
                outcome @ (CheckpointOutcome::Applied(revision)
                | CheckpointOutcome::Replayed(revision)),
            ) => {
                if matches!(outcome, CheckpointOutcome::Applied(_)) {
                    for event in &player.events {
                        if let DomainEvent::CharacterTokenGranted { tier, source, .. } = event {
                            metrics.token_granted(*tier, *source);
                        }
                    }
                }
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
                return Ok(());
            },
            Ok(CheckpointOutcome::Stale) => {
                player.fenced = true;
                metrics.failed();
                metrics.lag(Duration::ZERO);
                tracing::warn!(character = %cp.character_id, "checkpoint fenced by newer revision");
                anyhow::bail!("checkpoint fenced by newer revision");
            },
            Err(
                e @ (super::ports::CheckpointError::KeyReused
                | super::ports::CheckpointError::NotFound
                | super::ports::CheckpointError::Constraint(_)),
            ) => {
                player.fenced = true;
                metrics.failed();
                return Err(e.into());
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
    // i32 milli-tiles divided by 1000 fit the finite f32 range. Rounding to the
    // persistence Position precision is intentional at this infrastructure boundary.
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
    Option<Arc<ZoneSnapshot>>,
);

impl std::fmt::Debug for CheckpointLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointLane").finish_non_exhaustive()
    }
}

impl CheckpointLane {
    pub(crate) fn with_snapshot(snapshot: ZoneSnapshot) -> Self {
        Self(Arc::default(), Some(Arc::new(snapshot)))
    }

    /// Installs exactly once; bootstrap may already have installed a recovered lane.
    pub fn install(&self, mut service: CheckpointService) {
        if let Some(snapshot) = &self.1 {
            if !snapshot.checkpoints.is_empty() {
                service.restore(snapshot);
            }
        }
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
