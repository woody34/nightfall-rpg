//! The zone's authoritative state and its one transition, [`ZoneState::run_tick`]: apply this
//! tick's commands in ordinal order, expire corpses and respawn due slot members, run the NPC
//! AI, chase, advance movement, land due swings, start the next ones, then diff every player's
//! area of interest into its ordered output stream.

use std::collections::BTreeMap;
use std::sync::Arc;

use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::ai::{MemberState, SlotMember, SpawnSlotSpec};
use super::aoi::{AoiCell, AoiIndex, CellCoord};
use super::combat::{
    weapon_reach, CombatRole, CombatState, HateLedger, NpcCombat, PlayerLoad, SwingCancel,
};
use super::command::{
    AppliedCommand, AppliedTick, AppliedTickDraft, CommandSource, DeathFact, Disposition,
    ObserverOutput, Ordinal, RejectReason, SessionGeneration, ZoneCommand, ZoneEvent, ZoneInput,
};
use super::entity::{Entity, EntityId, EntityKind, Tick};
use super::fixed::{Fixed, Speed, Vec2Fixed};
use super::progression::{level_for_xp, xp_cap};
use super::stat_rules::{StatRules, StatRulesParts};
use super::stat_sheet::StatSheet;

/// Largest distance one `MoveTo` may cover, in tiles. Longer trips are several commands, which
/// bounds the work a single malicious intent can cause and keeps paths inside the AOI.
pub const MAX_MOVE_DISTANCE_TILES: i32 = 64;

/// Version of the [`ZoneSnapshot`] layout. Bump on any change to the snapshot or to the
/// meaning of a field; `from_snapshot` accepts 4 and 5. 2: combat state, hate
/// ledgers and the stat rules (Phase 1 E2.2). 3: NPC AI blocks, spawn slots and the respawn
/// scheduler (Phase 1 E3.2–E3.4). 4: safe point and the player's `alive` load flag (E2.4).
/// 5: application checkpoint lanes, excluded from the simulation digest.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 5;

/// Identity of a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZoneId(pub u32);

/// Everything the zone RNG is derived from: the zone and its epoch (plan D7, §8 #5). Two
/// zones, or two epochs of one zone, never share a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ZoneSeed {
    /// The zone.
    pub zone: ZoneId,
    /// Epoch: a new one every time the zone actor starts.
    pub epoch: u64,
}

impl ZoneSeed {
    /// Domain-separation tag so this key cannot collide with any other `ChaCha` use.
    const TAG: &'static [u8; 20] = b"nightfall.zone.rng.1";

    /// The 256-bit `ChaCha` key: tag (20 bytes) || zone id LE (4) || epoch LE (8). Explicit
    /// byte layout, so the key is identical on every platform and build.
    #[must_use]
    pub fn key(self) -> [u8; 32] {
        let mut key = [0_u8; 32];
        let (tag, rest) = key.split_at_mut(Self::TAG.len());
        tag.copy_from_slice(Self::TAG);
        let (zone, epoch) = rest.split_at_mut(4);
        zone.copy_from_slice(&self.zone.0.to_le_bytes());
        epoch.copy_from_slice(&self.epoch.to_le_bytes());
        key
    }
}

/// The complete state of the zone's `ChaCha12` generator: key, stream and position. Restoring
/// these three continues the stream exactly where it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RngState {
    /// 256-bit key (derived from [`ZoneSeed::key`]).
    pub key: [u8; 32],
    /// Stream id.
    pub stream: u64,
    /// Position in the stream, in 32-bit words.
    pub word_pos: u128,
}

impl RngState {
    fn capture(rng: &ChaCha12Rng) -> Self {
        Self {
            key: rng.get_seed(),
            stream: rng.get_stream(),
            word_pos: rng.get_word_pos(),
        }
    }

    fn restore(self) -> ChaCha12Rng {
        let mut rng = ChaCha12Rng::from_seed(self.key);
        rng.set_stream(self.stream);
        rng.set_word_pos(self.word_pos);
        rng
    }
}

/// The walkable rectangle of a zone, inclusive on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneBounds {
    min: Vec2Fixed,
    max: Vec2Fixed,
}

/// `ZoneBounds::new` was given a min that is not below-left of max.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("zone bounds min must be <= max on both axes")]
pub struct InvalidBounds;

impl ZoneBounds {
    /// Validated constructor.
    pub fn new(min: Vec2Fixed, max: Vec2Fixed) -> Result<Self, InvalidBounds> {
        if min.x <= max.x && min.y <= max.y {
            Ok(Self { min, max })
        } else {
            Err(InvalidBounds)
        }
    }

    /// Lower-left corner.
    #[must_use]
    pub const fn min(&self) -> Vec2Fixed {
        self.min
    }

    /// Upper-right corner.
    #[must_use]
    pub const fn max(&self) -> Vec2Fixed {
        self.max
    }

    /// Inclusive containment.
    #[must_use]
    pub fn contains(&self, p: Vec2Fixed) -> bool {
        self.min.x <= p.x && p.x <= self.max.x && self.min.y <= p.y && p.y <= self.max.y
    }
}

/// Provenance of a snapshot. Placeholders until the build pipeline and config loader fill
/// them; replay compares them and warns on mismatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// [`SNAPSHOT_SCHEMA_VERSION`] at the time of writing.
    pub schema_version: u32,
    /// Server build that wrote it. Crate version until CI stamps a commit id.
    pub build_id: String,
    /// Hash of the zone configuration (bounds, spawn tables) the epoch ran with.
    pub config_hash: String,
    /// Hash of the stat rule files the epoch's rules came from (`rules` holds the rules
    /// themselves); empty for a zone without rules.
    #[serde(default)]
    pub rules_hash: String,
    /// `JetStream` sequence of the epoch's first applied-tick record. `None` in the epoch-start
    /// snapshot, which is published before that record exists; the `zone_snapshots` row
    /// carries it (`jetstream_first_seq`) once the first record is acknowledged.
    pub first_log_seq: Option<u64>,
}

impl Default for SnapshotMeta {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            build_id: env!("CARGO_PKG_VERSION").to_owned(),
            config_hash: String::new(),
            rules_hash: String::new(),
            first_log_seq: None,
        }
    }
}

