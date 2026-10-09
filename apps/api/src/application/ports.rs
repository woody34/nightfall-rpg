//! Ports: the traits the application layer needs the outside world to implement.

use crate::domain::ids::uuid_id;
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
}

/// Level range the `characters.level` check constraint enforces.
pub const LEVEL_RANGE: std::ops::RangeInclusive<u32> = 1..=85;

/// What a zone needs from the database to admit a character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressionState {
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
}

impl CharacterCheckpoint {
    /// Stable digest of everything except the idempotency key: what "same body" means for a
    /// retry. Both adapters use it, so they agree on replay versus [`CheckpointError::KeyReused`].
    #[must_use]
    pub fn fingerprint(&self, events: &[DomainEvent]) -> String {
        use sha2::{Digest, Sha256};
        let body = serde_json::json!({
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
        if !LEVEL_RANGE.contains(&self.level) {
            Some("characters_level_range")
        } else if i64::try_from(self.xp).is_err() {
            Some("characters_xp_nonneg")
        } else if i32::try_from(self.hp).is_err() {
            Some("characters_hp_nonneg")
        } else if i32::try_from(self.mp).is_err() {
            Some("characters_mp_nonneg")
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
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
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

/// The per-session audit log of the real-time channel (plan D5, §8 #3): every inbound frame
/// as received and every outbound frame as sent, in order, per session.
///
/// Called on the socket path, so implementations must not block or await: buffer and drop
/// (counting the loss) rather than slow a session down. The zone's applied-tick log, not this,
/// is what replay re-runs; this log is for audit and for picking which session's output a
/// replay verifies. Story 3.2 supplies the `JetStream` adapter
/// (`nightfall.session.<id>.in` / `.out`); `InMemorySessionAudit` serves tests and dev.
pub trait SessionAudit: Send + Sync {
    /// One inbound frame, as received, before any validation. `seq` is the decoded
    /// `ClientMessage.seq`, or `None` when the frame could not be decoded (oversize,
    /// malformed, not binary).
    fn record_in(&self, session: SessionId, seq: Option<u32>, frame: &Bytes);

    /// One outbound frame, in the order the session queued it for the socket.
    fn record_out(&self, session: SessionId, frame: &Bytes);
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
