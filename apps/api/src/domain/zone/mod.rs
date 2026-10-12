//! Deterministic zone simulation (plan D7, docs/plans/phase-0b-connected-slice.md Story 3.1).
//!
//! The same snapshot plus the same commands on the same ticks must produce byte-identical
//! events on any machine and any build, so replay (Story 3.3) can prove a recorded session.
//! Rules for everything under this module (docs/engineering/rust-guidelines.md, Determinism):
//!
//! * No floats. Positions are [`Fixed`] (1/1000 tile, `i32`); distances compare squared
//!   integers. Conversion to wire floats lives in `interface::zone_mapping`.
//! * No hash-ordered collections. `BTreeMap` / `BTreeSet` only, so iteration order is defined.
//! * No wall clock. Time is the [`Tick`] passed in by the caller.
//! * No ambient randomness. The only generator is the `ChaCha12` stream seeded from
//!   [`ZoneSeed`] that [`ZoneState`] owns.
//!
//! The lints below turn the float rule into a compile error under clippy, and
//! `tests::zone_sources_use_no_nondeterministic_apis` scans these files for the rest.

#![deny(
    clippy::float_arithmetic,
    clippy::float_cmp,
    clippy::float_cmp_const,
    clippy::lossy_float_literal,
    clippy::cast_precision_loss,
    clippy::imprecise_flops,
    clippy::suboptimal_flops
)]

mod ai;
mod aoi;
mod combat;
mod combat_math;
mod command;
pub mod effective_stats;
pub mod effects;
mod entity;
pub mod evidence;
mod fixed;
pub mod item;
pub mod item_angles;
mod item_cast;
pub mod item_combat;
pub mod item_conditions;
pub mod item_enchant;
pub mod item_equipment;
pub mod item_exchange;
pub mod item_functions;
pub mod item_grant;
pub mod item_inventory;
pub mod item_modifiers;
pub mod item_refresh;
pub mod item_rules;
pub mod item_sources;
pub mod item_views;
mod npc_template;
mod phase2;
mod phase3;
mod phase3_checkpoint;
mod phase3_entity;
pub mod phase4;
pub mod phase4_admission;
pub mod phase4_checkpoint;
pub mod phase4_projection;
mod progression;
mod scaled;
pub mod skill;
pub mod skill_math;
pub mod skill_npc;
pub mod skill_profiles;
pub mod skill_rules;
pub mod stat_functions;
mod stat_rules;
mod stat_sheet;
#[cfg(test)]
mod stat_tests;
mod state;
pub mod state_damage;
pub mod state_death;
pub mod state_effects;
pub mod state_learn;
pub mod state_pvp;
pub mod state_regen;
pub mod state_skill;

