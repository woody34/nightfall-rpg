//! What goes into a zone (inputs and commands) and what comes out of one tick (the
//! [`AppliedTick`] record: applied commands, dispositions, world events, per-observer output).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::combat::{CombatView, NpcCombat, PlayerLoad, SwingCancel};
use super::entity::{EntityId, EntityKind, Tick};
use super::fixed::{Speed, Vec2Fixed};

/// Position of a command in the zone's application order, unique within an epoch. Assigned by
/// the zone when the command is applied, never by the sender (plan §8 #3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Ordinal(pub u64);

/// Which admission of a player's account owns the entity (plan §8 #8). A replacement session
/// gets a higher generation; commands from older generations are fenced off.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SessionGeneration(pub u64);

/// Who issued a command. Recorded with it, so replay applies the same authorisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandSource {
    /// The server itself: admission, eviction, NPC spawning. May command any entity.
    System,
    /// A connected player session. May only command its own entity, and only while its
    /// generation is current.
    Session {
        /// The session's player entity (its character id).
        entity: EntityId,
        /// The admission generation the session was opened with.
        generation: SessionGeneration,
    },
}

/// A command plus its source, as queued for the zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneInput {
    /// Who asked.
    pub source: CommandSource,
    /// The session's `ClientMessage.seq`, echoed in a rejection so the client can match it.
    /// `None` for system commands.
    pub seq: Option<u32>,
    /// What they asked for.
    pub command: ZoneCommand,
}

impl ZoneInput {
    /// A command issued by the server.
    #[must_use]
    pub const fn system(command: ZoneCommand) -> Self {
        Self {
            source: CommandSource::System,
            seq: None,
            command,
        }
    }

    /// A command issued by a player session.
    #[must_use]
    pub const fn session(
        entity: EntityId,
        generation: SessionGeneration,
        seq: u32,
        command: ZoneCommand,
    ) -> Self {
        Self {
            source: CommandSource::Session { entity, generation },
            seq: Some(seq),
            command,
        }
    }
}

/// An intent the zone may reject. Every state change is one of these (plan §8 #4), so the
/// applied-command log is a complete input history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ZoneCommand {
    /// A player enters with its loaded character state. System only.
    SpawnPlayer {
        /// The character id; player entity ids are always character ids.
        entity: EntityId,
        /// Character name.
        name: String,
        /// Saved position; must be inside the zone bounds.
        pos: Vec2Fixed,
        /// Movement speed.
        speed: Speed,
        /// Generation of the admitting session.
        generation: SessionGeneration,
        /// Loaded combat state; `None` spawns a noncombat player (zones without rules).
        load: Option<Box<PlayerLoad>>,
    },
    /// The server places an NPC. System only. The id is drawn from the zone's seeded RNG so a
    /// replay assigns the same one.
    SpawnNpc {
        /// Display name.
        name: String,
        /// Where it appears; must be inside the zone bounds.
        pos: Vec2Fixed,
        /// Movement speed.
        speed: Speed,
        /// Combat profile; `None` is a noncombat fixture.
        combat: Option<Box<NpcCombat>>,
    },
    /// Remove an entity. Removing an unknown entity is a no-op, so a disconnect racing a
    /// server-side removal is harmless.
    Despawn {
        /// Who leaves.
        entity: EntityId,
    },
    /// A newer session takes over a player entity (plan §8 #8). System only. The entity
    /// stays where it is; the new session receives the full AOI again on this tick.
    ReplaceSession {
        /// The player entity.
        entity: EntityId,
        /// The new session's generation; must be higher than the current one.
        generation: SessionGeneration,
    },
    /// Start walking toward `dest`.
    MoveTo {
        /// Who moves.
        entity: EntityId,
        /// Target point; inside the bounds and within [`super::MAX_MOVE_DISTANCE_TILES`].
        dest: Vec2Fixed,
    },
    /// Select or clear a live attackable NPC in the actor's AOI. Repeats are no-ops.
    SetTarget {
        /// Session-owned actor.
        entity: EntityId,
        /// None clears selection.
        target: Option<EntityId>,
    },
    /// Enable auto-attack on the current target: chase into reach, then swing every cycle.
    /// Repeating never resets a cycle or adds a swing.
    Attack {
        /// Session-owned actor.
        entity: EntityId,
    },
    /// Disable auto-attack and cancel the pending swing; repeating is harmless.
    StopAttack {
        /// Session-owned actor.
        entity: EntityId,
    },
    /// Request town respawn for a dead actor. E2.1 records a `NotYetImplemented` disposition.
    Respawn {
        /// Session-owned actor.
        entity: EntityId,
    },
    /// Stop where it stands.
    StopMove {
        /// Who stops.
        entity: EntityId,
    },
    /// An auto or social aggro entry: `npc` adds 1 hate for `target` and re-selects its most
    /// hated target (plan §3.1 "Hate"). System only; the NPC AI (E3.2) issues it.
    AddAggro {
        /// The hating NPC.
        npc: EntityId,
        /// The living player it notices.
        target: EntityId,
    },
}

