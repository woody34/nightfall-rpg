//! Combat state carried by entities and the zone (plan §3.2, Stories E2.2, E2.3, E2.5): the
//! per-entity combat block, the pending swing, what a player or NPC spawns with, and the NPC
//! hate ledger. Plain data; the transitions live in `state_combat.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::BaseStats;

use super::combat_math::add_hate;
use super::entity::{EntityId, Tick};
use super::fixed::{Fixed, UNITS_PER_TILE};
use super::npc_template::NpcTemplate;
use super::scaled::{Scaled, StatError};
use super::stat_rules::{FormulaConstants, StatKind, StatRules, WeaponBlock};
use super::stat_sheet::{
    accuracy, crit_permille, evasion, fist_random_damage, FinalStats, StatSheet,
};

/// L2 world units per tile (docs/planning/00-foundations.md: an AOI cell of 512 world units is
/// 16 tiles of 32). Converts the starter weapon's literal L2 reach into milli-tiles.
pub const L2_UNITS_PER_TILE: i32 = 32;

/// Base stats L2J High Five gives ordinary monsters (`STR 40 DEX 30 CON 43 INT 21 WIT 20
/// MEN 20`). NPC templates carry final P.Atk/P.Def/HP/speed but no accuracy, evasion or crit,
/// so those three come from the HF formulas over these stats, like L2J's `FuncAtkAccuracy`,
/// `FuncAtkEvasion` and `FuncAtkCritical` do for every `L2Character`.
pub const NPC_BASE_STATS: BaseStats = BaseStats {
    str: 40,
    dex: 30,
    con: 43,
    int: 21,
    wit: 20,
    men: 20,
};

/// HF monster base critical rate (`baseCritRate` 4, before the DEX bonus).
pub const NPC_BASE_CRIT: u32 = 4;

/// Why a pending swing ended without an impact. No random number is drawn for a cancelled
/// swing (plan §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwingCancel {
    /// `StopAttack`.
    Stopped,
    /// The attacker selected another target or cleared it.
    TargetChanged,
    /// The attacker was told to move (`MoveTo` / `StopMove`).
    Moved,
    /// The target was outside reach at impact.
    OutOfRange,
    /// The target died, left, was replaced by a new life, or became protected.
    TargetLost,
    /// The attacker died or left.
    AttackerDied,
}

/// A swing in flight: started at `start`, lands at `impact`, and the next may start at
/// `ready`. All three are whole ticks from the stat engine's [`super::attack_timing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Swing {
    /// Who it is aimed at.
    pub target: EntityId,
    /// The target's life when the swing started; a different life at impact cancels it.
    pub target_incarnation: u32,
    /// Start tick.
    pub start: Tick,
    /// Impact tick.
    pub impact: Tick,
    /// Earliest start of the next swing.
    pub ready: Tick,
}

/// What kind of combatant an entity is, with the facts only that kind has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CombatRole {
    /// A player character.
    Player {
        /// Class data id (`classes/<id>.toml`).
        class: String,
        /// Cumulative XP; level is derived from it.
        xp: u64,
    },
    /// A monster spawned from a template.
    Npc {
        /// Template id (`npcs/<id>.toml`).
        template: String,
        /// XP granted for the kill (E2.6).
        xp_reward: u64,
    },
}

/// The combat block of an entity (plan §3.2 "Snapshot ..." list). Present only on combatants;
/// noncombat fixture NPCs have none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatState {
    /// Player or NPC facts.
    pub role: CombatRole,
    /// Derived stats; maxima come from here.
    pub sheet: StatSheet,
    /// Current whole HP; 0 when dead.
    pub hp: u32,
    /// Current whole MP.
    pub mp: u32,
    /// Own melee reach.
    pub attack_range: Fixed,
    /// Body radius added to an attacker's reach.
    pub collision_radius: Fixed,
    /// Life counter, starting at 1; a respawned NPC gets a new one.
    pub incarnation: u32,
    /// Auto-attack enabled on the current target.
    pub auto_attack: bool,
    /// `true` while the destination was set by the chase, not by `MoveTo`.
    pub chasing: bool,
    /// The swing in flight, if any.
    pub swing: Option<Swing>,
    /// Earliest tick the next swing may start (survives a cancelled swing, so stopping and
    /// re-enabling never shortens the cycle).
    pub ready_at: Tick,
    /// Respawn protection ends at this tick (E2.4); attacking ends it early.
    pub protected_until: Option<Tick>,
}