pub use ai::{
    Intention, MemberState, NpcAi, NpcBrain, SlotMember, SpawnSlotSpec, MAX_ATTACK_TIMEOUT_TICKS,
    MAX_DRIFT_RANGE, MAX_DRIFT_RANGE_L2, RANDOM_WALK_RATE, THINK_INTERVAL_TICKS,
};
pub use aoi::{aoi_cells_for, AoiCell, AoiIndex, CellCoord, AOI_CELL_TILES};
pub use combat::{
    weapon_reach, CombatRole, CombatState, CombatView, HateEntry, HateLedger, NpcCombat,
    PlayerLoad, Swing, SwingCancel, L2_UNITS_PER_TILE, NPC_BASE_CRIT, NPC_BASE_STATS,
};
pub use combat_math::{
    add_hate, attack_timing, crit_lands, damage_hate, hit_chance_permille, hit_lands,
    npc_respawn_tick, physical_damage, spawn_protection_ticks, town_respawn_vitals, AttackTiming,
};
pub use command::{
    AppliedCommand, AppliedTick, AppliedTickDraft, AttackOutcome, AttackResultPhase3,
    AttackStartedPhase3, CombatViewPhase3, CommandSource, DeathFact, Disposition, ObserverOutput,
    Ordinal, OwnerPhase3View, ProgressionDelta, RejectReason, ResurrectOfferState,
    SessionGeneration, SkillOutcome, SkillResolutionKind, SkillTargetResult, TargetChangedPhase3,
    ZoneCommand, ZoneEvent, ZoneInput,
};
pub use entity::{Entity, EntityId, EntityKind, TargetingState, Tick, TICK_MS};
pub use fixed::{Fixed, Speed, Vec2Fixed, UNITS_PER_TILE};
pub use item::{
    EquipSlot, ItemCreationOrigin, ItemError, ItemInstance, ItemInstanceId, ItemInstanceRange,
    ItemLocation,
};
pub use item_enchant::EnchantAttempt;
pub use item_exchange::ExchangeAudit;
pub use item_grant::{ItemGrantCommand, ItemPausedCore};
pub use item_rules::Phase4Rules;
pub use npc_template::{NpcTemplate, NpcTemplateId, SpawnSlot};
pub use phase2::{
    profession_stats, transfer_resource, ClassStatsView, PlayerIdentityView, PlayerProgression,
};
pub use phase3::{Phase3AdmissionPolicy, Phase3Rules, Phase3Snapshot};
pub use phase3_checkpoint::{
    DeathRecord, FrozenLearnResult, LearnedEffectiveStats, Phase3Checkpoint, ResourceFractions,
    RestoredModifierSource, RestoredPhase3Timers,
};
pub use phase3_entity::{
    CastId, CastInFlight, CastState, EntityOrdinal, OfferId, Phase3Entity, Phase3EntityContext,
    Phase3Player, PvpState, ResurrectOffer,
};
pub use phase4::{Phase4Player, Phase4Snapshot};
pub use phase4_admission::{
    ItemCommand, ItemCommandKind, MaterializePendingItems, Phase4PlayerLoad,
};
pub use phase4_checkpoint::Phase4Checkpoint;
pub use phase4_projection::{FrozenItemResult, ItemResultDetail, Phase4Projection};
pub use progression::{add_xp, death_xp_loss, level_for_xp, xp_after_death, xp_cap};
pub use scaled::{ceil_div, floor_div, isqrt, Scaled, StatError, Q};
pub use stat_rules::{
    Archetype, BonusTable, ClassTemplate, FormulaConstants, Quadratic, RuleViolation,
    StatBonusTables, StatKind, StatRules, StatRulesParts, WeaponBlock, BONUS_TABLE_LEN,
    MAX_SUPPORTED_LEVEL,
};
pub use stat_sheet::{
    accuracy, attack_speed, crit_permille, evasion, fist_random_damage, level_mod, p_atk, p_def,
    resource_max, sqrt_dex, FinalStats, StatSheet,
};
pub use state::{
    CheckpointLearnReceiptSnapshot, CheckpointRequestSnapshot, CheckpointSnapshot, InvalidBounds,
    ItemCheckpointWorkSnapshot, NpcHate, RngState, SnapshotError, SnapshotMeta, StateDigestVersion,
    TickError, ZoneBounds, ZoneId, ZoneSeed, ZoneSnapshot, ZoneState, MAX_MOVE_DISTANCE_TILES,
    SNAPSHOT_SCHEMA_VERSION,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    /// Every source file in this module, embedded at compile time so the scan cannot miss a
    /// file that exists but is not listed: adding a module without adding it here fails
    /// `every_zone_source_is_scanned`.
    const SOURCES: [(&str, &str); 81] = [
        ("mod.rs", include_str!("mod.rs")),
        ("item.rs", include_str!("item.rs")),
        ("item_rules.rs", include_str!("item_rules.rs")),
        ("item_sources.rs", include_str!("item_sources.rs")),
        ("item_angles.rs", include_str!("item_angles.rs")),
        ("item_cast.rs", include_str!("item_cast.rs")),
        ("item_combat.rs", include_str!("item_combat.rs")),
        ("phase4.rs", include_str!("phase4.rs")),
        ("phase4_projection.rs", include_str!("phase4_projection.rs")),
        ("item_inventory.rs", include_str!("item_inventory.rs")),
        ("item_conditions.rs", include_str!("item_conditions.rs")),
        ("item_views.rs", include_str!("item_views.rs")),
        ("item_functions.rs", include_str!("item_functions.rs")),
        ("item_modifiers.rs", include_str!("item_modifiers.rs")),
        ("item_refresh.rs", include_str!("item_refresh.rs")),
        ("item_equipment.rs", include_str!("item_equipment.rs")),
        ("phase4_admission.rs", include_str!("phase4_admission.rs")),
        ("phase4_checkpoint.rs", include_str!("phase4_checkpoint.rs")),
        ("evidence.rs", include_str!("evidence.rs")),
        ("state_evidence.rs", include_str!("state_evidence.rs")),
        ("effects.rs", include_str!("effects.rs")),
        ("effective_stats.rs", include_str!("effective_stats.rs")),
        ("skill_profiles.rs", include_str!("skill_profiles.rs")),
        ("skill.rs", include_str!("skill.rs")),
        ("skill_math.rs", include_str!("skill_math.rs")),
        ("skill_npc.rs", include_str!("skill_npc.rs")),
        ("skill_rules.rs", include_str!("skill_rules.rs")),
        ("phase3.rs", include_str!("phase3.rs")),
        ("phase3_checkpoint.rs", include_str!("phase3_checkpoint.rs")),
        ("phase3_entity.rs", include_str!("phase3_entity.rs")),
        ("state_effects.rs", include_str!("state_effects.rs")),
        ("state_skill.rs", include_str!("state_skill.rs")),
        ("state_pvp.rs", include_str!("state_pvp.rs")),
        ("state_learn.rs", include_str!("state_learn.rs")),
        ("state_runtime_learn.rs", include_str!("state_runtime_learn.rs")),
        ("state_death.rs", include_str!("state_death.rs")),
        ("state_runtime_life.rs", include_str!("state_runtime_life.rs")),
        ("state_runtime_observe.rs", include_str!("state_runtime_observe.rs")),
        ("state_runtime_auto.rs", include_str!("state_runtime_auto.rs")),
        ("state_runtime_damage.rs", include_str!("state_runtime_damage.rs")),
        ("state_damage.rs", include_str!("state_damage.rs")),
        ("phase2.rs", include_str!("phase2.rs")),
        ("state_class.rs", include_str!("state_class.rs")),
        ("ai.rs", include_str!("ai.rs")),
        ("ai_tests.rs", include_str!("ai_tests.rs")),
        ("state_ai.rs", include_str!("state_ai.rs")),
        ("aoi.rs", include_str!("aoi.rs")),
        ("combat.rs", include_str!("combat.rs")),
        ("combat_tests.rs", include_str!("combat_tests.rs")),
        ("combat_math.rs", include_str!("combat_math.rs")),
        ("command.rs", include_str!("command.rs")),
        ("death_tests.rs", include_str!("death_tests.rs")),
        ("entity.rs", include_str!("entity.rs")),
        ("fixed.rs", include_str!("fixed.rs")),
        ("npc_template.rs", include_str!("npc_template.rs")),
        ("progression.rs", include_str!("progression.rs")),
        ("scaled.rs", include_str!("scaled.rs")),
        ("stat_functions.rs", include_str!("stat_functions.rs")),
        ("stat_rules.rs", include_str!("stat_rules.rs")),
        ("stat_sheet.rs", include_str!("stat_sheet.rs")),
        ("stat_tests.rs", include_str!("stat_tests.rs")),
        ("state_regen.rs", include_str!("state_regen.rs")),
        ("state_runtime.rs", include_str!("state_runtime.rs")),
        ("state_runtime_item_cast.rs", include_str!("state_runtime_item_cast.rs")),
        ("state_runtime_effects.rs", include_str!("state_runtime_effects.rs")),
        ("state.rs", include_str!("state.rs")),
        ("state_digest.rs", include_str!("state_digest.rs")),
        ("state_digest_items.rs", include_str!("state_digest_items.rs")),
        ("state_items.rs", include_str!("state_items.rs")),
        ("state_item_commands.rs", include_str!("state_item_commands.rs")),
        ("state_item_enchant.rs", include_str!("state_item_enchant.rs")),
        ("state_item_evidence.rs", include_str!("state_item_evidence.rs")),
        ("state_item_use.rs", include_str!("state_item_use.rs")),
        ("state_item_materialize.rs", include_str!("state_item_materialize.rs")),
        ("item_grant.rs", include_str!("item_grant.rs")),
        ("item_enchant.rs", include_str!("item_enchant.rs")),
        ("item_exchange.rs", include_str!("item_exchange.rs")),
        ("state_item_grant.rs", include_str!("state_item_grant.rs")),
        ("state_item_views.rs", include_str!("state_item_views.rs")),
        ("state_combat.rs", include_str!("state_combat.rs")),
        ("state_tests.rs", include_str!("state_tests.rs")),
    ];

    /// Tokens whose presence would make the zone nondeterministic. Built by concatenation so
    /// this list does not trip its own scan.
    fn forbidden() -> Vec<String> {
        [
            ("f3", "2"),
            ("f6", "4"),
            ("Hash", "Map"),
            ("Hash", "Set"),
            ("Inst", "ant"),
            ("System", "Time"),
            ("thread_", "rng"),
            ("Os", "Rng"),
            ("rand::", "random"),
            ("from_", "entropy"),
        ]
        .iter()
        .map(|(a, b)| format!("{a}{b}"))
        .collect()
    }

    #[test]
    fn zone_sources_use_no_nondeterministic_apis() {
        let forbidden = forbidden();
        for (file, src) in SOURCES {
            for (n, line) in src.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for token in &forbidden {
                    assert!(
                        !code.contains(token.as_str()),
                        "domain/zone/{file}:{} uses `{token}`, which breaks determinism (plan D7)",
                        n + 1
                    );
                }
            }
        }
    }

    #[test]
    fn every_zone_source_is_scanned() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain/zone");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e == "rs")
            })
            .collect();
        on_disk.sort();
        let mut scanned: Vec<String> = SOURCES.iter().map(|(n, _)| (*n).to_owned()).collect();
        scanned.sort();
        assert_eq!(on_disk, scanned);
    }
}