impl ZoneCommand {
    /// The entity the command is about. `None` for `SpawnNpc`, whose id is not known until the
    /// zone applies it.
    #[must_use]
    pub const fn entity(&self) -> Option<EntityId> {
        match self {
            Self::SpawnPlayer { entity, .. }
            | Self::Despawn { entity }
            | Self::ReplaceSession { entity, .. }
            | Self::MoveTo { entity, .. }
            | Self::StopMove { entity }
            | Self::SetTarget { entity, .. }
            | Self::Attack { entity }
            | Self::StopAttack { entity }
            | Self::Respawn { entity } => Some(*entity),
            Self::AddAggro { npc, .. } => Some(*npc),
            Self::SpawnNpc { .. } => None,
        }
    }
}

/// Why the zone refused a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// the actor is dead and cannot perform this intent.
    DeadActor,
    /// target is dead, a player, or a noncombat NPC.
    NonAttackableTarget,
    /// existing target is outside the actor's 3x3-cell AOI.
    TargetNotInAoi,
    /// target is outside melee range at impact; distinct from move `TOO_FAR`.
    OutOfRange,
    /// target has active respawn protection.
    Protected,
    /// intent recorded, but its behaviour is not implemented yet.
    NotYetImplemented,
    /// The actor or selected target is not in this zone.
    UnknownEntity,
    /// The target position is outside the zone bounds.
    OutOfBounds,
    /// The destination is further than the per-command limit.
    TooFar,
    /// A spawn reused an id that is already in the zone.
    AlreadyExists,
    /// The source may not issue this command or command this entity.
    NotPermitted,
    /// The session's generation is no longer current, or a replacement did not increase it.
    StaleSession,
    /// The command targets an NPC but only makes sense for a player.
    NotAPlayer,
    /// `SpawnPlayer` carried a class, level or XP the zone's rules do not accept.
    InvalidLoad,
}

impl RejectReason {
    /// Human-readable detail for `IntentRejected.detail`. Reasons without their own wire enum
    /// value (`AlreadyExists`, `NotPermitted`, `StaleSession`, `NotAPlayer`, `InvalidLoad`) go on the wire
    /// as `INVALID` with this text.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::DeadActor => "the actor is dead and cannot perform this intent",
            Self::NonAttackableTarget => "target is dead, a player, or a noncombat NPC",
            Self::TargetNotInAoi => "existing target is outside the actor's 3x3-cell AOI",
            Self::OutOfRange => {
                "target is outside melee range at impact; distinct from move TOO_FAR"
            },
            Self::Protected => "target has active respawn protection",
            Self::NotYetImplemented => "intent recorded, but its behaviour is not implemented yet",
            Self::UnknownEntity => "entity is not in this zone",
            Self::OutOfBounds => "position is outside the zone bounds",
            Self::TooFar => "destination is further than one move may cover",
            Self::AlreadyExists => "entity is already in this zone",
            Self::NotPermitted => "source may not issue this command",
            Self::StaleSession => "session generation is not current",
            Self::NotAPlayer => "entity is not a player",
            Self::InvalidLoad => "character state does not match the zone's rules",
        }
    }
}

/// The outcome record for a command the zone refused (plan §8 #4). Never silently dropped:
/// it is in the [`AppliedTick`] and in the issuing session's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disposition {
    /// The refused command's ordinal.
    pub ordinal: Ordinal,
    /// Who issued it.
    pub source: CommandSource,
    /// The issuing session's message seq, if any.
    pub seq: Option<u32>,
    /// The tick it was processed on.
    pub tick_seen: Tick,
    /// Why it was refused.
    pub reason: RejectReason,
}

/// One command as the zone processed it: ordinal, source, command. Accepted and refused
/// commands both appear, because both advance the ordinal and replay must re-feed both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedCommand {
    /// Position in application order.
    pub ordinal: Ordinal,
    /// Who issued it.
    pub source: CommandSource,
    /// The issuing session's message seq, if any.
    pub seq: Option<u32>,
    /// What it was.
    pub command: ZoneCommand,
}

