//! Ports: the traits the application layer needs the outside world to implement.

mod item_grants;
pub use item_grants::*;
mod item_mutations;
pub use item_mutations::*;
mod items;
pub use items::*;

use crate::domain::character_progression::{CharacterIdentity, ClassState, FrozenTransferResult};
use crate::domain::class::ClassId;
use crate::domain::ids::uuid_id;
use crate::domain::CharacterName;
use crate::domain::{
    AccountId, Character, CharacterId, DomainEvent, PlayTicket, Position, SessionGeneration,
    SessionId, TicketHash,
};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

pub use super::replay_log::{EventLog, ZoneSnapshotStore};

uuid_id!(
    /// Client-supplied key that makes a mutating request safe to retry. Must be a UUID.
    IdempotencyKey
);

impl IdempotencyKey {
    /// Parses the wire form. Empty or non-UUID input is rejected.
    pub fn parse(raw: &str) -> Result<Self, IdempotencyKeyError> {
        if raw.is_empty() {
            return Err(IdempotencyKeyError::Missing);
        }
        raw.parse().map_err(|_| IdempotencyKeyError::Invalid)
    }
}

/// Why an idempotency key was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdempotencyKeyError {
    /// The field was empty.
    #[error("idempotency_key is required")]
    Missing,
    /// The field was not a UUID.
    #[error("idempotency_key must be a UUID")]
    Invalid,
}

/// Result of an idempotent create.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateOutcome {
    /// First time this key was seen: the entity was persisted.
    Created(Character),
    /// Key seen before with the same fingerprint: the original entity is returned, nothing
    /// was written.
    Replayed(Character),
    /// Key seen before with a different fingerprint: the caller is reusing a key for a
    /// different request.
    KeyReused,
}

/// Persistence for characters. Every method is one atomic unit of work.
#[async_trait]
pub trait CharacterRepository: Send + Sync {
    /// Fetches by id.
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>>;

    /// Every character owned by `account`, in creation order (ids are time-ordered).
    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>>;

    /// Atomically: records `key` (scoped to `character.account_id` and the operation
    /// `create_character`) with `fingerprint`, inserts `character`, and stages its
    /// creation event for publication. If `key` already exists, writes nothing and returns
    /// [`CreateOutcome::Replayed`] or [`CreateOutcome::KeyReused`].
    ///
    /// # Errors
    /// `Err(RepositoryError::NameTaken)` when the normalized name is already in use by a
    /// different character. Any other failure is infrastructure.
    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError>;

    /// Forever-retained successful transfer lookup, scoped to the authenticated account.
    /// The default reads the bounded character ledgers; adapters may use an indexed receipt.
    async fn mutation_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<MutationReceiptLookup> {
        for character in self.list_by_account(caller).await? {
            if let Some(receipt) = character.class_state.receipt(key.as_uuid()) {
                return Ok(
                    if transfer_fingerprint(receipt.result.character_id, receipt.target_class_id)
                        == fingerprint
                    {
                        MutationReceiptLookup::Known(receipt.result.clone())
                    } else {
                        MutationReceiptLookup::Conflict
                    },
                );
            }
        }
        Ok(MutationReceiptLookup::Unknown)
    }

