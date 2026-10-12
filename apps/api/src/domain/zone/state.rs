//! The zone's authoritative state and its one transition, [`ZoneState::run_tick`]: apply this
//! tick's commands in ordinal order, expire corpses and respawn due slot members, run the NPC
//! AI, chase, advance movement, land due swings, start the next ones, then diff every player's
//! area of interest into its ordered output stream.

#[path = "state_digest.rs"]
mod digest;
#[path = "state_evidence.rs"]
mod evidence_hooks;
#[path = "state_item_commands.rs"]
mod item_commands;
#[path = "state_item_enchant.rs"]
mod item_enchant;
#[path = "state_item_evidence.rs"]
mod item_evidence_hooks;
#[path = "state_item_grant.rs"]
mod item_grant;
#[path = "state_item_materialize.rs"]
mod item_materialize;
#[path = "state_item_use.rs"]
mod item_use;
#[path = "state_item_views.rs"]
mod item_views;
#[path = "state_items.rs"]
mod items;
#[path = "state_runtime.rs"]
mod runtime;

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
/// meaning of a field; `from_snapshot` accepts 4 through 9. 2: combat state, hate
/// ledgers and the stat rules (Phase 1 E2.2). 3: NPC AI blocks, spawn slots and the respawn
/// scheduler (Phase 1 E3.2–E3.4). 4: safe point and the player's `alive` load flag (E2.4).
/// 5: application checkpoint lanes, excluded from the simulation digest.
/// 6: explicit state digest version; older snapshots default to JSON v1.
/// 7: resolved class registry, verified immutable hash, player identity and transfer ledger.
/// 8: `BinaryV4` once-ever token policy; older epochs keep supply disabled.
/// 9: isolated Phase 3 skills/effects/player-combat policy with `BinaryV5`.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 10;

/// Canonical state encoding hashed with SHA-256. Fixed for an epoch, including on restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StateDigestVersion {
    /// Original canonical JSON; the default for snapshots written before schema 6.
    #[default]
    JsonV1,
    /// Explicit little-endian binary layout in `state_digest.rs`.
    BinaryV2,
    /// Phase 2 identity, class ledger and immutable transfer receipts.
    BinaryV3,
    /// Once-ever token supply policy, bound to a new live epoch.
    BinaryV4,
    /// Phase 3 skills, effects, regeneration and player combat.
    BinaryV5,
    /// Item/equipment epochs with hash-resolved immutable rules.
    BinaryV6,
}

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
    /// Recorded pre-draft byte policy; required for V5 and forbidden in every legacy epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase3_admission_policy: Option<super::Phase3AdmissionPolicy>,
    /// Canonical catalogue provenance, independent of legacy stat rule provenance.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub classes_hash: String,
    /// Hash of the resolved immutable catalogue, verified on restore and bound into `BinaryV3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classes_rules_hash: Option<[u8; 32]>,
    /// [`SNAPSHOT_SCHEMA_VERSION`] at the time of writing.
    pub schema_version: u32,
    /// State digest algorithm for this epoch; absent in snapshot schemas 4 and 5.
    #[serde(default)]
    pub digest_version: StateDigestVersion,
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

impl SnapshotMeta {
    /// Explicit historical constructor; a latest-schema bump cannot upgrade legacy policy.
    pub fn legacy() -> Self {
        Self {
            phase3_admission_policy: None,
            classes_hash: String::new(),
            classes_rules_hash: None,
            schema_version: 8,
            digest_version: StateDigestVersion::BinaryV2,
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
    /// V6 item state; never synthesized for historical epochs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase4: Option<super::Phase4Snapshot>,
    /// Phase 3 canonical state; absent for every legacy epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase3: Option<super::Phase3Snapshot>,
    /// Resolved Phase 2 data; absent means legacy simulation rules.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_registry",
        deserialize_with = "deserialize_registry"
    )]
    pub classes: Option<Arc<crate::domain::class::ClassRegistry>>,
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

/// Exact acknowledged private item work retained until its fenced transaction succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemCheckpointWorkSnapshot {
    /// Final allocator from the acknowledged tick, never reconstructed from a later boundary.
    pub allocation: super::ItemInstanceRange,
    /// Frozen instance facts, receipts and audit evidence.
    pub projection: super::Phase4Projection,
}

/// Frozen paid-learning context for the same pending V6 transaction as item costs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CheckpointLearnReceiptSnapshot {
    pub account: crate::domain::AccountId,
    pub key: uuid::Uuid,
    pub fingerprint: String,
    pub result: super::FrozenLearnResult,
}

