//! Live progression projection. Only durably admitted ticks enter this lane; replay verification
//! has no repository. Requests are immutable until acknowledged, including after a lost commit
//! response. The durable applied prefix is the recovery queue.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::ports::{
    CharacterCheckpoint, CharacterRepository, CheckpointOutcome, IdempotencyKey, SessionAudit,
};
use super::replay_log::{AppliedTickRecord, EventLog};
use crate::domain::zone::{
    AppliedTick, AppliedTickDraft, CombatRole, EntityId, Phase3Entity, ProgressionDelta, Tick,
    ZoneCommand, ZoneId, ZoneSnapshot, UNITS_PER_TILE,
};
use crate::domain::{CharacterId, DomainEvent, EventMetadata, Position, SessionId};

/// Checkpoint telemetry, separate from simulation telemetry.
pub trait CheckpointMetrics: Send + Sync {
    /// Actual newly committed token; replayed acknowledgements do not re-emit metrics.
    fn token_granted(&self, _tier: u8, _source: crate::domain::character_progression::TokenSource) {
    }
    /// A newly committed paid purchase; exact checkpoint retries do not count again.
    fn skill_learned(&self) {}
    /// A failed transaction attempt.
    fn failed(&self);
    /// Age of the pending request; zero after acknowledgement.
    fn lag(&self, age: Duration);
}

struct Player {
    revision: u64,
    last_saved: Tick,
    latest: Option<CharacterCheckpoint>,
    last_phase3: Option<Phase3Entity>,
    events: Vec<DomainEvent>,
    learning: Option<super::ports::SuccessfulLearnReceipt>,
    items: Option<super::ports::ItemCheckpointWork>,
    dirty: bool,
    fenced: bool,
}