    /// The committed progression state, read when a character is admitted to a zone.
    /// `Ok(None)` when the character does not exist.
    async fn load_for_admission(
        &self,
        character_id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>>;

    /// One transaction: verifies `checkpoint.revision_seen` equals the stored revision, writes
    /// level/xp/hp/mp/alive/position, bumps the revision, stages `events` in the outbox, and
    /// records `checkpoint.idempotency` with the resulting revision as its response.
    ///
    /// A known key is checked first: same body replays the stored outcome
    /// ([`CheckpointOutcome::Replayed`]), different body is [`CheckpointError::KeyReused`].
    /// A stale `revision_seen` returns [`CheckpointOutcome::Stale`] and writes nothing.
    async fn checkpoint(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError>;

    /// T11: durably quarantine an owner whose checkpoint lane failed permanently while an
    /// unresolved epoch was recovered. Idempotent (the first record wins). Admission refuses a
    /// quarantined owner with `character_lane_quarantined` until an operator resolves it.
    async fn quarantine_lane(&self, quarantine: &LaneQuarantine) -> anyhow::Result<()> {
        anyhow::bail!("lane quarantine is unavailable in this repository: {}", quarantine.reason)
    }

    /// T11 operations: the quarantined owners recorded for `zone`, by character id. The boot
    /// summary and the runbook (running-locally.md, "Quarantined lanes") read this.
    async fn lane_quarantines(
        &self,
        zone: crate::domain::zone::ZoneId,
    ) -> anyhow::Result<Vec<LaneQuarantine>> {
        let _ = zone;
        Ok(Vec::new())
    }
}

/// A permanent per-owner recovery quarantine (T11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneQuarantine {
    /// The owner.
    pub character_id: CharacterId,
    /// The recovered zone.
    pub zone: crate::domain::zone::ZoneId,
    /// The recovered epoch.
    pub epoch: u64,
    /// The recorded tick at which the lane failed.
    pub tick: crate::domain::zone::Tick,
    /// The named failure.
    pub reason: String,
}

/// The named admission refusal for a quarantined owner.
pub const LANE_QUARANTINED: &str = "character_lane_quarantined";

/// The typed admission refusal for a quarantined owner (T11). Its text is
/// `character_lane_quarantined: <reason>`; callers downcast it from the `anyhow` load error to
/// refuse the owner by name instead of as a generic load failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {}", LANE_QUARANTINED, .reason)]
pub struct LaneQuarantined {
    /// The first recorded failure of the quarantined lane.
    pub reason: String,
}

/// Level range the `characters.level` check constraint enforces.
pub const LEVEL_RANGE: std::ops::RangeInclusive<u32> = 1..=85;

/// What a zone needs from the database to admit a character.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressionState {
    /// Position read atomically with progression and its revision.
    pub position: Position,
    /// Current level (1..=85).
    pub level: u32,
    /// Cumulative experience.
    pub xp: u64,
    /// Current HP; `None` means full (the stat engine knows the maximum).
    pub hp: Option<u32>,
    /// Current MP; `None` means full.
    pub mp: Option<u32>,
    /// False while dead; stays false across reconnects until a respawn checkpoint.
    pub alive: bool,
    /// Class profile id (`packages/data/classes/<id>.toml`).
    pub class_profile: String,
    /// Immutable identity read atomically with resources and revision.
    pub identity: CharacterIdentity,
    /// Immutable display name for frozen responses.
    pub name: CharacterName,
    /// Active main class ledger, including successful transfer receipts.
    pub class_state: ClassState,
    /// Explicit paused Phase 3 payload. None identifies legacy state.
    pub phase3: Option<crate::domain::zone::Phase3Checkpoint>,
    /// Complete recorded V6 admission input, populated only with the matching runtime bundle.
    pub phase4: Option<crate::domain::zone::Phase4PlayerLoad>,
    /// Bumped by every applied checkpoint; the fence for the next one.
    pub revision: u64,
}

/// A full persistence checkpoint for one character (plan §3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterCheckpoint {
    /// Whose checkpoint.
    pub character_id: CharacterId,
    /// The revision the zone loaded or last saw acknowledged.
    pub revision_seen: u64,
    /// Level.
    pub level: u32,
    /// Cumulative experience.
    pub xp: u64,
    /// Current HP.
    pub hp: u32,
    /// Current MP.
    pub mp: u32,
    /// False when dead.
    pub alive: bool,
    /// Where the character is.
    pub position: Position,
    /// `(operation, key)` of the idempotency record; operation matches `^[a-z][a-z_]*$`.
    pub idempotency: (String, IdempotencyKey),
    /// None is legacy recovery: preserve ledger and original checkpoint fingerprint bytes.
    pub class_state: Option<ClassState>,
    /// Strict V6 retained item state; absent on all legacy requests and fingerprints.
    pub phase4: Option<crate::domain::zone::Phase4Checkpoint>,
    /// Explicit V5 retained state; absent on all legacy requests and fingerprints.
    pub phase3: Option<crate::domain::zone::Phase3Checkpoint>,
}