/// A zone's complete state at a tick boundary (plan §8 #5). Rebuilding from it with
/// [`ZoneState::from_snapshot`] and feeding the same drafts yields byte-identical
/// [`AppliedTick`]s, which replay (Story 3.3) relies on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneSnapshot {
    /// Persistence lanes, filled by the actor at an admitted boundary. Schema 4 lacks them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<CheckpointSnapshot>,
    /// Provenance.
    pub meta: SnapshotMeta,
    /// Zone and epoch.
    pub seed: ZoneSeed,
    /// The next tick the zone will run.
    pub tick: Tick,
    /// The ordinal the next command will get.
    pub next_ordinal: Ordinal,
    /// Wall-clock time of tick 0, in Unix ms. `server_time_ms` is derived from it.
    pub time_origin_ms: i64,
    /// Generator state.
    pub rng: RngState,
    /// Walkable area.
    pub bounds: ZoneBounds,
    /// Every entity with full movement state, in id order.
    pub entities: Vec<Entity>,
    /// The AOI index. Derivable from `entities`, included so a reader need not re-derive it;
    /// `from_snapshot` checks the two agree.
    pub aoi: Vec<AoiCell>,
    /// The immutable stat rules the zone simulates with, so a restore never consults the
    /// current data files (plan §3.2). `None` for a zone without combat.
    pub rules: Option<StatRulesParts>,
    /// Every NPC's hate ledger, in NPC id order.
    pub hate: Vec<NpcHate>,
    /// The immutable spawn slots, with resolved combat and AI profiles (E3.4).
    #[serde(default)]
    pub spawn_slots: Vec<SpawnSlotSpec>,
    /// The respawn scheduler: every slot member, in (slot, member) order.
    #[serde(default)]
    pub spawn_members: Vec<MemberState>,
    /// Where dead players respawn (E2.4); `None` respawns them where they fell.
    pub safe_point: Option<Vec2Fixed>,
}

/// Persistence metadata owned by the application; it never participates in simulation hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    /// Player identity.
    pub entity: EntityId,
    /// Last acknowledged repository revision.
    pub revision: u64,
    /// Tick of the last cadence save.
    pub last_saved: Tick,
    /// Unsaved progression or resources.
    pub dirty: bool,
    /// A newer writer has fenced this lane.
    pub fenced: bool,
    /// Facts awaiting the next transaction.
    pub events: Vec<crate::domain::DomainEvent>,
    /// Exact latest checkpoint request, if a tick has supplied one.
    pub latest: Option<CheckpointRequestSnapshot>,
}

/// Integer-only representation of an application checkpoint request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRequestSnapshot {
    /// Revision used by the request (may precede an acknowledged lane revision).
    pub revision_seen: u64,
    /// Level.
    pub level: u32,
    /// Total experience.
    pub xp: u64,
    /// Current health.
    pub hp: u32,
    /// Current mana.
    pub mp: u32,
    /// Whether the character is alive.
    pub alive: bool,
    /// IEEE wire-position bits retained losslessly; the simulation never interprets them.
    pub position_bits: [u32; 2],
    /// Stable idempotency key.
    pub key: uuid::Uuid,
}

/// One NPC's hate ledger in a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcHate {
    /// The NPC.
    pub npc: EntityId,
    /// Its ledger.
    pub ledger: HateLedger,
}

/// A snapshot that cannot describe a valid zone.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SnapshotError {
    /// Written by an incompatible layout.
    #[error("snapshot schema {0} is not supported")]
    Schema(u32),
    /// The RNG key does not belong to the seed.
    #[error("rng key does not match the zone seed")]
    SeedMismatch,
    /// Two entities share an id.
    #[error("duplicate entity {0}")]
    DuplicateEntity(EntityId),
    /// An entity stands outside the bounds.
    #[error("entity {0} is out of bounds")]
    OutOfBounds(EntityId),
    /// The stored AOI index disagrees with the entity positions.
    #[error("aoi index does not match entity positions")]
    AoiMismatch,
    /// The stored rules fail validation.
    #[error("snapshot rules are invalid: {0}")]
    Rules(String),
    /// A hate ledger names an NPC that is not a living-or-dead combat NPC of the snapshot, or
    /// a combatant exists in a zone without rules.
    #[error("combat state does not match the snapshot's entities or rules")]
    CombatMismatch,
    /// The spawn scheduler does not cover exactly the slots' members, or an AI NPC's slot
    /// member does not name it.
    #[error("spawn scheduler does not match the spawn slots or entities")]
    SpawnMismatch,
    /// The safe point lies outside the bounds.
    #[error("safe point is out of bounds")]
    SafePointOutOfBounds,
}

/// A draft that does not continue this zone: replay has diverged or the log has a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TickError {
    /// The draft is for another epoch.
    #[error("draft epoch {got} but zone epoch is {expected}")]
    Epoch {
        /// Zone epoch.
        expected: u64,
        /// Draft epoch.
        got: u64,
    },
    /// The draft is not for the next tick.
    #[error("draft for tick {got} but next tick is {expected}")]
    Tick {
        /// Next tick.
        expected: Tick,
        /// Draft tick.
        got: Tick,
    },
    /// Ordinals are not consecutive from the zone's next ordinal.
    #[error("command ordinal {got:?} where {expected:?} was expected")]
    Ordinal {
        /// Expected ordinal.
        expected: Ordinal,
        /// Ordinal found.
        got: Ordinal,
    },
}

/// One zone's authoritative simulation state. Single owner (the zone actor); no interior
/// mutability, no locks, no clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneState {
    seed: ZoneSeed,
    checkpoints: Vec<CheckpointSnapshot>,
    meta: SnapshotMeta,
    rng: ChaCha12Rng,
    bounds: ZoneBounds,
    time_origin_ms: i64,
    next_tick: Tick,
    next_ordinal: Ordinal,
    entities: BTreeMap<EntityId, Entity>,
    aoi: AoiIndex,
    /// Per player: the entities its session has been told about, sorted. At every tick
    /// boundary this equals the player's AOI, so it is derived on restore rather than
    /// snapshotted.
    known: BTreeMap<EntityId, Vec<EntityId>>,
    /// The stat rules combat reads (injected at bootstrap, restored from snapshots).
    rules: Option<Arc<StatRules>>,
    /// Per NPC: who it hates (E2.5).
    hate: BTreeMap<EntityId, HateLedger>,
    /// Immutable spawn slots (E3.4).
    slots: Vec<SpawnSlotSpec>,
    /// Respawn scheduler: one record per slot member.
    members: BTreeMap<SlotMember, MemberState>,
    /// Town respawn point (E2.4), part of the immutable zone configuration.
    safe_point: Option<Vec2Fixed>,
    /// Per player: progression transitions of the tick in progress (E2.6). Always empty at a
    /// tick boundary (flushed into `Progression` events), so never snapshotted.
    progress: BTreeMap<EntityId, ProgressNote>,
}

