//! The zone's authoritative state and its one transition, [`ZoneState::run_tick`]: apply this
//! tick's commands in ordinal order, advance movement, then diff every player's area of
//! interest into its ordered output stream.

use std::collections::BTreeMap;

use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::aoi::{AoiCell, AoiIndex, CellCoord};
use super::command::{
    AppliedCommand, AppliedTick, AppliedTickDraft, CommandSource, Disposition, ObserverOutput,
    Ordinal, RejectReason, SessionGeneration, ZoneCommand, ZoneEvent, ZoneInput,
};
use super::entity::{Entity, EntityId, EntityKind, Tick};
use super::fixed::{Fixed, Speed, Vec2Fixed};

/// Largest distance one `MoveTo` may cover, in tiles. Longer trips are several commands, which
/// bounds the work a single malicious intent can cause and keeps paths inside the AOI.
pub const MAX_MOVE_DISTANCE_TILES: i32 = 64;

/// Version of the [`ZoneSnapshot`] layout. Bump on any change to the snapshot or to the
/// meaning of a field; `from_snapshot` refuses other versions.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
/// them (Story 3.4); replay compares them and warns on mismatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMeta {
    /// [`SNAPSHOT_SCHEMA_VERSION`] at the time of writing.
    pub schema_version: u32,
    /// Server build that wrote it. Crate version until CI stamps a commit id.
    pub build_id: String,
    /// Hash of the zone configuration (bounds, spawn tables) the epoch ran with.
    pub config_hash: String,
    /// `JetStream` sequence of the epoch's first applied-tick record, once Story 3.2 logs them.
    pub first_log_seq: Option<u64>,
}

impl Default for SnapshotMeta {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            build_id: env!("CARGO_PKG_VERSION").to_owned(),
            config_hash: String::new(),
            first_log_seq: None,
        }
    }
}

/// A zone's complete state at a tick boundary (plan §8 #5). Rebuilding from it with
/// [`ZoneState::from_snapshot`] and feeding the same drafts yields byte-identical
/// [`AppliedTick`]s, which replay (Story 3.3) relies on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneSnapshot {
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
}

impl ZoneState {
    /// An empty zone at tick 0, ordinal 0, with the RNG at the start of its stream.
    /// `time_origin_ms` is the wall-clock time the caller assigns to tick 0.
    #[must_use]
    pub fn new(seed: ZoneSeed, bounds: ZoneBounds, time_origin_ms: i64) -> Self {
        Self {
            seed,
            rng: ChaCha12Rng::from_seed(seed.key()),
            bounds,
            time_origin_ms,
            next_tick: Tick(0),
            next_ordinal: Ordinal(0),
            entities: BTreeMap::new(),
            aoi: AoiIndex::default(),
            known: BTreeMap::new(),
        }
    }