impl CharacterCheckpoint {
    /// Stable digest of everything except the idempotency key: what "same body" means for a
    /// retry. Both adapters use it, so they agree on replay versus [`CheckpointError::KeyReused`].
    #[must_use]
    pub fn fingerprint(&self, events: &[DomainEvent]) -> String {
        use sha2::{Digest, Sha256};
        let mut body = serde_json::json!({
            "character_id": self.character_id.as_uuid(),
            "revision_seen": self.revision_seen,
            "level": self.level,
            "xp": self.xp,
            "hp": self.hp,
            "mp": self.mp,
            "alive": self.alive,
            "position": [self.position.x, self.position.y],
            "events": events,
        });
        if let (Some(class_state), Some(object)) = (&self.class_state, body.as_object_mut()) {
            object.insert("class_state".into(), serde_json::json!(class_state));
        }
        if let (Some(phase3), Some(object)) = (&self.phase3, body.as_object_mut()) {
            object.insert("phase3".into(), serde_json::json!(phase3));
        }
        if let (Some(phase4), Some(object)) = (&self.phase4, body.as_object_mut()) {
            object.insert("phase4".into(), serde_json::json!(phase4));
        }
        Sha256::digest(body.to_string().as_bytes())
            .iter()
            .fold(String::new(), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            })
    }

    /// Mirrors the `characters` check constraints so the in-memory adapter rejects what
    /// Postgres would. Returns the violated constraint's name.
    #[must_use]
    pub fn violated_constraint(&self) -> Option<&'static str> {
        if self.phase4.as_ref().is_some_and(|p| {
            self.phase3.as_ref().is_none_or(|phase3| {
                self.alive != (self.hp > 0 || phase3.resource_fractions.hp > 0)
                    || p.validate_structure(phase3.modifier_install_order.len())
                        .is_err()
            })
        }) {
            Some("characters_phase4_state_valid")
        } else if self.phase3.as_ref().is_some_and(|p| {
            self.class_state.is_none()
                || p.timer_policy_version != 1
                || p.death_penalty_level > 5
                || p.resource_fractions.validate().is_err()
                || i64::try_from(p.flag_remaining_ticks).is_err()
                || p.reuse_remainders
                    .values()
                    .any(|v| *v <= 50 || i64::try_from(*v).is_err())
                || p.effect_remainders
                    .iter()
                    .any(|v| v.remaining_ticks <= 100 || i64::try_from(v.remaining_ticks).is_err())
                || p.last_death.as_ref().is_some_and(|d| {
                    i64::try_from(d.pre_death_xp).is_err()
                        || d.recoverable_death_xp_loss > d.pre_death_xp
                })
                || (!self.alive
                    && (self.hp != 0
                        || p.resource_fractions
                            != crate::domain::zone::ResourceFractions::default()))
        }) {
            Some("characters_phase3_state_valid")
        } else if !LEVEL_RANGE.contains(&self.level) {
            Some("characters_level_range")
        } else if i64::try_from(self.xp).is_err() {
            Some("characters_xp_nonneg")
        } else if i32::try_from(self.hp).is_err() {
            Some("characters_hp_nonneg")
        } else if i32::try_from(self.mp).is_err() {
            Some("characters_mp_nonneg")
        } else if self.class_state.as_ref().is_some_and(|s| {
            s.validate().is_err()
                || i64::try_from(s.sp).is_err()
                || i32::try_from(s.cp).is_err()
                || i32::try_from(s.token_tier_1_count).is_err()
                || i32::try_from(s.token_tier_2_count).is_err()
        }) {
            Some("characters_class_state_valid")
        } else {
            None
        }
    }
}

