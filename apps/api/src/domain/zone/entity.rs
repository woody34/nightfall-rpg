//! Simulation time and the entities a zone owns.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::ids::uuid_id;

use super::command::SessionGeneration;
use super::fixed::{Speed, Vec2Fixed};

/// Milliseconds per tick (plan D7, docs/planning/03-combat-and-skills.md §3.2).
pub const TICK_MS: u64 = 100;

/// A simulation tick number. The only clock the zone has: nothing in `domain::zone` reads
/// wall time, so a replay that feeds the same commands on the same ticks gets the same output.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u64);

impl Tick {
    /// The following tick, saturating at `u64::MAX` (a 100 ms tick takes 58 billion years to
    /// get there).
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Simulation time at the start of this tick, in milliseconds since the epoch started.
    #[must_use]
    pub const fn elapsed_ms(self) -> u64 {
        self.0.saturating_mul(TICK_MS)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

uuid_id!(
    /// Identity of an entity in a zone. Players use their character id; NPCs get one at spawn.
    EntityId
);

/// What an entity is. Mirrors `nightfall.v1.EntityKind` minus `UNSPECIFIED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// Controlled by a connected session.
    Player,
    /// Controlled by the server.
    Npc,
}

/// One entity in a zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Identity.
    pub id: EntityId,
    /// Player or NPC.
    pub kind: EntityKind,
    /// Display name.
    pub name: String,
    /// Current position.
    pub pos: Vec2Fixed,
    /// Where it is heading; `None` when standing still.
    pub dest: Option<Vec2Fixed>,
    /// Movement speed.
    pub speed: Speed,
    /// For players, the generation of the session that owns the entity. Always the default
    /// for NPCs.
    pub generation: SessionGeneration,
}