/// A world fact produced on a tick. Mirrors `nightfall.v1.WorldEvent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // names mirror the proto messages one to one
pub enum ZoneEvent {
    /// Authoritative attack result fact.
    AttackResult {
        /// attacker UUID.
        attacker: EntityId,
        /// target UUID.
        target: EntityId,
        /// impact tick; 100 ms per tick.
        tick: Tick,
        /// MISS, HIT or CRIT; UNSPECIFIED is invalid.
        outcome: AttackOutcome,
        /// whole HP removed; zero on MISS.
        damage: u32,
        /// authoritative remaining whole HP after impact.
        target_hp_after: u32,
        /// The target's life the swing landed on.
        target_incarnation: u32,
    },
    /// Authoritative entity died fact.
    EntityDied {
        /// dead entity UUID.
        entity: EntityId,
        /// death tick.
        tick: Tick,
        /// killing entity UUID; empty if no killer.
        killer: Option<EntityId>,
        /// The life that ended.
        incarnation: u32,
    },
    /// A swing started; it lands at `impact` unless cancelled, and the next may start at
    /// `ready` (so clients wind animations up before impact).
    AttackStarted {
        /// Start tick.
        tick: Tick,
        /// Swinging entity.
        attacker: EntityId,
        /// Its target.
        target: EntityId,
        /// The target's life.
        target_incarnation: u32,
        /// Impact tick.
        impact: Tick,
        /// Next-swing tick.
        ready: Tick,
    },
    /// A pending swing ended without an impact.
    AttackCancelled {
        /// Tick of the fact.
        tick: Tick,
        /// Swinging entity.
        attacker: EntityId,
        /// Its target.
        target: EntityId,
        /// Why.
        reason: SwingCancel,
    },
    /// Internal: an NPC's hate row changed. `hate == 0 && damage == 0` means the row was
    /// forgotten. Never sent to a client; recorded so off-AOI aggro is replayed and compared.
    HateChanged {
        /// Tick of the fact.
        tick: Tick,
        /// The hating NPC.
        npc: EntityId,
        /// The hated entity.
        target: EntityId,
        /// Hate after the change.
        hate: u64,
        /// Damage dealt so far.
        damage: u64,
    },
    /// Authoritative entity respawned fact.
    EntityRespawned {
        /// respawned entity UUID.
        entity: EntityId,
        /// respawn tick.
        tick: Tick,
        /// authoritative safe point in tile units.
        position: Vec2Fixed,
        /// restored whole HP.
        hp: u32,
    },
    /// Authoritative stats changed fact.
    StatsChanged {
        /// Tick of the fact.
        tick: Tick,
        /// owner UUID; this event is owner-only (MP is private).
        entity: EntityId,
        /// current whole HP, zero when dead.
        hp: u32,
        /// maximum whole HP.
        max_hp: u32,
        /// current whole MP.
        mp: u32,
        /// maximum whole MP.
        max_mp: u32,
        /// current level, including decreases after death.
        level: u32,
    },
    /// Authoritative xp gained fact.
    XpGained {
        /// Tick of the fact.
        tick: Tick,
        /// owner UUID; this event is owner-only.
        entity: EntityId,
        /// whole XP actually awarded after the level-85 cap.
        amount: u64,
        /// authoritative cumulative whole XP after award.
        total: u64,
    },
    /// Authoritative level up fact.
    LevelUp {
        /// Tick of the fact.
        tick: Tick,
        /// levelled entity UUID.
        entity: EntityId,
        /// newly attained level; one event per crossed threshold.
        level: u32,
    },
    /// Authoritative target changed fact.
    TargetChanged {
        /// Tick of the fact.
        tick: Tick,
        /// selecting actor UUID; owner-only.
        entity: EntityId,
        /// selected target UUID; empty means cleared.
        target: Option<EntityId>,
    },

    /// An entity appeared (in the zone, or in an observer's AOI). Carries full movement state
    /// so an observer can render an entity that is already walking.
    EntitySpawn {
        /// When.
        tick: Tick,
        /// Who.
        entity: EntityId,
        /// Player or NPC.
        kind: EntityKind,
        /// Display name.
        name: String,
        /// Where.
        pos: Vec2Fixed,
        /// Where it is heading, if moving.
        dest: Option<Vec2Fixed>,
        /// Movement speed.
        speed: Speed,
        /// For players, the owning session's generation; default for NPCs.
        generation: SessionGeneration,
        /// Public combat state; `None` for noncombat entities.
        combat: Option<CombatView>,
    },
    /// An entity moved this tick, arrived, or stopped.
    EntityMove {
        /// When.
        tick: Tick,
        /// Who.
        entity: EntityId,
        /// Position at the end of the tick.
        pos: Vec2Fixed,
        /// Where it is still heading; `None` once it has arrived or stopped.
        dest: Option<Vec2Fixed>,
        /// Movement speed.
        speed: Speed,
    },
    /// An entity left (the zone, or an observer's AOI).
    EntityDespawn {
        /// When.
        tick: Tick,
        /// Who.
        entity: EntityId,
    },
}