/// Persistence metadata owned by the application; it never participates in simulation hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    /// Optional unified paid-learning context, omitted from every historical snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning: Option<CheckpointLearnReceiptSnapshot>,
    /// Immutable critical item transaction awaiting acknowledgement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_work: Option<ItemCheckpointWorkSnapshot>,
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
    /// Paused item state, omitted from every historical request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase4: Option<super::Phase4Checkpoint>,
    /// Frozen paused timers and policy, absent from all legacy request fingerprints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase3: Option<super::Phase3Checkpoint>,
    /// Exact Phase 2 request body; omitted for old checkpoint fingerprints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_state: Option<crate::domain::character_progression::ClassState>,
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
    /// A new-policy command cannot enter a historical epoch, even through direct callers.
    #[error("Phase 3 command in a legacy epoch")]
    Phase3CommandInLegacy,
    /// Item intents require an explicitly admitted V6 epoch.
    #[error("Phase 4 command in a historical epoch")]
    Phase4CommandInLegacy,
    /// Only normalized authenticated item envelopes can become replay inputs.
    #[error("invalid Phase 4 command envelope")]
    Phase4CommandMalformed,
    /// The Phase 3 boundary exists, but gameplay installation is not complete.
    #[error("Phase 3 gameplay is not installed")]
    Phase3NotReady,
    /// A checked Phase 3 invariant failed; no draft is published.
    #[error("Phase 3 runtime invariant failed")]
    Phase3Invariant,
    /// Application stopped before applying a lifecycle transition after a failed save.
    #[error("checkpoint persistence fence")]
    PersistenceFence,
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