/// Result of [`CharacterRepository::checkpoint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointOutcome {
    /// Written; the character is now at this revision.
    Applied(u64),
    /// `revision_seen` did not match the stored revision; nothing was written.
    Stale,
    /// Same key and body as an applied checkpoint: its stored revision, nothing written.
    Replayed(u64),
}

/// Why a checkpoint failed.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    /// The key was used before with a different body.
    #[error("idempotency key reused with a different checkpoint")]
    KeyReused,
    /// No such character.
    #[error("character not found")]
    NotFound,
    /// A value violates the named database constraint (e.g. `characters_level_range`).
    #[error("checkpoint violates constraint {0}")]
    Constraint(String),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Repository failures the use case distinguishes.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    /// Unique constraint on the normalized name.
    #[error("character name already taken")]
    NameTaken,
    /// Seven character slots already occupied. Existing keys replay before this check.
    #[error("all seven character slots are occupied")]
    SlotsFull,
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A successful transfer's immutable receipt lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationReceiptLookup {
    /// No successful request used the key.
    Unknown,
    /// Key used for a different character or target.
    Conflict,
    /// Exact original response; returned before requiring a live session.
    Known(FrozenTransferResult),
}

/// Semantic transfer body; account scoping is provided by the repository.
#[must_use]
pub fn transfer_fingerprint(character: CharacterId, target: ClassId) -> String {
    format!("v1|{character}|{}", target.0)
}

/// One live eligibility option, including user-visible unmet requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct TransferEligibility {
    pub class_id: ClassId,
    pub eligible: bool,
    pub unmet: Vec<String>,
}

/// Live read produced by the actor, never by independently reading mutable DB progression.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct TransferOptionsState {
    pub options: Vec<TransferEligibility>,
    pub current_class_id: ClassId,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
}

/// Authenticated transfer routing. The implementation captures generation with the session
/// lifecycle lock, releases it before waiting, and replies only after durable checkpoint.
#[async_trait]
pub trait ClassTransferRuntime: Send + Sync {
    /// Read actor-owned eligibility for the caller's live character.
    async fn transfer_options(
        &self,
        caller: AccountId,
        character: CharacterId,
    ) -> Result<TransferOptionsState, super::AppError>;
    /// Applies the deterministic command and waits outside its command/log for durability.
    async fn change_class(
        &self,
        caller: AccountId,
        character: CharacterId,
        target: ClassId,
        key: IdempotencyKey,
    ) -> Result<FrozenTransferResult, super::AppError>;
}

/// Account-scoped operation for paid learning receipts (api-guidelines section 2).
pub const LEARN_SKILL_OPERATION: &str = "learn_skill";

/// Exact semantic request. Rank is never inferred again on a retry.
#[must_use]
pub fn learn_skill_fingerprint(
    character: CharacterId,
    skill: crate::domain::zone::skill::SkillRef,
) -> String {
    format!("v1|{character}|{}|{}", skill.id.0, skill.rank)
}

/// Shared compiled domain DTOs; receipts remain outside canonical simulation state.
pub use crate::domain::zone::{FrozenLearnResult, LearnedEffectiveStats};

// Preserve the existing application constructor while sharing Z's exact receipt DTO.
#[allow(clippy::derivable_impls)] // The DTO declaration is owned by the zone engine.
impl Default for LearnedEffectiveStats {
    fn default() -> Self {
        Self {
            p_atk_q: 0,
            p_def_q: 0,
            m_atk_q: 0,
            m_def_q: 0,
            attack_speed: 0,
            cast_speed: 0,
            speed_q: 0,
            accuracy_q: 0,
            evasion_q: 0,
            crit_permille: 0,
        }
    }
}

/// Receipt lookup must use `idempotency_keys`, never transfer history or simulation state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearnReceiptLookup {
    /// No committed success for the account/operation/key.
    Unknown,
    /// Key used for a different character, skill or rank.
    Conflict,
    /// Exact original owner-only result, valid while offline.
    Known(FrozenLearnResult),
}