impl CombatState {
    /// `true` when the entity is protected on `tick`.
    #[must_use]
    pub fn protected_at(&self, tick: Tick) -> bool {
        self.protected_until.is_some_and(|until| tick < until)
    }

    /// What observers entering AOI see.
    #[must_use]
    pub fn view(&self, dead: bool, attackable: bool) -> CombatView {
        CombatView {
            template: match &self.role {
                CombatRole::Npc { template, .. } => Some(template.clone()),
                CombatRole::Player { .. } => None,
            },
            incarnation: self.incarnation,
            dead,
            attackable,
            hp: self.hp,
            max_hp: self.sheet.max_hp(),
            level: self.sheet.level(),
            swing: self.swing,
        }
    }
}

/// Public combat state in an `EntitySpawn`, so an observer entering AOI (or a replacement
/// session) can render life, HP and a swing already in flight. MP and XP are owner-only and
/// travel in `StatsChanged` / `XpGained`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatView {
    /// NPC template id; `None` for players.
    pub template: Option<String>,
    /// Life counter.
    pub incarnation: u32,
    /// Dead (a corpse) or alive.
    pub dead: bool,
    /// Whether players may target it.
    pub attackable: bool,
    /// Current whole HP.
    pub hp: u32,
    /// Maximum whole HP.
    pub max_hp: u32,
    /// Level.
    pub level: u32,
    /// Swing in flight.
    pub swing: Option<Swing>,
}

/// A player's loaded character state, carried by `SpawnPlayer` so the zone never reads a
/// repository (plan D5). HP/MP `None` means full (a character never saved in combat).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerLoad {
    /// Class data id; resolved against the zone's injected rules.
    pub class: String,
    /// Saved level; must equal the level of `xp`.
    pub level: u32,
    /// Saved cumulative XP.
    pub xp: u64,
    /// Saved HP, clamped to the maximum; 0 spawns the character dead.
    pub hp: Option<u32>,
    /// Saved MP, clamped to the maximum.
    pub mp: Option<u32>,
    /// `false` spawns the character dead (HP 0) so death survives reconnect (E2.4).
    #[serde(default = "alive_default")]
    pub alive: bool,
}

const fn alive_default() -> bool {
    true
}

impl PlayerLoad {
    /// A character with no saved combat state yet: level 1, no XP, full HP/MP.
    #[must_use]
    pub fn fresh(class: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            level: 1,
            xp: 0,
            hp: None,
            mp: None,
            alive: true,
        }
    }
}

/// A monster's combat profile as `SpawnNpc` carries it: final values resolved from the
/// template once, at bootstrap, so the command (and its replay) is self-contained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcCombat {
    /// Template id.
    pub template: String,
    /// Final stats.
    pub stats: FinalStats,
    /// Melee reach.
    pub attack_range: Fixed,
    /// Body radius.
    pub collision_radius: Fixed,
    /// XP for the kill.
    pub xp_reward: u64,
}