impl ZoneSnapshot {
    /// Validates the epoch's immutable format/policy pair without hashing catalogue data.
    /// All decoding and encoding boundaries share this check; old epochs never normalize
    /// an unsupported digest into an accepted one.
    pub fn validate_version(&self) -> Result<(), SnapshotError> {
        use StateDigestVersion::{BinaryV2, BinaryV3, BinaryV4, BinaryV5, BinaryV6, JsonV1};
        if self.meta.schema_version < 10
            && (self.phase4.is_some()
                || self.checkpoints.iter().any(|lane| {
                    lane.item_work.is_some()
                        || lane.learning.is_some()
                        || lane
                            .latest
                            .as_ref()
                            .is_some_and(|request| request.phase4.is_some())
                }))
        {
            return Err(SnapshotError::CombatMismatch);
        }
        let valid =
            match (self.meta.schema_version, self.meta.digest_version, self.classes.is_some()) {
                (4 | 5, JsonV1, false)
                | (6..=8, JsonV1 | BinaryV2, false)
                | (7, BinaryV3, true)
                | (8, BinaryV4, true) => {
                    self.phase3.is_none()
                        && self.meta.phase3_admission_policy.is_none()
                        && self.checkpoints.iter().all(|lane| {
                            lane.latest
                                .as_ref()
                                .is_none_or(|request| request.phase3.is_none())
                                && lane.events.iter().all(|event| {
                                    matches!(
                                        event.subject(),
                                        "nightfall.character.created"
                                            | "nightfall.character.leveled"
                                            | "nightfall.character.class_changed"
                                            | "nightfall.character.died"
                                            | "nightfall.character.token_granted"
                                    )
                                })
                        })
                },
                (9, BinaryV5, true) => {
                    self.validate_skill_boundary(2, 1)
                        && self.checkpoints.iter().all(|lane| {
                            lane.events.iter().all(|event| {
                                matches!(
                                    event.subject(),
                                    "nightfall.character.created"
                                        | "nightfall.character.leveled"
                                        | "nightfall.character.class_changed"
                                        | "nightfall.character.died"
                                        | "nightfall.character.token_granted"
                                        | "nightfall.character.skill_learned"
                                )
                            })
                        })
                },
                (10, BinaryV6, true) => {
                    self.validate_skill_boundary(3, 2)
                        && self
                            .phase3
                            .as_ref()
                            .is_some_and(|p| p.rules.runtime_revision() == 3)
                        && self
                            .phase4
                            .as_ref()
                            .is_some_and(|p| p.formula_revision == 1 && p.timer_policy_version == 1)
                },
                (4..=10, _, _) => false,
                (version, _, _) => return Err(SnapshotError::Schema(version)),
            };
        if valid {
            self.validate_item_checkpoints()?;
            Ok(())
        } else {
            Err(SnapshotError::CombatMismatch)
        }
    }
    fn validate_item_checkpoints(&self) -> Result<(), SnapshotError> {
        for lane in &self.checkpoints {
            self.validate_learning_checkpoint(lane)?;
            let items = lane
                .latest
                .as_ref()
                .and_then(|request| request.phase4.as_ref());
            if let Some(checkpoint) = items {
                let sources = lane
                    .latest
                    .as_ref()
                    .and_then(|request| request.phase3.as_ref())
                    .ok_or(SnapshotError::CombatMismatch)?
                    .modifier_install_order
                    .len();
                checkpoint
                    .validate_structure(sources)
                    .map_err(|_| SnapshotError::CombatMismatch)?;
                if self
                    .phase4
                    .as_ref()
                    .is_none_or(|phase| phase.rules_hash != checkpoint.rules_hash)
                {
                    return Err(SnapshotError::CombatMismatch);
                }
            }
            if let Some(work) = &lane.item_work {
                let request = lane.latest.as_ref().ok_or(SnapshotError::CombatMismatch)?;
                let sources = request
                    .phase3
                    .as_ref()
                    .ok_or(SnapshotError::CombatMismatch)?
                    .modifier_install_order
                    .len();
                work.projection
                    .validate_structure(sources)
                    .map_err(|_| SnapshotError::CombatMismatch)?;
                if work.projection.entity != lane.entity
                    || items != Some(&work.projection.checkpoint)
                    || work.projection.tick >= self.tick
                {
                    return Err(SnapshotError::CombatMismatch);
                }
                if let Some(core) = &work.projection.paused_core {
                    if self.entities.iter().any(|entity| entity.id == lane.entity)
                        || core.load.checkpoint_revision != Some(lane.revision)
                        || core.load.checkpoint_revision != Some(request.revision_seen)
                        || core.load.hp != Some(request.hp)
                        || core.load.mp != Some(request.mp)
                        || core.load.level != request.level
                        || core.load.xp != request.xp
                        || core.load.alive != request.alive
                        || core.load.phase3 != request.phase3
                        || core.load.progression.as_ref().map(|p| &p.class_state)
                            != request.class_state.as_ref()
                    {
                        return Err(SnapshotError::CombatMismatch);
                    }
                } else if self.checkpoint_owner(lane.entity) != Some(work.projection.account) {
                    return Err(SnapshotError::CombatMismatch);
                }
                let allocation = self
                    .phase4
                    .as_ref()
                    .ok_or(SnapshotError::CombatMismatch)?
                    .allocator
                    .bounds();
                let stored = work.allocation.bounds();
                if stored.0 != allocation.0 || stored.2 != allocation.2 || stored.1 > allocation.1 {
                    return Err(SnapshotError::CombatMismatch);
                }
            }
        }
        Ok(())
    }
    fn checkpoint_owner(&self, entity: EntityId) -> Option<crate::domain::AccountId> {
        self.entities
            .iter()
            .find(|owner| owner.id == entity)
            .and_then(|owner| owner.combat.as_ref())
            .and_then(|combat| {
                if let CombatRole::Player {
                    progression: Some(progression),
                    ..
                } = &combat.role
                {
                    Some(progression.identity.account_id)
                } else {
                    None
                }
            })
    }
    fn validate_learning_checkpoint(&self, lane: &CheckpointSnapshot) -> Result<(), SnapshotError> {
        use crate::domain::DomainEvent;
        let learned: Vec<_> = lane
            .events
            .iter()
            .filter(|event| matches!(event, DomainEvent::CharacterSkillLearned { .. }))
            .collect();
        let Some(receipt) = &lane.learning else {
            if self.meta.schema_version >= 10 && !learned.is_empty() {
                return Err(SnapshotError::CombatMismatch);
            }
            return Ok(());
        };
        let request = lane.latest.as_ref().ok_or(SnapshotError::CombatMismatch)?;
        let result = &receipt.result;
        let valid = self.checkpoint_owner(lane.entity) == Some(receipt.account)
            && result.character_id.as_uuid() == lane.entity.as_uuid()
            && result.skill.id.0 > 0
            && result.skill.rank > 0
            && result.tick < self.tick
            && receipt.fingerprint
                == format!(
                    "v1|{}|{}|{}",
                    result.character_id, result.skill.id.0, result.skill.rank
                )
            && request.phase3.is_some()
            && request.phase4.is_some()
            && request.class_state.as_ref().is_some_and(|ledger| {
                ledger.learned_skills.iter().any(|skill| {
                    skill.key == format!("l2.skill.{}", result.skill.id.0)
                        && skill.level >= result.skill.rank
                })
            })
            && matches!(learned.as_slice(), [DomainEvent::CharacterSkillLearned {character_id, skill_id, skill_level, sp_spent, sp_remaining, tick, request_key, metadata}]
                if *character_id == result.character_id && *skill_id == result.skill.id.0
                    && *skill_level == result.skill.rank && *sp_spent == result.sp_spent
                    && *sp_remaining == result.sp_remaining && *tick == result.tick.0
                    && *request_key == receipt.key && request.revision_seen.checked_add(1) == Some(metadata.sequence.0));
        if valid {
            Ok(())
        } else {
            Err(SnapshotError::CombatMismatch)
        }
    }
}

