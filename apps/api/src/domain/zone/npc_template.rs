//! Validated NPC templates and spawn slots (plan Phase 1 E3.1). Pure data handed to the zone by
//! the infrastructure loader; no floats. Fractional combat values are Q units (1e-6).

use super::{Fixed, Speed, Vec2Fixed};

/// Q scale for fractional stats: 1.0 = `1_000_000`.
pub const Q: i64 = 1_000_000;

/// Stable template id, equal to the file stem under `packages/data/npcs/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NpcTemplateId(pub String);

/// An attackable monster type. Combat stats are final values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpcTemplate {
    /// Template id.
    pub id: NpcTemplateId,
    /// Display name.
    pub name: String,
    /// Level, at least 1.
    pub level: u16,
    /// P.Atk in Q units.
    pub p_atk_q: i64,
    /// P.Def in Q units.
    pub p_def_q: i64,
    /// Maximum HP, at least 1.
    pub max_hp: u32,
    /// Maximum MP.
    pub max_mp: u32,
    /// P.Atk.Spd; the attack interval derives from it.
    pub attack_speed: u32,
    /// Melee reach.
    pub attack_range: Fixed,
    /// Movement speed.
    pub move_speed: Speed,
    /// Collision radius.
    pub collision_radius: Fixed,
    /// XP granted on death.
    pub xp_reward: u64,
    /// Attacks players that come within `aggro_range`.
    pub aggressive: bool,
    /// Aggro detection range.
    pub aggro_range: Fixed,
    /// Clan for social help; `None` = helps nobody.
    pub clan_id: Option<String>,
    /// Range within which clan members are called to help.
    pub clan_help_range: Fixed,
    /// Maximum distance from home before it gives up and returns.
    pub leash_radius: Fixed,
    /// Ticks a corpse stays.
    pub corpse_decay_ticks: u32,
    /// Base respawn delay, seconds, at least 1.
    pub respawn_delay_secs: u32,
    /// Random extra respawn seconds, drawn from `0..=` this.
    pub respawn_random_secs: u32,
}

/// A place where `count` monsters of one template live and respawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSlot {
    /// Slot id, unique in the zone.
    pub id: String,
    /// The template spawned.
    pub template: NpcTemplateId,
    /// Home position, inside the zone bounds.
    pub home: Vec2Fixed,
    /// Monsters in the slot, at least 1.
    pub count: u16,
    /// Respawn delay in seconds (template default or slot override).
    pub respawn_delay_secs: u32,
    /// Random extra respawn seconds (template default or slot override).
    pub respawn_random_secs: u32,
}