/// Durable paid-learning receipt reads, scoped to the caller and `learn_skill` operation.
#[async_trait]
pub trait LearnReceiptRepository: Send + Sync {
    /// Lookup precedes live-generation and gameplay checks. Infrastructure failure is fatal.
    async fn learn_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<LearnReceiptLookup>;
}

/// Frozen success carried outside simulation state into the checkpoint transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SuccessfulLearnReceipt {
    pub account_id: AccountId,
    pub key: IdempotencyKey,
    pub request_fingerprint: String,
    pub result: FrozenLearnResult,
}

impl SuccessfulLearnReceipt {
    /// Checks the request/result binding and signed database bounds. Final checkpoint state
    /// may be later within the same tick; the receipt remains the mutation's frozen response.
    pub fn validate(
        &self,
        checkpoint: &CharacterCheckpoint,
        owner: AccountId,
    ) -> Result<(), CheckpointError> {
        let result = &self.result;
        if self.account_id != owner
            || result.character_id != checkpoint.character_id
            || result.skill.id.0 == 0
            || result.skill.rank == 0
            || self.request_fingerprint
                != learn_skill_fingerprint(result.character_id, result.skill)
            || i64::try_from(result.sp_spent).is_err()
            || i64::try_from(result.sp_remaining).is_err()
            || checkpoint.class_state.as_ref().is_none_or(|ledger| {
                !ledger.learned_skills.iter().any(|skill| {
                    skill.key == format!("l2.skill.{}", result.skill.id.0)
                        && skill.level >= result.skill.rank
                })
            })
        {
            return Err(CheckpointError::Constraint("character_learn_receipt_valid".into()));
        }
        Ok(())
    }

    /// Extends only learning checkpoint fingerprints. Ordinary legacy fingerprints stay exact.
    pub fn checkpoint_fingerprint(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> anyhow::Result<String> {
        use sha2::{Digest as _, Sha256};
        let body =
            serde_json::to_vec(&("learn-checkpoint-v1", checkpoint.fingerprint(events), self))?;
        Ok(format!("{:x}", Sha256::digest(body)))
    }
}

/// A learning commit either writes a fenced checkpoint or settles against an earlier receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LearningCheckpointOutcome {
    /// Applied/replayed checkpoint revision or stale fence. Stale consumes no receipt/key.
    Checkpoint(CheckpointOutcome),
    /// Another committed account/key won; return this exact result and write nothing.
    ReceiptReplayed(FrozenLearnResult),
}

/// Atomic final progression/ledger/outbox/receipt writer, called only after durable log ack.
#[async_trait]
pub trait LearningCheckpointRepository: LearnReceiptRepository {
    /// Receipt claim/recheck and checkpoint revision fence are one transaction. Failed/stale
    /// requests roll back every child row, receipt, checkpoint key and outbox fact.
    async fn checkpoint_learning(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        receipt: &SuccessfulLearnReceipt,
    ) -> Result<LearningCheckpointOutcome, CheckpointError>;
}

/// One next-rank learning option computed from final actor-owned state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs, clippy::struct_excessive_bools)] // Independent flags required by the wire contract.
pub struct LearnableSkill {
    pub skill: crate::domain::class::SkillLearnDef,
    pub effect_implemented: bool,
    pub learned: bool,
    pub affordable: bool,
    pub trainer_required: bool,
    pub eligible: bool,
    pub unmet: Vec<String>,
}

/// Live query response; database progression must never stand in for actor state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct LearnableSkillsState {
    pub skills: Vec<LearnableSkill>,
    pub current_class_id: ClassId,
    pub sp: u64,
}

/// RPC reply handle remains outside deterministic command and replay payloads.
pub type LearnSkillReply =
    tokio::sync::oneshot::Receiver<Result<FrozenLearnResult, super::AppError>>;