/// One zone's authoritative simulation state. Single owner (the zone actor); no interior
/// mutability, no locks, no clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneState {
    // Opt-in replay diagnostics only: never snapshot, hash, encode or send these facts.
    evidence: Option<super::evidence::EvidenceBuffer>,
    phase3: Option<super::phase3::Phase3Zone>,
    phase4: Option<super::phase4::Phase4Zone>,
    classes: Option<Arc<crate::domain::class::ClassRegistry>>,
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
    known: BTreeMap<EntityId, (CellCoord, Arc<[EntityId]>)>,
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
            classes: None,
            phase3: None,
            phase4: None,
            evidence: None,
            checkpoints: Vec::new(),
            meta: SnapshotMeta::legacy(),
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

    /// Constructs a new Phase 3 epoch explicitly with all three rule bundles.
    /// Explicit experimental runtime; service bootstrap remains fenced until Phase 3
    /// durability and the complete handler schedule have passed their integration gates.
    pub fn new_phase3(
        seed: ZoneSeed,
        bounds: ZoneBounds,
        time_origin_ms: i64,
        rules: Arc<StatRules>,
        classes: Arc<crate::domain::class::ClassRegistry>,
        phase3_rules: super::Phase3Rules,
    ) -> Result<Self, SnapshotError> {
        if phase3_rules.runtime_revision() > 2 {
            return Err(SnapshotError::Rules(
                "V6 skill bundle requires an explicit V6 epoch".into(),
            ));
        }
        Self::new_skill_epoch(seed, bounds, time_origin_ms, rules, classes, phase3_rules)
    }

    fn new_skill_epoch(
        seed: ZoneSeed,
        bounds: ZoneBounds,
        time_origin_ms: i64,
        rules: Arc<StatRules>,
        classes: Arc<crate::domain::class::ClassRegistry>,
        phase3_rules: super::Phase3Rules,
    ) -> Result<Self, SnapshotError> {
        Self::validate_damage_profiles_v5(&phase3_rules)?;
        phase3_rules
            .validate_catalogue(&classes)
            .map_err(SnapshotError::Rules)?;
        phase3_rules
            .geometry()
            .peace_square(bounds)
            .map_err(|error| SnapshotError::Rules(error.to_string()))?;
        phase3_rules
            .profiles()
            .starter_weapon
            .validate_physical(rules.starter_weapon())
            .map_err(|error| SnapshotError::Rules(error.to_string()))?;
        let mut state = Self::new(seed, bounds, time_origin_ms)
            .with_rules(rules)
            .with_classes(classes)?;
        let canonical = super::Phase3Snapshot::new(phase3_rules);
        if !canonical.validate() {
            return Err(SnapshotError::CombatMismatch);
        }
        state.phase3 = Some(
            super::phase3::Phase3Zone::from_canonical(canonical)
                .map_err(|error| SnapshotError::Rules(error.to_string()))?,
        );
        state.meta.schema_version = 9;
        state.meta.digest_version = StateDigestVersion::BinaryV5;
        state.meta.phase3_admission_policy = Some(super::Phase3AdmissionPolicy::version_one());
        Ok(state)
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
        Self::from_snapshot_with_items(snapshot, None)
    }

    /// Restore V6 with explicitly resolved immutable item rules. Historical snapshots must
    /// pass None; neither restoration path upgrades a recorded epoch.
    pub fn from_snapshot_with_items(
        snapshot: ZoneSnapshot,
        rules: Option<Arc<super::Phase4Rules>>,
    ) -> Result<Self, SnapshotError> {
        snapshot.validate_version()?;
        let phase4 = match (snapshot.phase4.clone(), rules) {
            (None, None) => None,
            (Some(canonical), Some(rules)) => {
                canonical
                    .validate_allocations(
                        &rules,
                        snapshot
                            .phase3
                            .as_ref()
                            .ok_or(SnapshotError::CombatMismatch)?,
                    )
                    .map_err(|e| SnapshotError::Rules(e.to_string()))?;
                Some(super::phase4::Phase4Zone { canonical, rules })
            },
            _ => {
                return Err(SnapshotError::Rules("explicit matching V6 item rules required".into()))
            },
        };
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
        state.phase4 = phase4;
        state.meta = snapshot.meta;
        state.classes = snapshot.classes;
        state.phase3 = snapshot
            .phase3
            .map(super::phase3::Phase3Zone::from_canonical)
            .transpose()
            .map_err(|error| SnapshotError::Rules(error.to_string()))?;
        let expected_hash = state.classes.as_deref().map(registry_hash).transpose()?;
        if state.meta.classes_rules_hash != expected_hash {
            return Err(SnapshotError::CombatMismatch);
        }
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
            if !state.phase3_enabled() {
                state.validate_class_entity(&e)?;
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
        if state.phase3_enabled() {
            state.validate_runtime_v5()?;
        }
        let players: Vec<(EntityId, Vec2Fixed)> = state
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::Player)
            .map(|e| (e.id, e.pos))
            .collect();
        let mut cells = BTreeMap::new();
        for (id, pos) in players {
            let cell = CellCoord::of(pos);
            let ids = cells.entry(cell).or_insert_with(|| {
                let mut ids: Vec<EntityId> = state.aoi.in_aoi(pos).collect();
                ids.sort_unstable();
                Arc::<[EntityId]>::from(ids)
            });
            state.known.insert(id, (cell, Arc::clone(ids)));
        }
        Ok(state)
    }

    /// The state as plain data. Only meaningful at a tick boundary, which is the only time
    /// the owner can call it.
    #[must_use]
    pub fn snapshot(&self) -> ZoneSnapshot {
        ZoneSnapshot {
            phase4: self.phase4.as_ref().map(|p| p.canonical.clone()),
            phase3: self.phase3.as_ref().map(|p| p.canonical.clone()),
            classes: self.classes.clone(),
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
        if !self.phase4_enabled() && draft.commands.iter().any(|c| c.command.requires_phase4()) {
            return Err(TickError::Phase4CommandInLegacy);
        }
        for command in &draft.commands {
            if let ZoneCommand::SpawnPlayer {
                load: Some(load), ..
            } = &command.command
            {
                load.validate_phase4()
                    .map_err(|_| TickError::Phase4CommandMalformed)?;
            }
            if let ZoneCommand::Item(item) = &command.command {
                item.authenticated_account()
                    .map_err(|_| TickError::Phase4CommandMalformed)?;
            }
        }
        if self.phase3_enabled() {
            self.run_tick_v5(draft)
        } else {
            if draft.commands.iter().any(|c| c.command.requires_phase3()) {
                return Err(TickError::Phase3CommandInLegacy);
            }
            self.run_tick_legacy(draft)
        }
    }

    /// Immutable recorded byte admission inputs; absent only for legacy epochs.
    #[must_use]
    pub fn phase3_admission_policy(&self) -> Option<&super::Phase3AdmissionPolicy> {
        self.meta.phase3_admission_policy.as_ref()
    }

    /// Policy is selected only by the validated epoch version, never registry presence.
    pub fn phase3_enabled(&self) -> bool {
        matches!(
            (self.meta.schema_version, self.meta.digest_version),
            (9, StateDigestVersion::BinaryV5) | (10, StateDigestVersion::BinaryV6)
        )
    }

    fn run_tick_legacy(&mut self, draft: AppliedTickDraft) -> Result<AppliedTick, TickError> {
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
            digest_version: self.meta.digest_version,
        })
    }

    /// Versioned SHA-256 of everything that changes between ticks (the immutable
    /// rules, bounds and seed are fixed by the snapshot): next tick and ordinal, generator
    /// position, entities, hate and the respawn scheduler, all in id order.
    #[must_use]
    pub fn state_digest(&self) -> [u8; 32] {
        match self.meta.digest_version {
            StateDigestVersion::JsonV1 => self.json_state_digest(),
            StateDigestVersion::BinaryV2 => self.binary_state_digest(),
            StateDigestVersion::BinaryV3 => self.phase2_state_digest(),
            StateDigestVersion::BinaryV4 => self.token_state_digest(),
            StateDigestVersion::BinaryV5 => self.phase3_state_digest(),
            StateDigestVersion::BinaryV6 => self.phase4_state_digest(),
        }
    }

    fn json_state_digest(&self) -> [u8; 32] {
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
        if (matches!(cmd, ZoneCommand::ChangeClass { .. })
            || (cmd.requires_phase3()
                && !matches!(
                    cmd,
                    ZoneCommand::SpawnPlayer { .. } | ZoneCommand::MaterializePendingItems(_)
                )))
            && source == CommandSource::System
        {
            return Err(RejectReason::NotPermitted);
        }
        let CommandSource::Session { entity, generation } = source else {
            return Ok(());
        };
        match cmd {
            ZoneCommand::Item(command) if command.entity == entity => {
                match self.entities.get(&entity) {
                    Some(e) if e.generation != generation => Err(RejectReason::StaleSession),
                    Some(_) | None => Ok(()),
                }
            },

            ZoneCommand::CastSkill { entity: target, .. }
            | ZoneCommand::CancelCast { entity: target, .. }
            | ZoneCommand::AcceptResurrect { entity: target, .. }
            | ZoneCommand::LearnSkill { entity: target, .. }
            | ZoneCommand::ChangeClass { entity: target, .. }
            | ZoneCommand::MoveTo { entity: target, .. }
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
            ZoneCommand::MaterializePendingItems(_)
            | ZoneCommand::ExtendItemRange { .. }
            | ZoneCommand::Item(_)
            | ZoneCommand::CastSkill { .. }
            | ZoneCommand::CancelCast { .. }
            | ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::LearnSkill { .. }
            | ZoneCommand::ChangeClass { .. }
            | ZoneCommand::MoveTo { .. }
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
    #[allow(clippy::too_many_lines)] // Exhaustive command dispatch keeps mutation order visible.
    fn apply(&mut self, tick: Tick, c: &AppliedCommand) -> Result<Vec<ZoneEvent>, RejectReason> {
        self.authorize(c.source, &c.command)?;
        self.refuse_dead_actor(c.source, &c.command)?;
        match &c.command {
            // Only the V5 dispatcher may execute these; legacy run_tick rejects the entire
            // draft before mutation. Keep this dispatcher exhaustive while rollout is fenced.
            ZoneCommand::MaterializePendingItems(_)
            | ZoneCommand::ExtendItemRange { .. }
            | ZoneCommand::Item(_)
            | ZoneCommand::CastSkill { .. }
            | ZoneCommand::CancelCast { .. }
            | ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::LearnSkill { .. } => Err(RejectReason::NotYetImplemented),
            ZoneCommand::ChangeClass {
                entity,
                account,
                request_key,
                target,
            } => self.change_class(tick, *entity, *account, *request_key, *target),
            ZoneCommand::SpawnPlayer {
                entity,
                name,
                pos,
                speed,
                generation,
                load,
            } => {
                let combat = match (&self.rules, load) {
                    (Some(rules), Some(load)) => Some(self.loaded_player(rules, load, *entity)?),
                    _ => None,
                };
                let mut events = self.spawn(
                    tick,
                    Spawned {
                        id: *entity,
                        kind: EntityKind::Player,
                        name,
                        pos: *pos,
                        speed: match (self.classes.as_deref(), load.as_deref()) {
                            (Some(registry), Some(load)) => {
                                let p =
                                    load.progression.as_ref().ok_or(RejectReason::InvalidLoad)?;
                                super::profession_stats(
                                    self.rules.as_deref().ok_or(RejectReason::InvalidLoad)?,
                                    registry,
                                    &p.identity,
                                    &p.class_state,
                                    load.level,
                                )
                                .map_err(|_| RejectReason::InvalidLoad)?
                                .2
                            },
                            _ => *speed,
                        },
                        generation: *generation,
                        combat,
                    },
                )?;
                self.reconcile_player_tokens(tick, *entity, None, &mut events);
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
            ZoneCommand::Item(command) => {
                if self
                    .entities
                    .get(&command.entity)
                    .is_some_and(|e| e.targeting.dead)
                {
                    Err(RejectReason::DeadActor)
                } else {
                    Ok(())
                }
            },
            ZoneCommand::CastSkill { entity, .. }
            | ZoneCommand::LearnSkill { entity, .. }
            | ZoneCommand::MoveTo { entity, .. }
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
            ZoneCommand::CancelCast { .. }
            | ZoneCommand::AcceptResurrect { .. }
            | ZoneCommand::ChangeClass { .. }
            | ZoneCommand::Respawn { .. }
            | ZoneCommand::Despawn { .. }
            | ZoneCommand::SpawnPlayer { .. }
            | ZoneCommand::SpawnNpc { .. }
            | ZoneCommand::ReplaceSession { .. }
            | ZoneCommand::AddAggro { .. }
            | ZoneCommand::MaterializePendingItems(_)
            | ZoneCommand::ExtendItemRange { .. } => Ok(()),
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
                phase3: None,
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
                if let Some(ledger) = self.hate.remove(&id) {
                    for (source, row) in ledger.iter() {
                        self.evidence_hate_removal(id, source, row, "despawn");
                    }
                }
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
            let v5_player = self
                .phase3
                .as_ref()
                .is_some_and(|p| p.canonical.rules.runtime_revision() >= 2)
                && selected.kind == EntityKind::Player
                && selected.combat.is_some();
            if !v5_player
                && (selected.kind == EntityKind::Player
                    || selected.targeting.dead
                    || !selected.targeting.attackable)
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
            phase3: None,
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
    /// Players share visible ids and facts per current cell, and AOI deltas per
    /// (previous cell, current cell). Joins and replacement sessions share the empty
    /// previous view. Owner-only facts merge with public facts in original event order.
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
        let mut facts = CellFacts::by_cell(entities, events);
        let mut views: BTreeMap<CellCoord, CellView<'_>> = BTreeMap::new();
        let mut outputs = BTreeMap::new();
        for player in entities.values().filter(|e| e.kind == EntityKind::Player) {
            let source = CommandSource::Session {
                entity: player.id,
                generation: player.generation,
            };
            // Keyed by the full source, so a fenced (older) session's responses never reach
            // the current one.
            let mut out = responses.remove(&source).unwrap_or_default();
            let cell = CellCoord::of(player.pos);
            let view = views.entry(cell).or_insert_with(|| {
                CellView::build(
                    &self.aoi,
                    entities,
                    player.pos,
                    &moved,
                    tick,
                    facts.remove(&cell).unwrap_or_default(),
                )
            });
            let previous = self.known.get(&player.id);
            let facts = view
                .facts
                .public
                .len()
                .saturating_add(view.facts.owned.get(&player.id).map_or(0, Vec::len));
            let delta = view.delta(previous, tick, entities);
            out.reserve(delta.len().saturating_add(facts));
            out.extend(delta.iter().cloned());
            self.known.insert(player.id, (cell, Arc::clone(&view.ids)));
            // A spawn precedes every fact referring to a newly visible entity.
            view.facts_into(player.id, &mut out);
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
#[cfg(test)]
fn fact_visible(event: &ZoneEvent, observer: EntityId, known: &[EntityId]) -> bool {
    let sees = |id: &EntityId| *id == observer || known.binary_search(id).is_ok();
    match event {
        ZoneEvent::ItemResult(result) => result.entity == observer,
        ZoneEvent::ItemView(view) => {
            if view.owner_only() {
                view.entity == observer
            } else {
                sees(&view.entity)
            }
        },
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
        ZoneEvent::ClassChanged { entity, .. }
        | ZoneEvent::EntityDied { entity, .. }
        | ZoneEvent::EntityRespawned { entity, .. } => sees(entity),
        ZoneEvent::EffectApplied { .. }
        | ZoneEvent::EffectRemoved { .. }
        | ZoneEvent::SkillCastStarted { .. }
        | ZoneEvent::SkillCastResult { .. }
        | ZoneEvent::SkillCastCancelled { .. }
        | ZoneEvent::PvpStateChanged { .. }
        | ZoneEvent::ResurrectOffer { .. }
        | ZoneEvent::SkillStateSync { .. }
        | ZoneEvent::SkillLearned { .. }
        | ZoneEvent::SpGained { .. }
        | ZoneEvent::SkillReuse { .. }
        | ZoneEvent::VitalsChanged { .. }
        | ZoneEvent::TokensReconciled { .. }
        | ZoneEvent::ClassTransfer { .. }
        | ZoneEvent::HateChanged { .. }
        | ZoneEvent::NpcIntentionChanged { .. }
        | ZoneEvent::ItemCheckpoint(_)
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

/// Facts routed once to occupied observer cells. Entries keep the original event index
/// so private and shared facts can be merged without changing causal order.
#[derive(Default)]
struct CellFacts<'a> {
    public: Vec<(usize, &'a ZoneEvent)>,
    owned: BTreeMap<EntityId, Vec<(usize, &'a ZoneEvent)>>,
}

impl<'a> CellFacts<'a> {
    fn by_cell(
        entities: &BTreeMap<EntityId, Entity>,
        events: &'a [ZoneEvent],
    ) -> BTreeMap<CellCoord, Self> {
        let mut cells: BTreeMap<_, Self> = entities
            .values()
            .filter(|e| e.kind == EntityKind::Player)
            .map(|e| (CellCoord::of(e.pos), Self::default()))
            .collect();
        for (index, event) in events.iter().enumerate() {
            let (entity, other, private) = match event {
                ZoneEvent::ItemResult(result) => (result.entity, None, true),
                ZoneEvent::ItemView(view) => (view.entity, None, view.owner_only()),
                ZoneEvent::TargetChanged { entity, target, .. } => (*entity, *target, true),
                ZoneEvent::StatsChanged { entity, .. }
                | ZoneEvent::XpGained { entity, .. }
                | ZoneEvent::LevelUp { entity, .. } => (*entity, None, true),
                ZoneEvent::AttackResult {
                    attacker, target, ..
                }
                | ZoneEvent::AttackStarted {
                    attacker, target, ..
                }
                | ZoneEvent::AttackCancelled {
                    attacker, target, ..
                } => (*attacker, Some(*target), false),
                ZoneEvent::ClassChanged { entity, .. }
                | ZoneEvent::EntityDied { entity, .. }
                | ZoneEvent::EntityRespawned { entity, .. } => (*entity, None, false),
                ZoneEvent::EffectApplied { .. }
                | ZoneEvent::EffectRemoved { .. }
                | ZoneEvent::SkillCastStarted { .. }
                | ZoneEvent::SkillCastResult { .. }
                | ZoneEvent::SkillCastCancelled { .. }
                | ZoneEvent::PvpStateChanged { .. }
                | ZoneEvent::ResurrectOffer { .. }
                | ZoneEvent::SkillStateSync { .. }
                | ZoneEvent::SkillLearned { .. }
                | ZoneEvent::SpGained { .. }
                | ZoneEvent::SkillReuse { .. }
                | ZoneEvent::VitalsChanged { .. }
                | ZoneEvent::TokensReconciled { .. }
                | ZoneEvent::ClassTransfer { .. }
                | ZoneEvent::HateChanged { .. }
                | ZoneEvent::NpcIntentionChanged { .. }
                | ZoneEvent::ItemCheckpoint(_)
                | ZoneEvent::Progression(_)
                | ZoneEvent::EntitySpawn { .. }
                | ZoneEvent::EntityMove { .. }
                | ZoneEvent::EntityDespawn { .. } => continue,
            };
            let Some(first) = entities.get(&entity) else {
                continue;
            };
            let origin = CellCoord::of(first.pos);
            let other_cell = match other {
                Some(id) => {
                    let Some(e) = entities.get(&id) else { continue };
                    Some(CellCoord::of(e.pos))
                },
                None => None,
            };
            let sees_other = |cell: CellCoord| {
                other_cell.is_none_or(|other| {
                    cell.x.abs_diff(other.x) <= 1 && cell.y.abs_diff(other.y) <= 1
                })
            };
            if private {
                if first.kind == EntityKind::Player && sees_other(origin) {
                    if let Some(facts) = cells.get_mut(&origin) {
                        facts.owned.entry(entity).or_default().push((index, event));
                    }
                }
            } else {
                // AOI is symmetric: only these nine observer cells can see the entity.
                for cell in origin
                    .neighbourhood()
                    .into_iter()
                    .filter(|c| sees_other(*c))
                {
                    if let Some(facts) = cells.get_mut(&cell) {
                        facts.public.push((index, event));
                    }
                }
            }
        }
        cells
    }
}

/// One cell's AOI at the end of a tick: who is visible (sorted) and the moves among them.
struct CellView<'a> {
    ids: Arc<[EntityId]>,
    moves: Vec<(EntityId, ObserverOutput)>,
    deltas: BTreeMap<Option<CellCoord>, Vec<ObserverOutput>>,
    facts: CellFacts<'a>,
}

impl<'a> CellView<'a> {
    fn build(
        aoi: &AoiIndex,
        entities: &BTreeMap<EntityId, Entity>,
        pos: Vec2Fixed,
        moved: &[EntityId],
        tick: Tick,
        facts: CellFacts<'a>,
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
            ids: Arc::<[EntityId]>::from(ids),
            moves: move_list,
            deltas: BTreeMap::new(),
            facts,
        }
    }

    fn delta(
        &mut self,
        previous: Option<&(CellCoord, Arc<[EntityId]>)>,
        tick: Tick,
        entities: &BTreeMap<EntityId, Entity>,
    ) -> &[ObserverOutput] {
        let key = previous.map(|(cell, _)| *cell);
        // At a boundary, all observers from a cell share the identical known list.
        // Missing entries mean a new or replaced session, whose previous AOI is empty.
        if !self.deltas.contains_key(&key) {
            let before = previous.map_or(&[][..], |(_, ids)| ids.as_ref());
            let mut delta = Vec::new();
            if before == self.ids.as_ref() {
                delta.extend(self.moves.iter().map(|(_, output)| output.clone()));
            } else {
                diff_into(&mut delta, tick, before, self, entities);
            }
            self.deltas.insert(key, delta);
        }
        self.deltas.get(&key).map_or(&[], Vec::as_slice)
    }

    fn facts_into(&self, player: EntityId, out: &mut Vec<ObserverOutput>) {
        let mut owned = self
            .facts
            .owned
            .get(&player)
            .into_iter()
            .flatten()
            .peekable();
        for (index, event) in &self.facts.public {
            while let Some((_, event)) = owned.next_if(|(i, _)| i < index) {
                out.push(ObserverOutput::Event((*event).clone()));
            }
            out.push(ObserverOutput::Event((*event).clone()));
        }
        out.extend(owned.map(|(_, event)| ObserverOutput::Event((*event).clone())));
    }
}

/// Appends despawns (`before` minus `now`), spawns (`now` minus `before`) and moves of
/// entities in both, each in id order. Both id lists are sorted.
fn diff_into(
    out: &mut Vec<ObserverOutput>,
    tick: Tick,
    before: &[EntityId],
    now: &CellView<'_>,
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
        identity: e.combat.as_ref().and_then(|c| match &c.role {
            CombatRole::Player { progression, .. } => progression.as_ref().map(|p| p.public()),
            CombatRole::Npc { .. } => None,
        }),
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
            progression: load.progression.clone(),
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
        class: match &c.role {
            CombatRole::Player { progression, .. } => progression.as_ref().map(|p| p.stats()),
            CombatRole::Npc { .. } => None,
        },
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

#[path = "state_class.rs"]
mod class_phase;

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

// Arc keeps admitted-boundary snapshots cheap; serialized content remains the full catalogue.
#[allow(clippy::ref_option)] // serde serialize_with requires a reference to the field.
fn serialize_registry<S: serde::Serializer>(
    registry: &Option<Arc<crate::domain::class::ClassRegistry>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    registry.as_deref().serialize(serializer)
}
fn deserialize_registry<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Arc<crate::domain::class::ClassRegistry>>, D::Error> {
    Option::<crate::domain::class::ClassRegistry>::deserialize(deserializer)
        .map(|registry| registry.map(Arc::new))
}
fn registry_hash(
    registry: &crate::domain::class::ClassRegistry,
) -> Result<[u8; 32], SnapshotError> {
    // Resolved integer rules and ordered collections; computed only at bootstrap/restore.
    let bytes = serde_json::to_vec(registry).map_err(|e| SnapshotError::Rules(e.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

impl ZoneSnapshot {
    fn validate_skill_boundary(&self, runtime: u32, policy: u32) -> bool {
        self.meta
            .phase3_admission_policy
            .is_some_and(|p| p.version == policy && p.is_valid())
            && self
                .phase3
                .as_ref()
                .is_some_and(|p| p.rules.runtime_revision() <= runtime && p.validate())
            && self
                .rules
                .as_ref()
                .zip(self.phase3.as_ref())
                .is_some_and(|(stats, p)| {
                    p.rules
                        .profiles()
                        .starter_weapon
                        .validate_physical(&stats.starter_weapon)
                        .is_ok()
                })
            && self
                .phase3
                .as_ref()
                .is_some_and(|p| p.rules.geometry().peace_square(self.bounds).is_ok())
            && self
                .classes
                .as_deref()
                .zip(self.phase3.as_ref())
                .is_some_and(|(classes, p)| p.rules.validate_catalogue(classes).is_ok())
    }
}