/// Serial checkpoint lane for one zone. The actor awaits this lane after log admission, so
/// backpressure retains every transition and DB latency cannot reorder simulation commands.
pub struct CheckpointService {
    repo: Arc<dyn CharacterRepository>,
    learning_repo: Option<Arc<dyn super::ports::LearningCheckpointRepository>>,
    item_repo: Option<Arc<dyn super::ports::ItemCheckpointRepository>>,
    item_admission: Option<Arc<dyn super::ports::ItemAdmissionRepository>>,
    audit: Arc<dyn SessionAudit>,
    metrics: Arc<dyn CheckpointMetrics>,
    players: BTreeMap<EntityId, Player>,
    sessions: BTreeMap<EntityId, SessionId>,
    last: Option<(u64, Tick)>,
    durability: Option<(Arc<dyn EventLog>, Arc<dyn super::replay_log::ZoneSnapshotStore>)>,
    refreshed: Instant,
    wrote_since_index: bool,
    internal_cadence: bool,
    pending: parking_lot::Mutex<BTreeMap<IdempotencyKey, Instant>>,
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
            learning_repo: None,
            item_repo: None,
            item_admission: None,
            audit,
            metrics,
            players: BTreeMap::new(),
            sessions: BTreeMap::new(),
            last: None,
            durability: None,
            refreshed: Instant::now(),
            wrote_since_index: false,
            internal_cadence: true,
            pending: parking_lot::Mutex::default(),
        }
    }

    /// Installs atomic paid-learning persistence before admitting experimental players.
    #[must_use]
    pub fn with_learning(
        mut self,
        repo: Arc<dyn super::ports::LearningCheckpointRepository>,
    ) -> Self {
        self.learning_repo = Some(repo);
        self
    }

    /// Installs the same atomic item/learning writer before recovery or V6 admission.
    #[must_use]
    pub fn with_items(mut self, repo: Arc<dyn super::ports::ItemCheckpointRepository>) -> Self {
        self.item_repo = Some(repo);
        self
    }
    /// Installs durable range reservations before an exhausted V6 actor resumes.
    #[must_use]
    pub fn with_item_admission(
        mut self,
        repo: Arc<dyn super::ports::ItemAdmissionRepository>,
    ) -> Self {
        self.item_admission = Some(repo);
        self
    }
    pub(crate) async fn extend_item_range(
        &self,
        seed: crate::domain::zone::ZoneSeed,
        exhausted_start: u64,
    ) -> Result<super::ports::ItemIdRange, super::CheckpointError> {
        self.item_admission
            .as_ref()
            .ok_or_else(|| super::CheckpointError::Constraint("item_admission_required".into()))?
            .reserve_item_range_extension(seed.zone, seed.epoch, exhausted_start)
            .await
    }
    /// Item simulation cannot activate without a typed durable receipt lane.
    pub fn items_enabled(&self) -> bool {
        self.item_repo.is_some()
    }
    /// Trusted actor preflight account/operation lookup, before simulation/RNG.
    pub async fn item_receipt_lookup(
        &self,
        account: crate::domain::AccountId,
        operation: super::ports::ItemMutationOperation,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<super::ports::ItemReceiptLookup> {
        self.item_repo
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("item persistence unavailable"))?
            .item_receipt_lookup(account, operation, key, fingerprint)
            .await
    }

    /// Whether paid mutations have a typed receipt/checkpoint writer.
    pub fn learning_enabled(&self) -> bool {
        self.learning_repo.is_some()
    }

    /// Account-wide frozen lookup, before live generation or gameplay checks.
    pub async fn learn_receipt_lookup(
        &self,
        account: crate::domain::AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<super::ports::LearnReceiptLookup> {
        self.learning_repo
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("learning persistence unavailable"))?
            .learn_receipt_lookup(account, key, fingerprint)
            .await
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
            close_recovered_epoch(log, zone, indexed.epoch).await?;
            store.close(zone, indexed.epoch).await?;
        }
        Ok(())
    }

    /// Captures every lane at a boundary after all in-flight requests have completed.
    pub fn snapshot(&self, snapshot: &mut ZoneSnapshot) {
        snapshot.meta.checkpoint_internal_cadence =
            snapshot.phase3.is_some() && self.internal_cadence;
        snapshot.checkpoints = self
            .players
            .iter()
            .map(|(entity, p)| crate::domain::zone::CheckpointSnapshot {
                learning: p
                    .learning
                    .as_ref()
                    .filter(|_| p.latest.as_ref().is_some_and(|cp| cp.phase4.is_some()))
                    .map(|receipt| crate::domain::zone::CheckpointLearnReceiptSnapshot {
                        account: receipt.account_id,
                        key: receipt.key.as_uuid(),
                        fingerprint: receipt.request_fingerprint.clone(),
                        result: receipt.result.clone(),
                    }),
                item_work: p.items.as_ref().map(|work| {
                    crate::domain::zone::ItemCheckpointWorkSnapshot {
                        allocation: work.allocation,
                        projection: work.projection.clone(),
                    }
                }),
                entity: *entity,
                revision: p.revision,
                last_saved: p.last_saved,
                dirty: p.dirty,
                fenced: p.fenced,
                events: p.events.clone(),
                latest: p.latest.as_ref().map(|cp| {
                    crate::domain::zone::CheckpointRequestSnapshot {
                        phase4: cp.phase4.clone(),
                        phase3: cp.phase3.clone(),
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
        self.internal_cadence = snapshot.meta.checkpoint_internal_cadence;
        self.wrote_since_index = false;
        self.players = snapshot
            .checkpoints
            .iter()
            .map(|p| {
                (
                    p.entity,
                    Player {
                        last_phase3: snapshot
                            .phase3
                            .as_ref()
                            .and_then(|phase| phase.entities.get(&p.entity))
                            .cloned(),
                        revision: p.revision,
                        last_saved: p.last_saved,
                        dirty: p.dirty,
                        fenced: p.fenced,
                        events: p.events.clone(),
                        learning: p.learning.as_ref().map(|receipt| {
                            super::ports::SuccessfulLearnReceipt {
                                account_id: receipt.account,
                                key: IdempotencyKey::from_uuid(receipt.key),
                                request_fingerprint: receipt.fingerprint.clone(),
                                result: receipt.result.clone(),
                            }
                        }),
                        items: p
                            .item_work
                            .as_ref()
                            .map(|work| super::ports::ItemCheckpointWork {
                                zone: snapshot.seed.zone,
                                epoch: snapshot.seed.epoch,
                                allocation: work.allocation,
                                projection: work.projection.clone(),
                            }),
                        latest: p.latest.as_ref().map(|cp| CharacterCheckpoint {
                            phase4: cp.phase4.clone(),
                            phase3: cp.phase3.clone(),
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
            // Idle boundaries refresh the template with the current revision and paused
            // timers. A just-acknowledged template still names the previous revision: that
            // exact boundary is already saved, including its learning fingerprint/receipt.
            player.dirty |= player
                .latest
                .as_ref()
                .is_some_and(|cp| cp.phase3.is_some() && cp.revision_seen == player.revision);
            let wrote = player.dirty;
            save(
                self.repo.as_ref(),
                self.learning_repo.as_deref(),
                self.item_repo.as_deref(),
                self.audit.as_ref(),
                self.metrics.as_ref(),
                &self.pending,
                self.sessions.get(&entity).copied(),
                player,
            )
            .await?;
            self.wrote_since_index |= wrote;
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
                            last_phase3: None,
                            last_saved: tick.tick,
                            latest: None,
                            events: Vec::new(),
                            learning: None,
                            items: None,
                            dirty: false,
                            fenced: false,
                        },
                    );
                }
            }
        }
        self.paused_items(tick, snapshot).await?;
        for event in &tick.events {
            if let crate::domain::zone::ZoneEvent::ClassTransfer { entity, .. }
            | crate::domain::zone::ZoneEvent::TokensReconciled { entity, .. }
            | crate::domain::zone::ZoneEvent::SkillLearned { entity, .. } = event
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
        let mut due = BTreeSet::new();
        for entity in &snapshot.entities {
            if snapshot.phase3.is_some() && entity.kind == crate::domain::zone::EntityKind::Player {
                anyhow::ensure!(
                    self.players.get(&entity.id).is_some_and(|p| !p.fenced),
                    "Phase3 player checkpoint lane missing or fenced"
                );
            }
            let Some(player) = self.players.get_mut(&entity.id) else {
                continue;
            };
            let Some(combat) = &entity.combat else {
                continue;
            };
            let CombatRole::Player { xp, .. } = combat.role else {
                continue;
            };
            let phase3 = snapshot.phase3.as_ref();
            let retained = phase3.and_then(|phase| phase.entities.get(&entity.id));
            anyhow::ensure!(
                phase3.is_none() || retained.is_some(),
                "player checkpoint sidecar missing"
            );
            let mut cp = CharacterCheckpoint {
                phase4: snapshot
                    .phase4
                    .as_ref()
                    .map(|phase| {
                        let skills = snapshot
                            .phase3
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("item checkpoint needs skill state"))?;
                        phase
                            .checkpoint(entity.id, tick.tick, skills)
                            .map_err(anyhow::Error::from)
                    })
                    .transpose()?,
                phase3: phase3
                    .map(|phase| phase.checkpoint(entity.id, tick.tick))
                    .transpose()?,
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
            let projections: Vec<_> = tick
                .events
                .iter()
                .filter_map(|event| {
                    if let crate::domain::zone::ZoneEvent::ItemCheckpoint(projection) = event {
                        (projection.entity == entity.id).then_some(projection.as_ref())
                    } else {
                        None
                    }
                })
                .collect();
            anyhow::ensure!(
                projections.len() <= 1,
                "multiple final item checkpoints for one tick/owner"
            );
            if projections.is_empty() {
                if let Some(before) = player.latest.as_ref().and_then(|cp| cp.phase4.as_ref()) {
                    let mut before = before.clone();
                    let mut after = cp
                        .phase4
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("item sidecar removed"))?;
                    // Paused remainders fall with time; every other retained change needs its frozen work.
                    before.item_reuse_remainders.clear();
                    after.item_reuse_remainders.clear();
                    anyhow::ensure!(
                        before == after,
                        "retained item mutation without private checkpoint"
                    );
                }
            }
            if let Some(projection) = projections.first() {
                anyhow::ensure!(self.item_repo.is_some(), "item checkpoint repository missing");
                let account = match &combat.role {
                    CombatRole::Player {
                        progression: Some(progression),
                        ..
                    } => progression.identity.account_id,
                    CombatRole::Player {
                        progression: None, ..
                    }
                    | CombatRole::Npc { .. } => {
                        anyhow::bail!("item checkpoint owner context missing")
                    },
                };
                let work = super::ports::ItemCheckpointWork {
                    zone: snapshot.seed.zone,
                    epoch: tick.epoch,
                    allocation: snapshot
                        .phase4
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("item allocation missing"))?
                        .allocator,
                    projection: (*projection).clone(),
                };
                work.validate(&cp, account)?;
                anyhow::ensure!(projection.tick == tick.tick, "item projection tick mismatch");
                player.items = Some(work);
                player.dirty = true;
            }
            let retained_changed = if self.internal_cadence {
                critical_phase3_changed(player.last_phase3.as_ref(), retained)
            } else {
                retained_phase3_changed(player.last_phase3.as_ref(), retained)
            };
            let retained_critical = phase3.is_some()
                && (retained_changed
                    || player
                        .latest
                        .as_ref()
                        .is_none_or(|old| old.class_state != cp.class_state));
            let changed = player.latest.as_ref().is_none_or(|old| {
                old.class_state != cp.class_state
                    || old.level != cp.level
                    || old.xp != cp.xp
                    || old.hp != cp.hp
                    || old.mp != cp.mp
                    || old.alive != cp.alive
                    || old.position != cp.position
            });
            player.dirty |=
                changed || retained_phase3_changed(player.last_phase3.as_ref(), retained);
            player.latest = Some(cp.clone());
            player.last_phase3 = retained.cloned();
            let mut immediate = player.items.is_some() || retained_critical || tick.events.iter().any(|event| matches!(event,
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
            for event in &tick.events {
                if let crate::domain::zone::ZoneEvent::SkillLearned {
                    tick: at,
                    entity: who,
                    account,
                    request_key,
                    result,
                } = event
                {
                    if *who != entity.id {
                        continue;
                    }
                    anyhow::ensure!(
                        self.learning_repo.is_some(),
                        "learning persistence unavailable"
                    );
                    anyhow::ensure!(
                        player.learning.is_none(),
                        "multiple learning facts in one tick"
                    );
                    let receipt = super::ports::SuccessfulLearnReceipt {
                        account_id: *account,
                        key: IdempotencyKey::from_uuid(*request_key),
                        request_fingerprint: super::ports::learn_skill_fingerprint(
                            result.character_id,
                            result.skill,
                        ),
                        result: result.as_ref().clone(),
                    };
                    let cp = player
                        .latest
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("learning checkpoint missing"))?;
                    receipt.validate(cp, *account)?;
                    let ordinal = player.events.len() as u64;
                    player.events.push(DomainEvent::CharacterSkillLearned {
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
                        character_id: result.character_id,
                        skill_id: result.skill.id.0,
                        skill_level: result.skill.rank,
                        sp_spent: result.sp_spent,
                        sp_remaining: result.sp_remaining,
                        tick: at.0,
                        request_key: *request_key,
                    });
                    player.learning = Some(receipt);
                    player.dirty = true;
                    immediate = true;
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
                            | DomainEvent::CharacterTokenGranted { metadata, .. }
                            | DomainEvent::CharacterSkillLearned { metadata, .. } => metadata,
                            DomainEvent::ItemGranted { fact }
                            | DomainEvent::ItemEquipped { fact }
                            | DomainEvent::ItemUnequipped { fact }
                            | DomainEvent::ItemConsumed { fact }
                            | DomainEvent::ItemEnchanted { fact }
                            | DomainEvent::ItemDestroyed { fact }
                            | DomainEvent::ItemRefined { fact }
                            | DomainEvent::ItemRefineCancelled { fact }
                            | DomainEvent::ItemExchanged { fact } => &mut fact.metadata,
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
            if let Some(work) = &player.items {
                player
                    .events
                    .extend(item_events(&cp, work, player.events.len())?);
            }
            if immediate || tick.tick.0.saturating_sub(player.last_saved.0) >= 50 {
                due.insert(entity.id);
            }
        }
        let wrote = due
            .iter()
            .any(|id| self.players.get(id).is_some_and(|p| p.dirty));
        self.save_due(&due, tick.tick).await?;
        self.wrote_since_index |= wrote;
        self.players.retain(|id, _| {
            let present = snapshot.entities.iter().any(|e| e.id == *id);
            if !present {
                self.sessions.remove(id);
            }
            present
        });
        self.last = Some((tick.epoch, tick.tick));
        let index_started = std::time::Instant::now();
        if self.wrote_since_index {
            self.index_tick(snapshot.seed.zone, tick.epoch, tick.tick, true)
                .await;
            self.wrote_since_index = false;
        }
        super::zone_actor::runtime_phase("checkpoint_index", index_started);
        self.refresh_if_due(snapshot).await;
        Ok(())
    }

    async fn save_due(&mut self, due: &BTreeSet<EntityId>, tick: Tick) -> anyhow::Result<()> {
        // Account-scoped receipts and outbox facts retain entity order. Independent resource
        // checkpoints may overlap, but the whole batch still gates index completion/output.
        let concurrency = if due.iter().any(|id| {
            self.players
                .get(id)
                .is_some_and(|p| !p.events.is_empty() || p.learning.is_some() || p.items.is_some())
        }) {
            1
        } else {
            8
        };
        let repo = self.repo.as_ref();
        let learning = self.learning_repo.as_deref();
        let items = self.item_repo.as_deref();
        let audit = self.audit.as_ref();
        let metrics = self.metrics.as_ref();
        let sessions = &self.sessions;
        let pending = &self.pending;
        let mut saves = Vec::new();
        for (id, player) in &mut self.players {
            if !due.contains(id) {
                continue;
            }
            let session = sessions.get(id).copied();
            saves.push(async move {
                save(repo, learning, items, audit, metrics, pending, session, player).await?;
                player.last_saved = tick;
                Ok(())
            });
        }
        let mut saves = futures_util::stream::iter(saves).buffered(concurrency);
        if concurrency == 1 {
            while let Some(result) = saves.next().await {
                result?;
            }
            return Ok(());
        }
        // Settle every started transaction even on failure; never cancel an in-flight commit.
        let results: Vec<anyhow::Result<()>> = saves.collect().await;
        results.into_iter().collect()
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
        let mut state = super::replay_log::restore_snapshot(stored.snapshot)?;
        self.recover_pending(false).await?;
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
        // A baseline can itself carry pending critical facts, even without a later record;
        // under the internal-state cadence every retained Phase 3 lane is settled here.
        self.recover_pending(true).await?;
        // Ordinary progress may roll back; critical facts have already been checkpointed.
        self.players.clear();
        self.sessions.clear();
        self.last = None;
        // Historical policy belongs only to the settled epoch, never its successor.
        self.internal_cadence = true;
        self.wrote_since_index = false;
        Ok(())
    }
}

/// A recovered epoch has no live actor. Verify its complete original prefix before
/// durably closing it; the index closes only after this acknowledgement. Recovery
/// snapshots accelerate DB settlement but cannot authorize a truncated replay watermark.
async fn close_recovered_epoch(log: &dyn EventLog, zone: ZoneId, epoch: u64) -> anyhow::Result<()> {
    use super::replay_log::{EpochStatus, Watermark, WatermarkReason};
    if matches!(log.epoch_status(zone, epoch).await?, EpochStatus::Complete(_)) {
        return Ok(());
    }
    let original = log
        .read_snapshot(zone, epoch)
        .await?
        .ok_or_else(|| anyhow::anyhow!("epoch-start snapshot missing during recovery closure"))?;
    let mut state = super::replay_log::restore_snapshot(original.snapshot)?;
    let mut records = log.read_epoch(zone, epoch).await?;
    let mut count = 0_u64;
    let mut last_tick = None;
    while let Some(record) = records.next().await {
        let record = record?;
        anyhow::ensure!(
            record.zone == zone && record.epoch == epoch,
            "recovery closure crossed epochs"
        );
        let applied = state.run_tick(AppliedTickDraft {
            epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        })?;
        anyhow::ensure!(
            record.reproduced_by(&AppliedTickRecord::from_applied(zone, &applied)),
            "recovery closure diverged at tick {}",
            record.tick.0
        );
        count = count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("recovery record count overflow"))?;
        last_tick = Some(record.tick);
    }
    log.write_watermark(&Watermark {
        zone,
        epoch,
        last_tick,
        records: count,
        reason: WatermarkReason::EpochEnd,
    })
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Explicit writers/audit/metrics and the shared pending set.
async fn save(
    repo: &dyn CharacterRepository,
    learning_repo: Option<&dyn super::ports::LearningCheckpointRepository>,
    item_repo: Option<&dyn super::ports::ItemCheckpointRepository>,
    audit: &dyn SessionAudit,
    metrics: &dyn CheckpointMetrics,
    pending: &parking_lot::Mutex<BTreeMap<IdempotencyKey, Instant>>,
    session: Option<SessionId>,
    player: &mut Player,
) -> anyhow::Result<()> {
    anyhow::ensure!(!player.fenced, "checkpoint lane is fenced");
    if !player.dirty {
        return Ok(());
    }
    anyhow::ensure!(
        player.learning.is_some()
            || !player
                .events
                .iter()
                .any(|event| matches!(event, DomainEvent::CharacterSkillLearned { .. })),
        "learning fact requires its typed receipt; recover from durable tick prefix"
    );
    let cp = player
        .latest
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("dirty checkpoint has no request"))?;
    let started = Instant::now();
    let tracked = PendingSave {
        metrics,
        pending,
        key: cp.idempotency.1,
    };
    pending.lock().insert(tracked.key, started);
    let metrics: &dyn CheckpointMetrics = &tracked;
    let mut backoff = Duration::from_millis(100);
    loop {
        metrics.lag(started.elapsed());
        let attempt = async {
            if let Some(work) = &player.items {
                let writer = item_repo.ok_or_else(|| {
                    super::ports::CheckpointError::Constraint("item_repository_missing".into())
                })?;
                writer
                    .checkpoint_items(cp, &player.events, work, player.learning.as_ref())
                    .await
            } else if let Some(receipt) = &player.learning {
                let repo = learning_repo.ok_or_else(|| {
                    super::ports::CheckpointError::Constraint("learning_repository_missing".into())
                })?;
                match repo
                    .checkpoint_learning(cp, &player.events, receipt)
                    .await?
                {
                    super::ports::LearningCheckpointOutcome::Checkpoint(outcome) => Ok(outcome),
                    // A different live writer may have won after our speculative purchase.
                    // Its receipt is safe for preflight, but cannot acknowledge this draft.
                    super::ports::LearningCheckpointOutcome::ReceiptReplayed(_) => {
                        Ok(CheckpointOutcome::Stale)
                    },
                }
            } else {
                repo.checkpoint(cp, &player.events).await
            }
        };
        match observe_attempt(attempt, metrics, started).await {
            Ok(
                outcome @ (CheckpointOutcome::Applied(revision)
                | CheckpointOutcome::Replayed(revision)),
            ) => {
                if matches!(outcome, CheckpointOutcome::Applied(_)) {
                    for event in &player.events {
                        if matches!(event, DomainEvent::CharacterSkillLearned { .. }) {
                            metrics.skill_learned();
                        }
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
                player.learning = None;
                player.items = None;
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

// Identities stay internal; the exported gauge is the oldest pending age, without labels.
struct PendingSave<'a> {
    metrics: &'a dyn CheckpointMetrics,
    pending: &'a parking_lot::Mutex<BTreeMap<IdempotencyKey, Instant>>,
    key: IdempotencyKey,
}
impl PendingSave<'_> {
    fn publish(&self, remove: bool) {
        let mut pending = self.pending.lock();
        if remove {
            pending.remove(&self.key);
        }
        self.metrics.lag(
            pending
                .values()
                .map(Instant::elapsed)
                .max()
                .unwrap_or_default(),
        );
    }
}
impl Drop for PendingSave<'_> {
    fn drop(&mut self) {
        self.publish(true);
    }
}
impl CheckpointMetrics for PendingSave<'_> {
    fn failed(&self) {
        self.metrics.failed();
    }
    fn lag(&self, _: Duration) {
        self.publish(false);
    }
    fn skill_learned(&self) {
        self.metrics.skill_learned();
    }
    fn token_granted(&self, tier: u8, source: crate::domain::character_progression::TokenSource) {
        self.metrics.token_granted(tier, source);
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

// Only externally durable facts gate output; internal retained state stays dirty until cadence.
fn critical_phase3_changed(before: Option<&Phase3Entity>, after: Option<&Phase3Entity>) -> bool {
    let facts = |side: &Phase3Entity| {
        side.player.as_ref().map(|p| {
            (
                p.pvp.karma,
                p.pvp.pk_count,
                p.pvp.pvp_count,
                p.death_penalty_level,
                p.last_death.clone(),
            )
        })
    };
    match (before, after) {
        (Some(a), Some(b)) => facts(a) != facts(b),
        (None, None) => false,
        _ => true,
    }
}

// Compare canonical retained mutations, never a projection's naturally decreasing remainder.
// Casts, offers and regen deadlines are not persisted across a real disconnect.
fn retained_phase3_changed(before: Option<&Phase3Entity>, after: Option<&Phase3Entity>) -> bool {
    match (before, after) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            a.reuse != b.reuse
                || a.effects != b.effects
                || a.modifier_install_order != b.modifier_install_order
                || a.fractions != b.fractions
                || a.player
                    .as_ref()
                    .map(|p| (&p.pvp, p.death_penalty_level, &p.last_death))
                    != b.player
                        .as_ref()
                        .map(|p| (&p.pvp, p.death_penalty_level, &p.last_death))
        },
        (None, Some(_)) | (Some(_), None) => true,
    }
}

#[cfg(test)]
#[path = "checkpoint_phase3_tests.rs"]
mod phase3_tests;

#[cfg(test)]
#[path = "checkpoint_items_tests.rs"]
pub(crate) mod item_tests;

#[allow(clippy::manual_let_else)] // Exhaustive item/non-item metadata match.
fn item_events(
    cp: &CharacterCheckpoint,
    work: &super::ports::ItemCheckpointWork,
    offset: usize,
) -> anyhow::Result<Vec<DomainEvent>> {
    use crate::domain::zone::phase4_projection::ItemOperation;
    let mut facts = Vec::new();
    for (position, delta) in work.projection.ledger.iter().enumerate() {
        // D49: enchant attempts have their own captured audit, including unchanged results;
        // D95: so do Refine/CancelRefine, whose target rows are always retained.
        if (delta.operation == ItemOperation::Enchant && delta.after.is_some())
            || matches!(delta.operation, ItemOperation::Refine | ItemOperation::CancelRefine)
        {
            continue;
        }
        let fact = crate::domain::ItemEventFact {
            metadata: EventMetadata {
                event_id: Uuid::nil(),
                sequence: (0, 0),
            },
            character_id: cp.character_id,
            instance_id: delta.instance,
            template_id: delta.template()?,
            count_delta: delta.count_delta,
            before: delta.before.clone(),
            after: delta.after.clone(),
            causes: delta
                .causes
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()?,
        };
        let event = match delta.operation {
            ItemOperation::Grant | ItemOperation::Starter | ItemOperation::Backfill => {
                DomainEvent::ItemGranted { fact }
            },
            ItemOperation::Equip => DomainEvent::ItemEquipped { fact },
            ItemOperation::Unequip => DomainEvent::ItemUnequipped { fact },
            ItemOperation::Consume | ItemOperation::Use => DomainEvent::ItemConsumed { fact },
            ItemOperation::Enchant | ItemOperation::Destroy | ItemOperation::Crystallize => {
                DomainEvent::ItemDestroyed { fact }
            },
            ItemOperation::Refine | ItemOperation::CancelRefine => {
                anyhow::bail!("refine audit must come from its captured operation")
            },
            ItemOperation::Exchange => {
                anyhow::bail!("exchange audit must come from its captured operation")
            },
            ItemOperation::ToggleAutoShot => {
                anyhow::bail!("pure owner toggle cannot fabricate an item audit row")
            },
        };
        facts.push(((position, 1_u8, delta.operation_ordinal), event));
    }
    facts.extend(captured_item_events(cp, work)?);
    // Canonical ledger row order (D66/D69/D76), not operation_ordinal (a per-instance position).
    facts.sort_by_key(|(key, _)| *key);
    facts
        .into_iter()
        .enumerate()
        .map(|(index, (_, mut event))| {
            let ordinal = u64::try_from(
                offset
                    .checked_add(index)
                    .ok_or_else(|| anyhow::anyhow!("item event ordinal overflow"))?,
            )?;
            let fact = match &mut event {
                DomainEvent::ItemGranted { fact }
                | DomainEvent::ItemEquipped { fact }
                | DomainEvent::ItemUnequipped { fact }
                | DomainEvent::ItemConsumed { fact }
                | DomainEvent::ItemEnchanted { fact }
                | DomainEvent::ItemDestroyed { fact }
                | DomainEvent::ItemRefined { fact }
                | DomainEvent::ItemRefineCancelled { fact }
                | DomainEvent::ItemExchanged { fact } => fact,
                DomainEvent::CharacterCreated { .. }
                | DomainEvent::CharacterLeveled { .. }
                | DomainEvent::CharacterClassChanged { .. }
                | DomainEvent::CharacterSkillLearned { .. }
                | DomainEvent::CharacterDied { .. }
                | DomainEvent::CharacterTokenGranted { .. } => {
                    anyhow::bail!("non-item audit event")
                },
            };
            fact.metadata = EventMetadata {
                event_id: stable_id(
                    work.projection.entity,
                    work.zone,
                    work.epoch,
                    work.projection.tick,
                    Some(ordinal),
                ),
                sequence: (
                    cp.revision_seen
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("checkpoint revision overflow"))?,
                    ordinal,
                ),
            };
            fact.validate()?;
            Ok(event)
        })
        .collect()
}

fn captured_item_events(
    cp: &CharacterCheckpoint,
    work: &super::ports::ItemCheckpointWork,
) -> anyhow::Result<Vec<((usize, u8, u32), DomainEvent)>> {
    // D78: a captured enchant/exchange audit precedes the attempt's OWN ledger row (the
    // target's row of that operation carrying its receipt), so earlier same-tick rows on the
    // target (Equip, then Enchant) stay first. Without such a row it precedes the target's
    // first row carrying the receipt, else the target's first row of a later command (its
    // receipt or any Consume aggregate contributor); with none of these (an unchanged attempt
    // and nothing later on the target) it follows every row, in operation order.
    use crate::domain::zone::phase4_projection::{ItemOperation, ItemReceiptRef};
    let ledger = &work.projection.ledger;
    let command = |receipt: ItemReceiptRef| {
        work.projection
            .receipts
            .iter()
            .position(|fact| fact.reference() == receipt)
    };
    let position = |target: crate::domain::zone::ItemInstanceId,
                    receipt: ItemReceiptRef,
                    own: ItemOperation| {
        let carries = |delta: &crate::domain::zone::phase4_projection::ItemDeltaPlan| {
            delta.instance == target
                && (delta.receipt == Some(receipt) || delta.receipt_refs.contains(&receipt))
        };
        ledger
            .iter()
            .position(|delta| carries(delta) && delta.operation == own)
            .or_else(|| ledger.iter().position(carries))
            .or_else(|| {
                let mine = command(receipt)?;
                ledger.iter().position(|delta| {
                    delta.instance == target
                        && delta
                            .receipt
                            .iter()
                            .chain(&delta.receipt_refs)
                            .filter_map(|receipt| command(*receipt))
                            .any(|later| later > mine)
                })
            })
            .unwrap_or(ledger.len())
    };
    let mut facts = Vec::new();
    for attempt in &work.projection.enchant_attempts {
        facts.push((
            (
                position(attempt.target, attempt.receipt, ItemOperation::Enchant),
                0,
                attempt.operation_ordinal,
            ),
            DomainEvent::ItemEnchanted {
                fact: crate::domain::ItemEventFact {
                    metadata: EventMetadata {
                        event_id: Uuid::nil(),
                        sequence: (0, 0),
                    },
                    character_id: cp.character_id,
                    instance_id: attempt.target,
                    template_id: attempt.template,
                    count_delta: if attempt.after.is_none() { -1 } else { 0 },
                    before: Some(attempt.before.clone()),
                    after: attempt.after.clone(),
                    causes: vec![serde_json::to_value(attempt)?],
                },
            },
        ));
    }
    for refine in &work.projection.refines {
        let applied =
            refine.outcome == crate::domain::zone::phase4_projection::ItemRefineOutcome::Applied;
        let fact = crate::domain::ItemEventFact {
            metadata: EventMetadata {
                event_id: Uuid::nil(),
                sequence: (0, 0),
            },
            character_id: cp.character_id,
            instance_id: refine.after.id,
            template_id: refine.before.template_id,
            count_delta: 0,
            before: Some(refine.before.clone()),
            after: Some(refine.after.clone()),
            causes: vec![serde_json::to_value(refine)?],
        };
        facts.push((
            (
                position(refine.after.id, refine.receipt, refine.receipt.operation),
                0,
                refine.operation_ordinal,
            ),
            if applied {
                DomainEvent::ItemRefined { fact }
            } else {
                DomainEvent::ItemRefineCancelled { fact }
            },
        ));
    }
    for exchange in &work.projection.exchanges {
        facts.push((
            (
                position(exchange.before.id, exchange.receipt, ItemOperation::Consume),
                0,
                exchange.operation_ordinal,
            ),
            DomainEvent::ItemExchanged {
                fact: crate::domain::ItemEventFact {
                    metadata: EventMetadata {
                        event_id: Uuid::nil(),
                        sequence: (0, 0),
                    },
                    character_id: cp.character_id,
                    instance_id: exchange.before.id,
                    template_id: exchange.before.template_id,
                    count_delta: -1,
                    before: Some(exchange.before.clone()),
                    after: None,
                    causes: vec![serde_json::to_value(exchange)?],
                },
            },
        ));
    }
    Ok(facts)
}

#[path = "checkpoint_paused.rs"]
pub(crate) mod paused;