/// Generation-fenced Phase 3 routing. Trainer/gameplay checks belong to the zone actor.
#[async_trait]
pub trait Phase3Runtime: Send + Sync {
    /// Capture generation under the session lifecycle lock; never hold that lock awaiting a reply.
    /// Offline/stale lanes are `FAILED_PRECONDITION`; a stopped zone is UNAVAILABLE.
    async fn generation(
        &self,
        caller: AccountId,
        character: CharacterId,
    ) -> Result<SessionGeneration, super::AppError>;

    /// Read next ranks (required level <= current level) from live state, checking the captured
    /// account/character/generation again at execution. Includes honest unmet reasons.
    async fn learnable_skills(
        &self,
        caller: AccountId,
        character: CharacterId,
        generation: SessionGeneration,
    ) -> Result<LearnableSkillsState, super::AppError>;

    /// Enqueue account/character/generation/key/exact skill rank. Recheck the durable receipt
    /// before drafting; serialize learn/transfer and lifecycle barriers per character/tick.
    /// Success is sent only after log ack + atomic final checkpoint/receipt/outbox ack.
    /// Duplicate queued keys settle against that receipt; failed requests spend nothing.
    async fn enqueue_learn_skill(
        &self,
        caller: AccountId,
        character: CharacterId,
        generation: SessionGeneration,
        key: IdempotencyKey,
        skill: crate::domain::zone::skill::SkillRef,
    ) -> Result<LearnSkillReply, super::AppError>;
}

/// Source operate classification, independent of runtime capability or eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SkillOperateType {
    ActiveInstant,
    ActiveInstantContinuous,
    ActiveContinuous,
    Passive,
    Toggle,
    Other,
}

/// Read-only catalogue entry backed by the independently validated registry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct SkillCatalogueEntry {
    pub skill_id: crate::domain::zone::skill::SkillId,
    pub key: String,
    pub display_name: String,
    pub max_level: u32,
    pub effect_implemented: bool,
    pub deferred_reasons: Vec<String>,
    pub l2_ref: String,
    /// Validated base values in rank order (index = rank - 1), before stat modifiers.
    pub mp_consume_start: Vec<u32>,
    pub mp_consume_end: Vec<u32>,
    pub hp_consume: Vec<u32>,
    /// Source milliseconds, before stat scaling and tick conversion.
    pub reuse_ms: Vec<u32>,
    pub hit_time_ms: Vec<u32>,
    pub operate_type: SkillOperateType,
    /// Exact registry strings, expanded in rank order, including unsupported values.
    pub target_type: Vec<String>,
    pub affect_scope: Vec<String>,
}

/// Immutable catalogue in skill-id order; metadata availability does not authorize gameplay.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct SkillCatalogue {
    pub data_version: String,
    pub skills: Vec<SkillCatalogueEntry>,
}

/// Startup registry adapter. Requests never read or parse data files.
pub trait SkillCataloguePort: Send + Sync {
    /// Missing catalogue is UNAVAILABLE; corrupt registry is INTERNAL.
    fn catalogue(&self) -> Result<SkillCatalogue, super::AppError>;
}

/// Verifies bearer tokens issued by the identity provider (plan Revision 1, item 15).
#[async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Verifies `token` (the part after `Bearer `): signature, algorithm, issuer, audience,
    /// expiry. Returns the claims the game uses.
    async fn verify(&self, token: &str) -> Result<Claims, AuthError>;
}

/// What the game reads from a verified access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    /// Subject: the identity provider's user id, which is the [`AccountId`].
    pub sub: String,
    /// Audiences the token was issued for (contains the configured audience).
    pub aud: Vec<String>,
    /// Realm roles (`player`, `gm`, ...).
    pub roles: Vec<String>,
    /// Expiry, Unix seconds.
    pub exp: i64,
}