impl ZoneEvent {
    /// The tick the event was produced on.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        match self {
            Self::EntitySpawn { tick, .. }
            | Self::EntityMove { tick, .. }
            | Self::EntityDespawn { tick, .. }
            | Self::AttackResult { tick, .. }
            | Self::EntityDied { tick, .. }
            | Self::EntityRespawned { tick, .. }
            | Self::StatsChanged { tick, .. }
            | Self::XpGained { tick, .. }
            | Self::LevelUp { tick, .. }
            | Self::TargetChanged { tick, .. }
            | Self::AttackStarted { tick, .. }
            | Self::AttackCancelled { tick, .. }
            | Self::HateChanged { tick, .. } => *tick,
        }
    }

    /// The entity the event is about.
    #[must_use]
    pub const fn entity(&self) -> EntityId {
        match self {
            Self::EntitySpawn { entity, .. }
            | Self::EntityMove { entity, .. }
            | Self::EntityDespawn { entity, .. }
            | Self::EntityDied { entity, .. }
            | Self::EntityRespawned { entity, .. }
            | Self::StatsChanged { entity, .. }
            | Self::XpGained { entity, .. }
            | Self::LevelUp { entity, .. }
            | Self::TargetChanged { entity, .. } => *entity,
            Self::AttackResult { attacker, .. }
            | Self::AttackStarted { attacker, .. }
            | Self::AttackCancelled { attacker, .. } => *attacker,
            Self::HateChanged { npc, .. } => *npc,
        }
    }
}

/// One item in a player's ordered output stream (plan §8 #6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObserverOutput {
    /// A world event visible to this observer.
    Event(ZoneEvent),
    /// One of this observer's own commands was accepted and applied on `tick`. Becomes
    /// `nightfall.v1.Ack`. Only commands that carry a session `seq` are acknowledged.
    Accepted {
        /// The accepted command's ordinal.
        ordinal: Ordinal,
        /// The issuing session's message seq.
        seq: u32,
        /// The tick it was applied on.
        tick: Tick,
    },
    /// One of this observer's own commands was refused.
    Rejected(Disposition),
}

/// Phase one of a tick: this tick's inputs with their ordinals assigned, before anything is
/// applied. The zone actor commits it with [`super::ZoneState::run_tick`] and hands the
/// resulting [`AppliedTick`] to its `TickGate` (the durable, acknowledged log append) before
/// releasing anything. Replay rebuilds drafts from the log and commits them the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedTickDraft {
    /// The zone's epoch.
    pub epoch: u64,
    /// The tick these commands will be applied on.
    pub tick: Tick,
    /// Commands in application order with consecutive ordinals.
    pub commands: Vec<AppliedCommand>,
}

/// Everything one tick did. This is the replay log's unit (plan §8 #3): feeding `commands` to
/// a zone restored from the snapshot at `tick` reproduces this record exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedTick {
    /// The zone's epoch.
    pub epoch: u64,
    /// The tick.
    pub tick: Tick,
    /// `time_origin_ms + tick * 100`: the tick's `server_time_ms` on the wire. Derived, never
    /// read from a clock (plan §8 #6).
    pub server_time_ms: i64,
    /// Commands in application order, accepted and refused.
    pub commands: Vec<AppliedCommand>,
    /// The refused subset, in ordinal order.
    pub dispositions: Vec<Disposition>,
    /// Zone-wide world events in the order they happened.
    pub events: Vec<ZoneEvent>,
    /// Per-player ordered output: responses to its own commands (acks and rejections, in
    /// ordinal order), then AOI despawns, AOI spawns, and moves of known entities, each group
    /// in entity-id order, then the combat facts it may see in causal (event) order. Only
    /// players with non-empty output appear. A session sends exactly this, in this order
    /// (plan §8 #6).
    pub outputs: BTreeMap<EntityId, Vec<ObserverOutput>>,
    /// SHA-256 of the canonical end-of-tick state (entities, hate, RNG, counters), so drift
    /// in state nobody observes still fails replay (plan §3.2).
    pub state_digest: [u8; 32],
}

impl AppliedTick {
    /// `true` when the tick changed nothing and produced nothing.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.commands.is_empty() && self.events.is_empty() && self.outputs.is_empty()
    }
}

/// Physical hit outcome, excluding the invalid wire UNSPECIFIED value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackOutcome {
    /// No damage; no crit or spread roll.
    Miss,
    /// Landed normal damage.
    Hit,
    /// Landed critical damage.
    Crit,
}
