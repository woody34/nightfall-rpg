//! Ports: the traits the application layer needs the outside world to implement.

use std::fmt;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::{Character, CharacterId, DomainEvent};

/// Client-supplied key that makes a mutating request safe to retry. Must be a UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(Uuid);

impl IdempotencyKey {
    /// Parses the wire form. Empty or non-UUID input is rejected.
    pub fn parse(raw: &str) -> Result<Self, IdempotencyKeyError> {
        if raw.is_empty() {
            return Err(IdempotencyKeyError::Missing);
        }
        Uuid::parse_str(raw)
            .map(Self)
            .map_err(|_| IdempotencyKeyError::Invalid)
    }

    /// Underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
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

    /// Atomically: records `key` with `fingerprint`, inserts `character`, and stages its
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

/// Outbound domain events. Implementations must be safe to call after a transaction commits
/// and must tolerate duplicate delivery (consumers are idempotent).
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publishes one event on its subject.
    async fn publish(&self, event: &DomainEvent) -> anyhow::Result<()>;
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