/// Why a token was rejected. Every variant except [`AuthError::Unavailable`] is the caller's
/// problem and maps to `UNAUTHENTICATED`.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Not a JWT.
    #[error("malformed bearer token")]
    Malformed,
    /// Past `exp` (plus leeway).
    #[error("token expired")]
    Expired,
    /// Signed with a key id the issuer does not publish.
    #[error("token signed by an unknown key")]
    UnknownKey,
    /// Failed verification: signature, algorithm, issuer, audience, or a missing claim.
    #[error("invalid token: {0}")]
    Invalid(&'static str),
    /// The verifier could not do its job (keys unreachable). Not the caller's fault.
    #[error("token verification unavailable")]
    Unavailable(#[source] anyhow::Error),
}

/// Result of recording a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginOutcome {
    /// First login: the account row was created.
    Created,
    /// Known account; `last_login_at` was moved forward.
    Bumped,
    /// Known account, seen within the bump interval: nothing was written.
    Unchanged,
}

/// Persistence for accounts. Every method is one atomic unit of work.
#[async_trait]
pub trait AccountRepository: Send + Sync {
    /// Creates the account if it does not exist (`created_at = last_login_at = now`);
    /// otherwise sets `last_login_at = now` only if the stored value is at least
    /// `min_interval` older. Concurrent first logins create exactly one row.
    async fn record_login(
        &self,
        id: AccountId,
        now: DateTime<Utc>,
        min_interval: chrono::Duration,
    ) -> anyhow::Result<LoginOutcome>;
}

/// What `IssuePlayTicket` returns, and what is stored under its idempotency key so a retry
/// gets the identical response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedTicket {
    /// The secret. Its `Debug` is redacted.
    pub ticket: PlayTicket,
    /// Instant after which the ticket is rejected.
    pub expires_at: DateTime<Utc>,
    /// Where the client connects.
    pub ws_url: String,
}

/// A ticket to persist, with everything the issue transaction needs.
#[derive(Debug, Clone)]
pub struct NewTicket {
    /// The caller; owns `character_id`.
    pub account_id: AccountId,
    /// The character the ticket admits.
    pub character_id: CharacterId,
    /// SHA-256 of the secret; the only form stored in the ticket table.
    pub hash: TicketHash,
    /// Issue instant.
    pub issued_at: DateTime<Utc>,
    /// The full response, stored for idempotent replay.
    pub response: IssuedTicket,
}

/// Result of an idempotent ticket issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueOutcome {
    /// New ticket stored under the account's new session generation.
    Issued {
        /// The response to return.
        response: IssuedTicket,
        /// The generation the ticket carries.
        generation: SessionGeneration,
    },
    /// Same key, same fingerprint: the stored response, nothing written.
    Replayed(IssuedTicket),
    /// Same key, different fingerprint.
    KeyReused,
}

/// A consumed ticket: who may connect, as which character, under which generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    /// The authenticated account.
    pub account_id: AccountId,
    /// The character to spawn.
    pub character_id: CharacterId,
    /// Fences older sessions of the same account (plan Revision 1, item 8).
    pub generation: SessionGeneration,
}

/// Why a ticket could not be consumed.
#[derive(Debug, thiserror::Error)]
pub enum ConsumeError {
    /// No ticket with this hash.
    #[error("unknown ticket")]
    Unknown,
    /// Already used once.
    #[error("ticket already consumed")]
    Consumed,
    /// Past its expiry.
    #[error("ticket expired")]
    Expired,
    /// A newer ticket was issued for the same account. The ticket is consumed regardless.
    #[error("ticket superseded by a newer one")]
    Superseded,
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Persistence for play tickets and session generations. Every method is one transaction.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Atomically: claims `key` (scoped to the account and the operation
    /// `issue_play_ticket`) with `fingerprint` and the full response, bumps the account's
    /// session generation, and stores the ticket hash with that generation. A known key
    /// writes nothing and returns [`IssueOutcome::Replayed`] or [`IssueOutcome::KeyReused`].
    async fn issue_ticket_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        ticket: &NewTicket,
    ) -> anyhow::Result<IssueOutcome>;