    /// Rebuilds a zone from a snapshot, validating it.
    pub fn from_snapshot(snapshot: ZoneSnapshot) -> Result<Self, SnapshotError> {
        if snapshot.meta.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(SnapshotError::Schema(snapshot.meta.schema_version));
        }
        if snapshot.rng.key != snapshot.seed.key() {
            return Err(SnapshotError::SeedMismatch);
        }
        let mut state = Self::new(snapshot.seed, snapshot.bounds, snapshot.time_origin_ms);
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
            state.aoi.insert(e.id, e.pos);
            state.entities.insert(e.id, e);
        }
        if state.aoi.to_cells() != snapshot.aoi {
            return Err(SnapshotError::AoiMismatch);
        }
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
            meta: SnapshotMeta::default(),
            seed: self.seed,
            tick: self.next_tick,
            next_ordinal: self.next_ordinal,
            time_origin_ms: self.time_origin_ms,
            rng: RngState::capture(&self.rng),
            bounds: self.bounds,
            entities: self.entities.values().cloned().collect(),
            aoi: self.aoi.to_cells(),
        }
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
        // Accept v < 4_294_967_000, the largest multiple of 1000 not above 2^32.
        const LIMIT: u32 = u32::MAX - (u32::MAX % 1000) - 1;
        loop {
            let v = self.rng.next_u32();
            if v <= LIMIT {
                // v % 1000 < 1000, so the conversion cannot fail.
                return u16::try_from(v % 1000).unwrap_or(0);
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

    /// Phase two: commits a draft. Applies its commands in ordinal order, advances movement,
    /// then diffs every player's AOI. The result is a pure function of the state and the
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
        events.extend(self.step(tick));
        let outputs = self.observe(tick, &dispositions, &events);
        self.next_tick = tick.next();
        Ok(AppliedTick {
            epoch: draft.epoch,
            tick,
            server_time_ms: self.server_time_ms(tick),
            commands: draft.commands,
            dispositions,
            events,
            outputs,
        })
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
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::ReplaceSession { .. } => Err(RejectReason::NotPermitted),
        }
    }

    /// Applies one command. `Err` is a refusal and leaves the zone unchanged.
    fn apply(&mut self, tick: Tick, c: &AppliedCommand) -> Result<Vec<ZoneEvent>, RejectReason> {
        self.authorize(c.source, &c.command)?;
        match &c.command {
            ZoneCommand::SpawnPlayer {
                entity,
                name,
                pos,
                speed,
                generation,
            } => self.spawn(tick, *entity, EntityKind::Player, name, *pos, *speed, *generation),
            ZoneCommand::SpawnNpc { name, pos, speed } => {
                if !self.bounds.contains(*pos) {
                    return Err(RejectReason::OutOfBounds);
                }
                let id = self.random_entity_id();
                let generation = SessionGeneration::default();
                self.spawn(tick, id, EntityKind::Npc, name, *pos, *speed, generation)
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
                // The new session knows nothing yet: the AOI diff resends everything.
                self.known.remove(entity);
                Ok(Vec::new())
            },
            ZoneCommand::MoveTo { entity, dest } => self.move_to(*entity, *dest),
            ZoneCommand::StopMove { entity } => {
                let e = self
                    .entities
                    .get_mut(entity)
                    .ok_or(RejectReason::UnknownEntity)?;
                Ok(e.dest
                    .take()
                    .map(|_| ZoneEvent::EntityMove {
                        tick,
                        entity: *entity,
                        pos: e.pos,
                        dest: None,
                        speed: e.speed,
                    })
                    .into_iter()
                    .collect())
            },
        }
    }

    #[allow(clippy::too_many_arguments)] // one call per spawn kind; a struct would only rename them
    fn spawn(
        &mut self,
        tick: Tick,
        id: EntityId,
        kind: EntityKind,
        name: &str,
        pos: Vec2Fixed,
        speed: Speed,
        generation: SessionGeneration,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        if self.entities.contains_key(&id) {
            return Err(RejectReason::AlreadyExists);
        }
        if !self.bounds.contains(pos) {
            return Err(RejectReason::OutOfBounds);
        }
        self.aoi.insert(id, pos);
        self.entities.insert(
            id,
            Entity {
                id,
                kind,
                name: name.to_owned(),
                pos,
                dest: None,
                speed,
                generation,
            },
        );
        Ok(vec![ZoneEvent::EntitySpawn {
            tick,
            entity: id,
            kind,
            name: name.to_owned(),
            pos,
            dest: None,
            speed,
            generation,
        }])
    }

    fn despawn(&mut self, tick: Tick, id: EntityId) -> Vec<ZoneEvent> {
        match self.entities.remove(&id) {
            Some(e) => {
                self.aoi.remove(id, e.pos);
                self.known.remove(&id);
                vec![ZoneEvent::EntityDespawn { tick, entity: id }]
            },
            None => Vec::new(),
        }
    }

    fn move_to(
        &mut self,
        entity: EntityId,
        dest: Vec2Fixed,
    ) -> Result<Vec<ZoneEvent>, RejectReason> {
        let e = self
            .entities
            .get_mut(&entity)
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
        // Accepted silently: `step` on this same tick emits the first `EntityMove`.
        e.dest = Some(dest);
        Ok(Vec::new())
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
    /// that cannot move (speed 0) emits nothing. Entities are visited in id order.
    pub fn step(&mut self, tick: Tick) -> Vec<ZoneEvent> {
        let mut events = Vec::new();
        for e in self.entities.values_mut() {
            let Some(dest) = e.dest else { continue };
            let from = e.pos;
            e.pos = from.step_toward(dest, e.speed);
            if e.pos == from && e.pos != dest {
                continue;
            }
            if e.pos == dest {
                e.dest = None;
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
        events
    }

    /// Builds each player's ordered output for the tick and updates what it knows. Order per
    /// player: its own dispositions (ordinal order), AOI despawns, AOI spawns, then the
    /// end-of-tick state of known entities that moved; each group in entity-id order. Players
    /// are visited in id order. Nothing here depends on anything but state and inputs.
    ///
    /// Players in the same cell share one AOI, so the sorted AOI and its move list are built
    /// once per occupied cell and reused; a player whose AOI did not change (the common case)
    /// just copies the cell's move list.
    fn observe(
        &mut self,
        tick: Tick,
        dispositions: &[Disposition],
        events: &[ZoneEvent],
    ) -> BTreeMap<EntityId, Vec<ObserverOutput>> {
        let mut moved: Vec<EntityId> = events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::EntityMove { .. }))
            .map(ZoneEvent::entity)
            .collect();
        moved.sort_unstable();
        moved.dedup();
        let mut rejected: BTreeMap<CommandSource, Vec<ObserverOutput>> = BTreeMap::new();
        for d in dispositions {
            rejected
                .entry(d.source)
                .or_default()
                .push(ObserverOutput::Rejected(*d));
        }
        let entities = &self.entities;
        self.known.retain(|id, _| entities.contains_key(id));
        let mut views: BTreeMap<CellCoord, CellView> = BTreeMap::new();
        let mut outputs = BTreeMap::new();
        for player in entities.values().filter(|e| e.kind == EntityKind::Player) {
            let source = CommandSource::Session {
                entity: player.id,
                generation: player.generation,
            };
            let mut out = rejected.remove(&source).unwrap_or_default();
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
            if !out.is_empty() {
                outputs.insert(player.id, out);
            }
        }
        outputs
    }
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
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
