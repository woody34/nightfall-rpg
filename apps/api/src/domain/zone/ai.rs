//! NPC AI data (plan §3.2, Stories E3.2–E3.4): the L2J intention subset, per-NPC AI state, the
//! immutable spawn-slot specs and the respawn scheduler's per-member state. Plain data; the
//! transitions live in `state_ai.rs`.
//!
//! Constants follow L2J `L2AttackableAI` (docs/planning/06-world-and-content.md §2.5) at 100 ms
//! ticks; L2 world units convert at [`L2_UNITS_PER_TILE`].

use serde::{Deserialize, Serialize};

use super::combat::{NpcCombat, L2_UNITS_PER_TILE};
use super::entity::{EntityId, Tick};
use super::fixed::{Fixed, Speed, Vec2Fixed, UNITS_PER_TILE};
use super::npc_template::{NpcTemplate, SpawnSlot};

/// Ticks between two thinks of one NPC (L2J: once a second).
pub const THINK_INTERVAL_TICKS: u64 = 10;

/// An `Active` NPC that can move wanders on 1 think in this many (`RANDOM_WALK_RATE`).
pub const RANDOM_WALK_RATE: u32 = 30;

/// Ticks in `Attack` without a landed hit before the NPC gives up (`MAX_ATTACK_TIMEOUT`).
pub const MAX_ATTACK_TIMEOUT_TICKS: u64 = 1200;

/// L2J's random-walk offset around the spawn (`MAX_DRIFT_RANGE`, 300 world units).
pub const MAX_DRIFT_RANGE_L2: i32 = 300;

/// [`MAX_DRIFT_RANGE_L2`] in milli-tiles: `300 * 1000 / 32 = 9375`. A wander destination is
/// at most this far from home on each axis.
pub const MAX_DRIFT_RANGE: Fixed =
    Fixed::from_raw(MAX_DRIFT_RANGE_L2 * UNITS_PER_TILE / L2_UNITS_PER_TILE);

/// The L2J intention subset (plan D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intention {
    /// No player nearby; thinks only to wake.
    Idle,
    /// Players nearby: scans for aggro, otherwise may wander.
    Active,
    /// Fighting its most hated target.
    Attack,
    /// Walking home; cannot be damaged or aggroed; full HP on arrival.
    ReturnHome,
    /// A corpse, then hidden until its slot respawns it.
    Dead,
}

/// Which spawn-slot member an NPC is: `slot` indexes the zone's slot list, `member` counts
/// `0..count`. Stable across lives.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct SlotMember {
    /// Index into the zone's spawn slots.
    pub slot: u16,
    /// Member within the slot.
    pub member: u16,
}

/// The per-NPC AI block on [`super::Entity`]. Only spawn-slot NPCs have one; NPCs spawned by
/// `SpawnNpc` keep the bare E2.5 behaviour (target the most hated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcAi {
    /// Owning slot member.
    pub slot: SlotMember,
    /// Where it spawned, wanders around and returns to.
    pub home: Vec2Fixed,
    /// Current intention.
    pub intention: Intention,
    /// Entering `Attack` and every landed hit reset this; [`MAX_ATTACK_TIMEOUT_TICKS`] after
    /// it the NPC returns home.
    pub last_hit: Tick,
    /// This engagement already made (or answered) a clan call; no recursive help.
    pub called_help: bool,
    /// While dead: the tick the corpse is removed.
    pub corpse_until: Option<Tick>,
}

impl NpcAi {
    /// Think phase `0..10`: thinks run when `tick % 10` equals it. Derived from the id (itself
    /// drawn from the zone RNG), so it costs no draw and survives respawn.
    #[must_use]
    pub fn think_phase(id: EntityId) -> u64 {
        let phase = id
            .as_uuid()
            .as_u128()
            .checked_rem(u128::from(THINK_INTERVAL_TICKS))
            .unwrap_or(0);
        u64::try_from(phase).unwrap_or(0)
    }
}

/// The AI half of a template, resolved once at bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcBrain {
    /// Attacks players entering `aggro_range`.
    pub aggressive: bool,
    /// Aggro detection range.
    pub aggro_range: Fixed,
    /// Social clan; `None` helps nobody.
    pub clan: Option<String>,
    /// Range of a clan call.
    pub clan_help_range: Fixed,
    /// Leaving this radius around home ends the fight.
    pub leash_radius: Fixed,
    /// Ticks a corpse stays.
    pub corpse_decay_ticks: u32,
}

impl NpcBrain {
    /// The template's AI fields.
    #[must_use]
    pub fn from_template(t: &NpcTemplate) -> Self {
        Self {
            aggressive: t.aggressive,
            aggro_range: t.aggro_range,
            clan: t.clan_id.clone(),
            clan_help_range: t.clan_help_range,
            leash_radius: t.leash_radius,
            corpse_decay_ticks: t.corpse_decay_ticks,
        }
    }
}

/// One spawn slot as the zone keeps it: everything a new life needs, so respawning never
/// consults data files. Immutable; part of the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSlotSpec {
    /// Slot id from the zone file.
    pub id: String,
    /// Display name of its monsters.
    pub name: String,
    /// Home of every member.
    pub home: Vec2Fixed,
    /// Members.
    pub count: u16,
    /// Movement speed.
    pub speed: Speed,
    /// Combat profile.
    pub combat: NpcCombat,
    /// AI profile.
    pub brain: NpcBrain,
    /// Base respawn delay, seconds.
    pub respawn_delay_secs: u32,
    /// Random extra respawn seconds, `0..=` this.
    pub respawn_random_secs: u32,
}

impl SpawnSlotSpec {
    /// A slot with its template's combat and AI profiles.
    #[must_use]
    pub fn new(slot: &SpawnSlot, template: &NpcTemplate, combat: NpcCombat) -> Self {
        Self {
            id: slot.id.clone(),
            name: template.name.clone(),
            home: slot.home,
            count: slot.count,
            speed: template.move_speed,
            combat,
            brain: NpcBrain::from_template(template),
            respawn_delay_secs: slot.respawn_delay_secs,
            respawn_random_secs: slot.respawn_random_secs,
        }
    }
}

/// The scheduler's record of one slot member (plan E3.4). `entity` is fixed by the first life
/// and reused by every respawn; `respawn_at` is set while the member is dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberState {
    /// Which member.
    pub member: SlotMember,
    /// The member's entity id; `None` until its first life.
    pub entity: Option<EntityId>,
    /// The last life spawned (0 before the first).
    pub incarnation: u32,
    /// When the next life spawns; `None` while one is alive.
    pub respawn_at: Option<Tick>,
}