    /// Atomically marks the ticket consumed and returns its admission. Concurrent consumers of
    /// one ticket: exactly one succeeds.
    ///
    /// # Errors
    /// [`ConsumeError::Unknown`], [`ConsumeError::Consumed`], [`ConsumeError::Expired`]
    /// (`expires_at <= now`), or [`ConsumeError::Superseded`] when the ticket's generation is
    /// older than the account's current one.
    async fn consume_ticket(
        &self,
        hash: &TicketHash,
        now: DateTime<Utc>,
    ) -> Result<Admission, ConsumeError>;
}

/// Source of secrets. Tests substitute a fixed one so a sentinel value can be traced.
pub trait SecretGenerator: Send + Sync {
    /// A fresh play ticket from a cryptographically secure source.
    fn play_ticket(&self) -> anyhow::Result<PlayTicket>;
}

/// Outbound domain events. Implementations must be safe to call after a transaction commits
/// and must tolerate duplicate delivery (consumers are idempotent).
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publishes one event on its subject.
    async fn publish(&self, event: &DomainEvent) -> anyhow::Result<()>;
}

/// Zone position sampled on the session path, for audit only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAuditContext {
    /// The socket's zone.
    pub zone: crate::domain::zone::ZoneId,
    /// The running zone epoch.
    pub epoch: u64,
    /// Next tick at receive time, or the tick whose output is queued.
    pub tick: crate::domain::zone::Tick,
}

impl Default for SessionAuditContext {
    fn default() -> Self {
        Self {
            zone: crate::domain::zone::ZoneId(0),
            epoch: 0,
            tick: crate::domain::zone::Tick(0),
        }
    }
}

/// The per-session audit log of the real-time channel (plan D5, §8 #3): every inbound frame
/// as received and every outbound frame as sent, in order, per session.
///
/// Called on the socket path, so implementations must not block or await: buffer and drop
/// (counting the loss) rather than slow a session down. The zone's applied-tick log, not this,
/// is what replay re-runs; this log is for audit and for picking which session's output a
/// replay verifies. Story 3.2 supplies the `JetStream` adapter
/// (`nightfall.session.<id>.in` / `.out`); `InMemorySessionAudit` serves tests and dev.
pub trait SessionAudit: Send + Sync {
    /// A committed save acknowledgement; audit-only, never a socket frame.
    fn record_checkpoint(&self, _session: SessionId, _ack: &super::checkpoint::CheckpointAck) {}

    /// One inbound frame, as received, before any validation. `seq` is the decoded
    /// `ClientMessage.seq`, or `None` when the frame could not be decoded (oversize,
    /// malformed, not binary).
    fn record_in(&self, session: SessionId, seq: Option<u32>, frame: &Bytes);

    /// One outbound frame, in the order the session queued it for the socket.
    fn record_out(&self, session: SessionId, frame: &Bytes);

    /// Inbound recording with zone metadata. Defaults to the original port for adapters
    /// that do not store replay-log metadata. This receives game data only, never WS auth.
    fn record_in_context(
        &self,
        session: SessionId,
        seq: Option<u32>,
        frame: &Bytes,
        _context: SessionAuditContext,
    ) {
        self.record_in(session, seq, frame);
    }

    /// Outbound recording with the producing tick (the latest zone tick for a session-local
    /// rejection). Called after reserving queue space and before releasing it to the socket.
    fn record_out_context(&self, session: SessionId, frame: &Bytes, _context: SessionAuditContext) {
        self.record_out(session, frame);
    }
}

/// Wall clock, abstracted so time-dependent use cases are deterministic in tests.
pub trait Clock: Send + Sync {
    /// Current UTC time.
    fn now(&self) -> DateTime<Utc>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn idempotency_key_rejects_empty_and_garbage() {
        assert_eq!(IdempotencyKey::parse("").unwrap_err(), IdempotencyKeyError::Missing);
        assert_eq!(IdempotencyKey::parse("nope").unwrap_err(), IdempotencyKeyError::Invalid);
        let k = IdempotencyKey::parse("0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e").unwrap();
        assert_eq!(k.to_string(), "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e");
    }
}