impl NpcCombat {
    /// Resolves a template. P.Atk, P.Def, HP/MP and attack speed are the template's final
    /// values (plan §3.1); accuracy, evasion and crit derive from [`NPC_BASE_STATS`] and
    /// [`NPC_BASE_CRIT`]; the random-damage radius is HF's unarmed `5 + isqrt(level)`.
    pub fn from_template(rules: &StatRules, t: &NpcTemplate) -> Result<Self, StatError> {
        let c = rules.constants();
        let level = u32::from(t.level);
        let dex = rules.bonus().bonus(StatKind::Dex, NPC_BASE_STATS.dex)?;
        let stats = FinalStats {
            level,
            max_hp: t.max_hp,
            max_mp: t.max_mp,
            p_atk: Scaled::from_raw(t.p_atk_q),
            p_def: Scaled::from_raw(t.p_def_q),
            accuracy: accuracy(
                c,
                &NPC_BASE_STATS,
                level,
                rules.accuracy_level_add(level)?,
                Scaled::ZERO,
            )?,
            evasion: evasion(c, &NPC_BASE_STATS, level, rules.evasion_level_add(level)?)?,
            crit_permille: crit_permille(c, NPC_BASE_CRIT, dex)?,
            attack_speed: Scaled::from_int(t.attack_speed.into())?,
            random_damage: fist_random_damage(c, level)?,
        };
        StatSheet::from_final(stats)?;
        Ok(Self {
            template: t.id.0.clone(),
            stats,
            attack_range: t.attack_range,
            collision_radius: t.collision_radius,
            xp_reward: t.xp_reward,
        })
    }
}

/// The starter weapon's reach in milli-tiles: `attack_range_l2 * 1000 / 32`, floored.
pub fn weapon_reach(w: &WeaponBlock) -> Result<Fixed, StatError> {
    let units = i64::from(w.attack_range_l2)
        .checked_mul(i64::from(UNITS_PER_TILE))
        .ok_or(StatError::Overflow)?
        .checked_div(i64::from(L2_UNITS_PER_TILE))
        .ok_or(StatError::Overflow)?;
    i32::try_from(units)
        .map(Fixed::from_raw)
        .map_err(|_| StatError::Overflow)
}

/// One attacker's row in an NPC's ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HateEntry {
    /// Hate, capped at the HF cap.
    pub hate: u64,
    /// Damage actually dealt, tracked separately from hate (plan §3.1 "Hate").
    pub damage: u64,
}

/// An NPC's hate/damage ledger, ordered by attacker id (plan E2.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HateLedger {
    entries: BTreeMap<EntityId, HateEntry>,
}

impl HateLedger {
    /// Adds `hate` (capped) and `damage` for `attacker`; returns the new row.
    pub fn add(
        &mut self,
        c: &FormulaConstants,
        attacker: EntityId,
        hate: u64,
        damage: u64,
    ) -> HateEntry {
        let row = self.entries.entry(attacker).or_default();
        row.hate = add_hate(c, row.hate, hate);
        row.damage = row.damage.saturating_add(damage);
        *row
    }

    /// Drops `attacker`; `true` if it had a row.
    pub fn forget(&mut self, attacker: EntityId) -> bool {
        self.entries.remove(&attacker).is_some()
    }

    /// One attacker's row.
    #[must_use]
    pub fn get(&self, attacker: EntityId) -> Option<HateEntry> {
        self.entries.get(&attacker).copied()
    }

    /// Rows in attacker-id order.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, HateEntry)> + '_ {
        self.entries.iter().map(|(id, e)| (*id, *e))
    }

    /// No rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The most hated attacker among those `eligible` accepts. Ties keep `current` if it is
    /// among the tied, then go to the lowest id (plan §3.2).
    pub fn most_hated(
        &self,
        current: Option<EntityId>,
        mut eligible: impl FnMut(EntityId) -> bool,
    ) -> Option<EntityId> {
        let mut best: Option<(u64, EntityId)> = None;
        for (id, row) in &self.entries {
            if !eligible(*id) {
                continue;
            }
            let better = match best {
                None => true,
                Some((hate, chosen)) => {
                    row.hate > hate || (row.hate == hate && Some(*id) == current && chosen != *id)
                },
            };
            if better {
                best = Some((row.hate, *id));
            }
        }
        best.map(|(_, id)| id)
    }
}