/// What one player's progression did so far this tick; see [`ZoneState::note_progress`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProgressNote {
    level_before: u32,
    xp_before: u64,
    xp_gained: u64,
    levels_gained: Vec<u32>,
    died: Option<DeathFact>,
    respawned: bool,
}

impl ZoneState {
    /// An empty zone at tick 0, ordinal 0, with the RNG at the start of its stream.
    /// `time_origin_ms` is the wall-clock time the caller assigns to tick 0.
    #[must_use]
    pub fn new(seed: ZoneSeed, bounds: ZoneBounds, time_origin_ms: i64) -> Self {
        Self {
            seed,
            checkpoints: Vec::new(),
            meta: SnapshotMeta::default(),
            rng: ChaCha12Rng::from_seed(seed.key()),
            bounds,
            time_origin_ms,
            next_tick: Tick(0),
            next_ordinal: Ordinal(0),
            entities: BTreeMap::new(),
            aoi: AoiIndex::default(),
            known: BTreeMap::new(),
            rules: None,
            hate: BTreeMap::new(),
            slots: Vec::new(),
            members: BTreeMap::new(),
            safe_point: None,
            progress: BTreeMap::new(),
        }
    }

    /// Where dead players respawn (the zone file's `[safe_point]`); must be inside the bounds.
    #[must_use]
    pub fn with_safe_point(mut self, point: Vec2Fixed) -> Self {
        self.safe_point = self.bounds.contains(point).then_some(point);
        self
    }

    /// The town respawn point, if configured.
    #[must_use]
    pub const fn safe_point(&self) -> Option<Vec2Fixed> {
        self.safe_point
    }

    /// The zone with combat: players spawned with a [`super::PlayerLoad`] get stats from
    /// `rules`, and every swing reads its constants.
    #[must_use]
    pub fn with_rules(mut self, rules: Arc<StatRules>) -> Self {
        self.rules = Some(rules);
        self
    }

    /// The injected rules, if any.
    #[must_use]
    pub fn rules(&self) -> Option<&StatRules> {
        self.rules.as_deref()
    }

    /// One NPC's hate ledger.
    #[must_use]
    pub fn hate_ledger(&self, npc: EntityId) -> Option<&HateLedger> {
        self.hate.get(&npc)
    }

    /// Rebuilds a zone from a snapshot, validating it.
    pub fn from_snapshot(snapshot: ZoneSnapshot) -> Result<Self, SnapshotError> {
        if ![4, SNAPSHOT_SCHEMA_VERSION].contains(&snapshot.meta.schema_version) {
            return Err(SnapshotError::Schema(snapshot.meta.schema_version));
        }
        if snapshot.rng.key != snapshot.seed.key() {
            return Err(SnapshotError::SeedMismatch);
        }
        let mut state = Self::new(snapshot.seed, snapshot.bounds, snapshot.time_origin_ms);
        if let Some(parts) = snapshot.rules {
            let rules = StatRules::new(parts).map_err(|violations| {
                SnapshotError::Rules(
                    violations
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;
            state.rules = Some(Arc::new(rules));
        }
        if snapshot
            .safe_point
            .is_some_and(|p| !state.bounds.contains(p))
        {
            return Err(SnapshotError::SafePointOutOfBounds);
        }
        state.meta = snapshot.meta;
        state.meta.schema_version = SNAPSHOT_SCHEMA_VERSION;
        state.checkpoints = snapshot.checkpoints;
        state.safe_point = snapshot.safe_point;
        state.rng = snapshot.rng.restore();
        state.next_tick = snapshot.tick;
        state.next_ordinal = snapshot.next_ordinal;
        for e in snapshot.entities {
            if !state.bounds.contains(e.pos) {
                return Err(SnapshotError::OutOfBounds(e.id));
            }
            if state.entities.contains_key(&e.id) {
                return Err(SnapshotError::DuplicateEntity(e.id));
            }
            if e.combat.is_some() && state.rules.is_none() {
                return Err(SnapshotError::CombatMismatch);
            }
            state.aoi.insert(e.id, e.pos);
            state.entities.insert(e.id, e);
        }
        if state.aoi.to_cells() != snapshot.aoi {
            return Err(SnapshotError::AoiMismatch);
        }
        for NpcHate { npc, ledger } in snapshot.hate {
            let owner = state.entities.get(&npc);
            if owner.is_none_or(|e| e.kind != EntityKind::Npc || e.combat.is_none())
                || state.hate.insert(npc, ledger).is_some()
            {
                return Err(SnapshotError::CombatMismatch);
            }
        }
        state.restore_spawns(snapshot.spawn_slots, snapshot.spawn_members)?;
        let players: Vec<(EntityId, Vec2Fixed)> = state
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::Player)
            .map(|e| (e.id, e.pos))
            .collect();
        for (id, pos) in players {
            let mut ids: Vec<EntityId> = state.aoi.in_aoi(pos).collect();
            ids.sort_unstable();
            state.known.insert(id, ids);
        }
        Ok(state)
    }

    /// The state as plain data. Only meaningful at a tick boundary, which is the only time
    /// the owner can call it.
    #[must_use]
    pub fn snapshot(&self) -> ZoneSnapshot {
        ZoneSnapshot {
            checkpoints: self.checkpoints.clone(),
            meta: self.meta.clone(),
            seed: self.seed,
            tick: self.next_tick,
            next_ordinal: self.next_ordinal,
            time_origin_ms: self.time_origin_ms,
            rng: RngState::capture(&self.rng),
            bounds: self.bounds,
            entities: self.entities.values().cloned().collect(),
            aoi: self.aoi.to_cells(),
            rules: self.rules.as_ref().map(|r| r.parts().clone()),
            hate: self
                .hate
                .iter()
                .map(|(npc, ledger)| NpcHate {
                    npc: *npc,
                    ledger: ledger.clone(),
                })
                .collect(),
            spawn_slots: self.slots.clone(),
            spawn_members: self.members.values().copied().collect(),
            safe_point: self.safe_point,
        }
    }

    /// Validates and installs a snapshot's spawn slots and scheduler: exactly one record per
    /// slot member, and every AI NPC is the entity its member names.
    fn restore_spawns(
        &mut self,
        slots: Vec<SpawnSlotSpec>,
        members: Vec<MemberState>,
    ) -> Result<(), SnapshotError> {
        if !slots.is_empty() && self.rules.is_none() {
            return Err(SnapshotError::CombatMismatch);
        }
        let expected: usize = slots.iter().map(|s| usize::from(s.count)).sum();
        for m in members {
            let fits = slots
                .get(usize::from(m.member.slot))
                .is_some_and(|s| m.member.member < s.count);
            if !fits || self.members.insert(m.member, m).is_some() {
                return Err(SnapshotError::SpawnMismatch);
            }
        }
        if self.members.len() != expected {
            return Err(SnapshotError::SpawnMismatch);
        }
        for e in self.entities.values() {
            let Some(ai) = &e.ai else { continue };
            let owned = self
                .members
                .get(&ai.slot)
                .is_some_and(|m| m.entity == Some(e.id) && e.combat.is_some());
            if !owned || e.kind != EntityKind::Npc {
                return Err(SnapshotError::SpawnMismatch);
            }
        }
        self.slots = slots;
        Ok(())
    }

    /// Zone and epoch.
    #[must_use]
    pub const fn seed(&self) -> ZoneSeed {
        self.seed
    }

    /// Walkable area.
    #[must_use]
    pub const fn bounds(&self) -> ZoneBounds {
        self.bounds
    }

    /// The next tick `run_tick` expects.
    #[must_use]
    pub const fn next_tick(&self) -> Tick {
        self.next_tick
    }

    /// The ordinal the next command will get.
    #[must_use]
    pub const fn next_ordinal(&self) -> Ordinal {
        self.next_ordinal
    }

    /// `time_origin_ms + tick * 100`. The only source of wire timestamps.
    #[must_use]
    pub fn server_time_ms(&self, tick: Tick) -> i64 {
        let elapsed = i64::try_from(tick.elapsed_ms()).unwrap_or(i64::MAX);
        self.time_origin_ms.saturating_add(elapsed)
    }

    /// Number of entities.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// One entity.
    #[must_use]
    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// All entities in id order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// The AOI index.
    #[must_use]
    pub const fn aoi(&self) -> &AoiIndex {
        &self.aoi
    }

    /// A uniform roll in `0..1000`, the shape every combat roll takes
    /// (docs/planning/03-combat-and-skills.md §3.2). All zone randomness goes through the
    /// seeded generator; rejection sampling avoids modulo bias.
    pub fn roll_permille(&mut self) -> u16 {
        // < 1000, so the conversion cannot fail.
        u16::try_from(self.roll_below(1000)).unwrap_or(0)
    }

    /// A uniform draw in `0..n` (`n >= 1`) from one or more 32-bit words: words at or above
    /// the largest multiple of `n` not above 2^32 are rejected, so there is no modulo bias.
    /// `n == 1` still consumes a word; callers skip the draw for a zero-width range.
    pub fn roll_below(&mut self, n: u32) -> u32 {
        let n = u64::from(n.max(1));
        let span = 1_u64 << 32;
        let limit = span.saturating_sub(span.checked_rem(n).unwrap_or(0));
        loop {
            let v = u64::from(self.rng.next_u32());
            if v < limit {
                return u32::try_from(v.checked_rem(n).unwrap_or(0)).unwrap_or(0);
            }
        }
    }

    /// Phase one of a tick: assigns consecutive ordinals to `inputs` (in the given order) for
    /// the next tick. Pure: the zone is unchanged until the draft is committed.
    #[must_use]
    pub fn draft(&self, inputs: Vec<ZoneInput>) -> AppliedTickDraft {
        let mut ordinal = self.next_ordinal;
        let commands = inputs
            .into_iter()
            .map(|i| {
                let c = AppliedCommand {
                    ordinal,
                    source: i.source,
                    seq: i.seq,
                    command: i.command,
                };
                ordinal = Ordinal(ordinal.0.saturating_add(1));
                c
            })
            .collect();
        AppliedTickDraft {
            epoch: self.seed.epoch,
            tick: self.next_tick,
            commands,
        }
    }

    /// Phase two: commits a draft. Applies its commands in ordinal order, runs the spawn and
    /// AI phases, advances movement and combat, then diffs every player's AOI. The result is a pure function of the state and the
    /// draft. A draft that does not continue this zone is refused and nothing changes.
    pub fn run_tick(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
        self.check_draft(&draft)?;
        let tick = draft.tick;
        let mut events = Vec::new();
        let mut dispositions = Vec::new();
        for c in &draft.commands {
            match self.apply(tick, c) {
                Ok(evs) => events.extend(evs),
                Err(reason) => dispositions.push(Disposition {
                    ordinal: c.ordinal,
                    source: c.source,
                    seq: c.seq,
                    tick_seen: tick,
                    reason,
                }),
            }
            self.next_ordinal = Ordinal(c.ordinal.0.saturating_add(1));
        }
        self.spawn_phase(tick, &mut events);
        self.ai_phase(tick, &mut events);
        self.chase(tick, &mut events);
        events.extend(self.step(tick));
        let impacts = events.len();
        self.land_impacts(tick, &mut events);
        self.note_hits(tick, events.get(impacts..).unwrap_or_default());
        self.start_swings(tick, &mut events);
        self.flush_progression(tick, &mut events);
        let responses = responses(tick, &draft.commands, &dispositions);
        let outputs = self.observe(tick, responses, &events);
        self.next_tick = tick.next();
        Ok(AppliedTick {
            epoch: draft.epoch,
            tick,
            server_time_ms: self.server_time_ms(tick),
            commands: draft.commands,
            dispositions,
            events,
            outputs,
            state_digest: self.state_digest(),
        })
    }

    /// SHA-256 of the canonical JSON of everything that changes between ticks (the immutable
    /// rules, bounds and seed are fixed by the snapshot): next tick and ordinal, generator
    /// position, entities, hate and the respawn scheduler, all in id order.
    #[must_use]
    pub fn state_digest(&self) -> [u8; 32] {
        #[derive(Serialize)]
        struct View<'a> {
            tick: Tick,
            ordinal: Ordinal,
            rng: RngState,
            entities: Vec<&'a Entity>,
            hate: &'a BTreeMap<EntityId, HateLedger>,
            spawns: Vec<&'a MemberState>,
        }
        let view = View {
            tick: self.next_tick,
            ordinal: self.next_ordinal,
            rng: RngState::capture(&self.rng),
            entities: self.entities.values().collect(),
            hate: &self.hate,
            spawns: self.members.values().collect(),
        };
        // Serialising plain data with ordered maps cannot fail; an empty input would still
        // be deterministic.
        let bytes = serde_json::to_vec(&view).unwrap_or_default();
        Sha256::digest(&bytes).into()
    }

    fn check_draft(&self, draft: &AppliedTickDraft) -> Result<(), TickError> {
        if draft.epoch != self.seed.epoch {
            return Err(TickError::Epoch {
                expected: self.seed.epoch,
                got: draft.epoch,
            });
        }
        if draft.tick != self.next_tick {
            return Err(TickError::Tick {
                expected: self.next_tick,
                got: draft.tick,
            });
        }
        let mut expected = self.next_ordinal;
        for c in &draft.commands {
            if c.ordinal != expected {
                return Err(TickError::Ordinal {
                    expected,
                    got: c.ordinal,
                });
            }
            expected = Ordinal(expected.0.saturating_add(1));
        }
        Ok(())
    }

    /// Checks that `source` may issue `cmd`. Sessions may only steer or remove their own
    /// entity, and only while their generation is current; everything else is system-only.
    fn authorize(&self, source: CommandSource, cmd: &ZoneCommand) -> Result<(), RejectReason> {
        let CommandSource::Session { entity, generation } = source else {
            return Ok(());
        };
        match cmd {
            ZoneCommand::MoveTo { entity: target, .. }
            | ZoneCommand::StopMove { entity: target }
            | ZoneCommand::SetTarget { entity: target, .. }
            | ZoneCommand::Attack { entity: target }
            | ZoneCommand::StopAttack { entity: target }
            | ZoneCommand::Respawn { entity: target }
            | ZoneCommand::Despawn { entity: target }
                if *target == entity =>
            {
                match self.entities.get(&entity) {
                    Some(e) if e.generation != generation => Err(RejectReason::StaleSession),
                    // Unknown entities are handled by the command itself.
                    Some(_) | None => Ok(()),
                }
            },
            ZoneCommand::MoveTo { .. }
            | ZoneCommand::StopMove { .. }
            | ZoneCommand::SetTarget { .. }
            | ZoneCommand::Attack { .. }
            | ZoneCommand::StopAttack { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::AddAggro { .. }
            | ZoneCommand::ReplaceSession { .. } => Err(RejectReason::NotPermitted),
        }
    }

    /// Applies one command. `Err` is a refusal and leaves the zone unchanged.
    fn apply(&mut self, tick: Tick, c: &AppliedCommand) -> Result<Vec<ZoneEvent>, RejectReason> {
        self.authorize(c.source, &c.command)?;
        self.refuse_dead_actor(c.source, &c.command)?;
        match &c.command {
            ZoneCommand::SpawnPlayer {
                entity,
                name,
                pos,
                speed,
                generation,
                load,
            } => {
                let combat = match (&self.rules, load) {
                    (Some(rules), Some(load)) => Some(player_combat(rules, load)?),
                    _ => None,
                };
                let mut events = self.spawn(
                    tick,
                    Spawned {
                        id: *entity,
                        kind: EntityKind::Player,
                        name,
                        pos: *pos,
                        speed: *speed,
                        generation: *generation,
                        combat,
                    },
                )?;
                events.extend(self.owner_state(tick, *entity));
                Ok(events)
            },
            ZoneCommand::SpawnNpc {
                name,
                pos,
                speed,
                combat,
            } => {
                if !self.bounds.contains(*pos) {
                    return Err(RejectReason::OutOfBounds);
                }
                let combat = match (&self.rules, combat) {
                    (Some(_), Some(spec)) => Some(npc_combat(spec)?),
                    (None, Some(_)) => return Err(RejectReason::NotPermitted),
                    (_, None) => None,
                };
                let id = self.random_entity_id();
                self.spawn(
                    tick,
                    Spawned {
                        id,
                        kind: EntityKind::Npc,
                        name,
                        pos: *pos,
                        speed: *speed,
                        generation: SessionGeneration::default(),
                        combat,
                    },
                )
            },
            ZoneCommand::Despawn { entity } => Ok(self.despawn(tick, *entity)),
            ZoneCommand::ReplaceSession { entity, generation } => {
                let e = self
                    .entities
                    .get_mut(entity)
                    .ok_or(RejectReason::UnknownEntity)?;
                if e.kind != EntityKind::Player {
                    return Err(RejectReason::NotAPlayer);
                }
                if *generation <= e.generation {
                    return Err(RejectReason::StaleSession);
                }
                e.generation = *generation;
                // The new session knows nothing yet: the AOI diff resends everything, and the
                // owner-only state follows.
                self.known.remove(entity);
                Ok(self.owner_state(tick, *entity))
            },
            ZoneCommand::SetTarget { entity, target } => self.set_target(tick, *entity, *target),
            ZoneCommand::Attack { entity } => self.attack(tick, *entity),
            ZoneCommand::StopAttack { entity } => self.stop_attack(tick, *entity),
            ZoneCommand::Respawn { entity } => self.respawn_player(tick, *entity),
            ZoneCommand::MoveTo { entity, dest } => self.move_to(tick, *entity, *dest),
            ZoneCommand::StopMove { entity } => {
                let mut events = Vec::new();
                self.disengage(tick, *entity, SwingCancel::Moved, &mut events);
                let e = self
                    .entities
                    .get_mut(entity)
                    .ok_or(RejectReason::UnknownEntity)?;
                if let Some(c) = e.combat.as_mut() {
                    c.chasing = false;
                }
                events.extend(e.dest.take().map(|_| ZoneEvent::EntityMove {
                    tick,
                    entity: *entity,
                    pos: e.pos,
                    dest: None,
                    speed: e.speed,
                }));
                Ok(events)
            },
            ZoneCommand::AddAggro { npc, target } => self.add_aggro(tick, *npc, *target),
        }
    }

    /// A dead actor may not steer itself (plan §3.2). `Respawn` (E2.4) and `Despawn`
    /// (disconnect) stay possible; system commands are not intents.
    fn refuse_dead_actor(
        &self,
        source: CommandSource,
        cmd: &ZoneCommand,
    ) -> Result<(), RejectReason> {
        if source == CommandSource::System {
            return Ok(());
        }
        match cmd {
            ZoneCommand::MoveTo { entity, .. }
            | ZoneCommand::StopMove { entity }
            | ZoneCommand::SetTarget { entity, .. }
            | ZoneCommand::Attack { entity }
            | ZoneCommand::StopAttack { entity } => {
                if self.entities.get(entity).is_some_and(|e| e.targeting.dead) {
                    Err(RejectReason::DeadActor)
                } else {
                    Ok(())
                }
            },
            ZoneCommand::Respawn { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::AddAggro { .. } => Ok(()),
        }
    }

    /// The owner-only state a player's session needs on admission or replacement: its
    /// `StatsChanged` and, if it has one, its selection.
    fn owner_state(&self, tick: Tick, entity: EntityId) -> Vec<ZoneEvent> {
        let Some(e) = self.entities.get(&entity) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if let Some(c) = &e.combat {
            events.push(stats_changed(tick, entity, c));
        }
        if e.targeting.target.is_some() {
            events.push(ZoneEvent::TargetChanged {
                tick,
                entity,
                target: e.targeting.target,
            });
        }
        events
    }

    fn spawn(&mut self, tick: Tick, s: Spawned<'_>) -> Result<Vec<ZoneEvent>, RejectReason> {
        if self.entities.contains_key(&s.id) {
            return Err(RejectReason::AlreadyExists);
        }
        if !self.bounds.contains(s.pos) {
            return Err(RejectReason::OutOfBounds);
        }
        self.aoi.insert(s.id, s.pos);
        let dead = s.combat.as_ref().is_some_and(|c| c.hp == 0);
        let entity = Entity {
            id: s.id,
            kind: s.kind,
            name: s.name.to_owned(),
            pos: s.pos,
            dest: None,
            speed: s.speed,
            generation: s.generation,
            targeting: super::TargetingState {
                target: None,
                dead,
                attackable: s.kind == EntityKind::Npc && s.combat.is_some(),
            },
            combat: s.combat,
            ai: None,
        };
        let event = spawn_event(tick, &entity);
        self.entities.insert(s.id, entity);
        Ok(vec![event])
    }

    fn despawn(&mut self, tick: Tick, id: EntityId) -> Vec<ZoneEvent> {
        let mut events = Vec::new();
        self.disengage(tick, id, SwingCancel::AttackerDied, &mut events);
        match self.entities.remove(&id) {
            Some(e) => {
                self.aoi.remove(id, e.pos);
                self.known.remove(&id);
                self.hate.remove(&id);
                events.push(ZoneEvent::EntityDespawn { tick, entity: id });
                self.release_target(tick, id, &mut events);
                events
            },
            None => events,
        }
    }

    fn set_target(
        &mut self,
        tick: Tick,
        entity: EntityId,
        target: Option<EntityId>,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let actor = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if let Some(id) = target {
            let selected = self.entities.get(&id).ok_or(RejectReason::UnknownEntity)?;
            if !self.aoi.in_aoi(actor.pos).any(|visible| visible == id) {
                return Err(RejectReason::TargetNotInAoi);
            }
            if selected.kind == EntityKind::Player
                || selected.targeting.dead
                || !selected.targeting.attackable
            {
                return Err(RejectReason::NonAttackableTarget);
            }
        }
        if actor.targeting.target == target {
            return Ok(Vec::new());
        }
        // Changing or clearing the selection ends the attack (plan §3.2).
        let mut events = Vec::new();
        self.disengage(tick, entity, SwingCancel::TargetChanged, &mut events);
        let actor = self
            .entities
            .get_mut(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        actor.targeting.target = target;
        events.push(ZoneEvent::TargetChanged {
            tick,
            entity,
            target,
        });
        Ok(events)
    }

    fn move_to(
        &mut self,
        tick: Tick,
        entity: EntityId,
        dest: Vec2Fixed,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let e = self
            .entities
            .get(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if !self.bounds.contains(dest) {
            return Err(RejectReason::OutOfBounds);
        }
        if !e
            .pos
            .within(dest, Fixed::from_tiles(MAX_MOVE_DISTANCE_TILES))
        {
            return Err(RejectReason::TooFar);
        }
        // Walking away ends the attack (plan §3.2).
        let mut events = Vec::new();
        self.disengage(tick, entity, SwingCancel::Moved, &mut events);
        let e = self
            .entities
            .get_mut(&entity)
            .ok_or(RejectReason::UnknownEntity)?;
        if let Some(c) = e.combat.as_mut() {
            c.chasing = false;
        }
        // Accepted silently: `step` on this same tick emits the first `EntityMove`.
        e.dest = Some(dest);
        Ok(events)
    }

    /// A version-4 UUID from the zone RNG, for ids that are not inputs (plan §8 #6).
    fn random_entity_id(&mut self) -> EntityId {
        let mut bytes = [0_u8; 16];
        self.rng.fill_bytes(&mut bytes);
        EntityId::from_uuid(uuid::Builder::from_random_bytes(bytes).into_uuid())
    }

    /// The movement phase of a tick, exposed for benchmarks; [`Self::run_tick`] is the
    /// transition. Every entity with a destination moves one step and emits an `EntityMove`;
    /// the step that reaches the destination emits a final one with `dest: None`. An entity
    /// that cannot move (speed 0) emits nothing. Entities are visited in id order. After all
    /// moves, returning NPCs that arrived home heal and become active in the same id order.
    pub fn step(&mut self, tick: Tick) -> Vec<ZoneEvent> {
        let mut events = Vec::new();
        let mut arrived = Vec::new();
        for e in self.entities.values_mut() {
            let Some(dest) = e.dest else { continue };
            let from = e.pos;
            e.pos = from.step_toward(dest, e.speed);
            if e.pos == from && e.pos != dest {
                continue;
            }
            if e.pos == dest {
                e.dest = None;
                arrived.push(e.id);
            }
            self.aoi.relocate(e.id, from, e.pos);
            events.push(ZoneEvent::EntityMove {
                tick,
                entity: e.id,
                pos: e.pos,
                dest: e.dest,
                speed: e.speed,
            });
        }
        // The AI phase precedes movement: complete returns here so the arrival tick cannot
        // end with an NPC at home but still injured and unattackable until the next tick.
        for id in arrived {
            if self.is_returning(id) {
                self.check_return(tick, id, &mut events);
            }
        }
        events
    }

    /// Builds each player's ordered output for the tick and updates what it knows. Order per
    /// player: responses to its own commands (ordinal order), AOI despawns, AOI spawns, then the
    /// end-of-tick state of known entities that moved; each group in entity-id order. Players
    /// are visited in id order. Nothing here depends on anything but state and inputs.
    ///
    /// Players in the same cell share one AOI, so the sorted AOI and its move list are built
    /// once per occupied cell and reused; a player whose AOI did not change (the common case)
    /// just copies the cell's move list.
    fn observe(
        &mut self,
        tick: Tick,
        mut responses: BTreeMap<CommandSource, Vec<ObserverOutput>>,
        events: &[ZoneEvent],
    ) -> BTreeMap<EntityId, Vec<ObserverOutput>> {
        let mut moved: Vec<EntityId> = events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::EntityMove { .. }))
            .map(ZoneEvent::entity)
            .collect();
        moved.sort_unstable();
        moved.dedup();
        let entities = &self.entities;
        self.known.retain(|id, _| entities.contains_key(id));
        let mut views: BTreeMap<CellCoord, CellView> = BTreeMap::new();
        let mut outputs = BTreeMap::new();
        for player in entities.values().filter(|e| e.kind == EntityKind::Player) {
            let source = CommandSource::Session {
                entity: player.id,
                generation: player.generation,
            };
            // Keyed by the full source, so a fenced (older) session's responses never reach
            // the current one.
            let mut out = responses.remove(&source).unwrap_or_default();
            let view = views
                .entry(CellCoord::of(player.pos))
                .or_insert_with(|| CellView::build(&self.aoi, entities, player.pos, &moved, tick));
            let before = self.known.entry(player.id).or_default();
            if *before == view.ids {
                out.extend(view.moves.iter().map(|(_, m)| m.clone()));
            } else {
                diff_into(&mut out, tick, before, view, entities);
                before.clone_from(&view.ids);
            }
            // Facts after AOI output, so a spawn always precedes a fact referring to a newly
            // visible entity; facts about entities outside the AOI are recorded, not sent.
            out.extend(
                events
                    .iter()
                    .filter(|event| fact_visible(event, player.id, &view.ids))
                    .cloned()
                    .map(ObserverOutput::Event),
            );
            if !out.is_empty() {
                outputs.insert(player.id, out);
            }
        }
        outputs
    }
}

/// Whether `observer`, which ends the tick knowing `known` (sorted), is sent `event` as a
/// fact. Owner-only: stats, XP, level and selection (a selection only once its target is
/// known). Cross-entity combat facts: only when both sides are known. Hate and AI intentions
/// are internal.
/// Spawns, moves and despawns are the AOI diff's, never facts.
fn fact_visible(event: &ZoneEvent, observer: EntityId, known: &[EntityId]) -> bool {
    let sees = |id: &EntityId| *id == observer || known.binary_search(id).is_ok();
    match event {
        ZoneEvent::TargetChanged { entity, target, .. } => {
            *entity == observer && target.as_ref().is_none_or(sees)
        },
        ZoneEvent::StatsChanged { entity, .. }
        | ZoneEvent::XpGained { entity, .. }
        | ZoneEvent::LevelUp { entity, .. } => *entity == observer,
        ZoneEvent::AttackResult {
            attacker, target, ..
        }
        | ZoneEvent::AttackStarted {
            attacker, target, ..
        }
        | ZoneEvent::AttackCancelled {
            attacker, target, ..
        } => sees(attacker) && sees(target),
        ZoneEvent::EntityDied { entity, .. } | ZoneEvent::EntityRespawned { entity, .. } => {
            sees(entity)
        },
        ZoneEvent::HateChanged { .. }
        | ZoneEvent::NpcIntentionChanged { .. }
        | ZoneEvent::Progression(_)
        | ZoneEvent::EntitySpawn { .. }
        | ZoneEvent::EntityMove { .. }
        | ZoneEvent::EntityDespawn { .. } => false,
    }
}

/// Each session's responses for the tick, keyed by source, in ordinal order: an
/// [`ObserverOutput::Accepted`] for every applied command that carries a `seq`, an
/// [`ObserverOutput::Rejected`] for every refused one. `dispositions` is in ordinal order, as
/// `commands` is.
fn responses(
    tick: Tick,
    commands: &[AppliedCommand],
    dispositions: &[Disposition],
) -> BTreeMap<CommandSource, Vec<ObserverOutput>> {
    let mut out: BTreeMap<CommandSource, Vec<ObserverOutput>> = BTreeMap::new();
    let mut refused = dispositions.iter().peekable();
    for c in commands {
        let response = match refused.next_if(|d| d.ordinal == c.ordinal) {
            Some(d) => Some(ObserverOutput::Rejected(*d)),
            None => c.seq.map(|seq| ObserverOutput::Accepted {
                ordinal: c.ordinal,
                seq,
                tick,
            }),
        };
        if let (CommandSource::Session { .. }, Some(r)) = (c.source, response) {
            out.entry(c.source).or_default().push(r);
        }
    }
    out
}

/// One cell's AOI at the end of a tick: who is visible (sorted) and the moves among them.
struct CellView {
    ids: Vec<EntityId>,
    moves: Vec<(EntityId, ObserverOutput)>,
}

impl CellView {
    fn build(
        aoi: &AoiIndex,
        entities: &BTreeMap<EntityId, Entity>,
        pos: Vec2Fixed,
        moved: &[EntityId],
        tick: Tick,
    ) -> Self {
        let mut ids: Vec<EntityId> = aoi.in_aoi(pos).collect();
        ids.sort_unstable();
        let move_list = ids
            .iter()
            .filter(|id| moved.binary_search(id).is_ok())
            .filter_map(|id| entities.get(id))
            .map(|e| {
                let m = ZoneEvent::EntityMove {
                    tick,
                    entity: e.id,
                    pos: e.pos,
                    dest: e.dest,
                    speed: e.speed,
                };
                (e.id, ObserverOutput::Event(m))
            })
            .collect();
        Self {
            ids,
            moves: move_list,
        }
    }
}

/// Appends despawns (`before` minus `now`), spawns (`now` minus `before`) and moves of
/// entities in both, each in id order. Both id lists are sorted.
fn diff_into(
    out: &mut Vec<ObserverOutput>,
    tick: Tick,
    before: &[EntityId],
    now: &CellView,
    entities: &BTreeMap<EntityId, Entity>,
) {
    let has = |list: &[EntityId], id: &EntityId| list.binary_search(id).is_ok();
    out.extend(
        before
            .iter()
            .filter(|id| !has(&now.ids, id))
            .map(|id| ObserverOutput::Event(ZoneEvent::EntityDespawn { tick, entity: *id })),
    );
    out.extend(
        now.ids
            .iter()
            .filter(|id| !has(before, id))
            .filter_map(|id| entities.get(id))
            .map(|e| ObserverOutput::Event(spawn_event(tick, e))),
    );
    out.extend(
        now.moves
            .iter()
            .filter(|(id, _)| has(before, id))
            .map(|(_, m)| m.clone()),
    );
}

/// The spawn an observer receives when `e` enters its AOI: full current movement state.
fn spawn_event(tick: Tick, e: &Entity) -> ZoneEvent {
    ZoneEvent::EntitySpawn {
        tick,
        entity: e.id,
        kind: e.kind,
        name: e.name.clone(),
        pos: e.pos,
        dest: e.dest,
        speed: e.speed,
        generation: e.generation,
        combat: e
            .combat
            .as_ref()
            .map(|c| c.view(e.targeting.dead, e.targeting.attackable)),
    }
}

/// The arguments of [`ZoneState::spawn`].
struct Spawned<'a> {
    id: EntityId,
    kind: EntityKind,
    name: &'a str,
    pos: Vec2Fixed,
    speed: Speed,
    generation: SessionGeneration,
    combat: Option<CombatState>,
}

/// A player's combat block from its loaded state.
fn player_combat(rules: &StatRules, load: &PlayerLoad) -> Result<CombatState, RejectReason> {
    let class = rules.class(&load.class).ok_or(RejectReason::InvalidLoad)?;
    let max_xp = xp_cap(rules).map_err(|_| RejectReason::InvalidLoad)?;
    if load.xp > max_xp || level_for_xp(rules, load.xp) != load.level {
        return Err(RejectReason::InvalidLoad);
    }
    let weapon = rules.starter_weapon();
    let sheet = StatSheet::for_player(rules, class, load.level, Some(weapon))
        .map_err(|_| RejectReason::InvalidLoad)?;
    let hp = if load.alive {
        load.hp.map_or(sheet.max_hp(), |hp| hp.min(sheet.max_hp()))
    } else {
        0
    };
    Ok(CombatState {
        role: CombatRole::Player {
            class: load.class.clone(),
            xp: load.xp,
        },
        hp,
        mp: load.mp.map_or(sheet.max_mp(), |mp| mp.min(sheet.max_mp())),
        sheet,
        attack_range: weapon_reach(weapon).map_err(|_| RejectReason::InvalidLoad)?,
        collision_radius: Fixed::from_raw(0),
        incarnation: 1,
        auto_attack: false,
        chasing: false,
        swing: None,
        ready_at: Tick(0),
        protected_until: None,
    })
}

/// An NPC's combat block from its resolved profile, at full HP and MP.
fn npc_combat(spec: &NpcCombat) -> Result<CombatState, RejectReason> {
    let sheet = StatSheet::from_final(spec.stats).map_err(|_| RejectReason::NotPermitted)?;
    if sheet.max_hp() == 0 {
        return Err(RejectReason::NotPermitted);
    }
    Ok(CombatState {
        role: CombatRole::Npc {
            template: spec.template.clone(),
            xp_reward: spec.xp_reward,
        },
        hp: sheet.max_hp(),
        mp: sheet.max_mp(),
        sheet,
        attack_range: spec.attack_range,
        collision_radius: spec.collision_radius,
        incarnation: 1,
        auto_attack: false,
        chasing: false,
        swing: None,
        ready_at: Tick(0),
        protected_until: None,
    })
}

/// The owner-only resource fact.
pub(super) fn stats_changed(tick: Tick, entity: EntityId, c: &CombatState) -> ZoneEvent {
    ZoneEvent::StatsChanged {
        tick,
        entity,
        hp: c.hp,
        max_hp: c.sheet.max_hp(),
        mp: c.mp,
        max_mp: c.sheet.max_mp(),
        level: c.sheet.level(),
        xp: match c.role {
            CombatRole::Player { xp, .. } => xp,
            CombatRole::Npc { .. } => 0,
        },
    }
}

#[path = "state_combat.rs"]
mod combat_phase;

#[path = "state_ai.rs"]
mod ai_phase;

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ai_tests.rs"]
#[allow(
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::unreachable
)]
mod ai_tests;

#[cfg(test)]
#[path = "combat_tests.rs"]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::unreachable
)]
mod combat_tests;

#[cfg(test)]
#[path = "death_tests.rs"]
#[allow(
    clippy::many_single_char_names,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::wildcard_enum_match_arm,
    clippy::unreachable
)]
mod death_tests;
