//! Create a character. Idempotent on the client key; atomic in the repository; publishes
//! `CharacterCreated` after the write commits.

use std::sync::Arc;

use uuid::Uuid;

use crate::application::ports::RepositoryError;
use crate::application::{AppError, CharacterRepository, CreateOutcome, EventBus, IdempotencyKey};
use crate::domain::{Character, CharacterName, DomainEvent, Race};

/// Validated input. Built by the transport adapter from the wire request.
#[derive(Debug, Clone)]
pub struct CreateCharacterInput {
    /// Client retry key.
    pub idempotency_key: IdempotencyKey,
    /// Owning account.
    pub account_id: Uuid,
    /// Raw name; validated here.
    pub name: String,
    /// Chosen race.
    pub race: Race,
}

/// The create-character use case.
pub struct CreateCharacter {
    characters: Arc<dyn CharacterRepository>,
    bus: Arc<dyn EventBus>,
}

impl CreateCharacter {
    /// Builds the use case.
    #[must_use]
    pub fn new(characters: Arc<dyn CharacterRepository>, bus: Arc<dyn EventBus>) -> Self {
        Self { characters, bus }
    }

    /// Executes. Returns the character, whether created now or replayed from a previous call
    /// with the same key.
    pub async fn execute(&self, input: CreateCharacterInput) -> Result<Character, AppError> {
        let name = CharacterName::new(input.name)?;
        let character = Character::create(input.account_id, name, input.race);
        let fingerprint = Self::fingerprint(&character);

        let outcome = self
            .characters
            .create_idempotent(&input.idempotency_key, &fingerprint, &character)
            .await
            .map_err(|e| match e {
                RepositoryError::NameTaken => {
                    AppError::AlreadyExists(format!("character name {}", character.name))
                },
                RepositoryError::Other(e) => AppError::Infrastructure(e),
            })?;

        match outcome {
            CreateOutcome::Created(c) => {
                // The write is committed. Publication is best-effort here; the outbox row the
                // repository staged is the durable path (docs/engineering/architecture.md §3).
                let event = DomainEvent::CharacterCreated {
                    character_id: c.id,
                    account_id: c.account_id,
                    race: c.race,
                };
                if let Err(e) = self.bus.publish(&event).await {
                    tracing::warn!(error = %e, character_id = %c.id, "event publish failed; outbox will retry");
                }
                Ok(c)
            },
            CreateOutcome::Replayed(c) => {
                tracing::info!(key = %input.idempotency_key, "create_character replayed");
                Ok(c)
            },
            CreateOutcome::KeyReused => Err(AppError::IdempotencyConflict),
        }
    }

    /// What makes two requests "the same request" for idempotency purposes. Excludes the
    /// generated id, so a retry fingerprints identically.
    fn fingerprint(c: &Character) -> String {
        format!("v1|{}|{}|{}", c.account_id, c.name.normalized(), c.race.as_str())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::infrastructure::memory::{InMemoryCharacterRepository, InMemoryEventBus};

    fn sut() -> (CreateCharacter, Arc<InMemoryCharacterRepository>, Arc<InMemoryEventBus>) {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let bus = Arc::new(InMemoryEventBus::default());
        (CreateCharacter::new(repo.clone(), bus.clone()), repo, bus)
    }

    fn input(key: &str, name: &str) -> CreateCharacterInput {
        CreateCharacterInput {
            idempotency_key: IdempotencyKey::parse(key).unwrap(),
            account_id: Uuid::nil(),
            name: name.to_owned(),
            race: Race::Elf,
        }
    }

    const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
    const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

    #[tokio::test]
    async fn creates_and_publishes_event() {
        let (uc, repo, bus) = sut();
        let c = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        assert_eq!(c.name.as_str(), "Legolas");
        assert_eq!(repo.len(), 1);
        let events = bus.published();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], DomainEvent::CharacterCreated { character_id, .. } if *character_id == c.id)
        );
    }

    #[tokio::test]
    async fn same_key_replays_without_second_write_or_event() {
        let (uc, repo, bus) = sut();
        let first = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let second = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(repo.len(), 1);
        assert_eq!(bus.published().len(), 1);
    }

    #[tokio::test]
    async fn same_key_different_body_is_a_conflict() {
        let (uc, _, _) = sut();
        uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let err = uc.execute(input(KEY_A, "Gimli")).await.unwrap_err();
        assert!(matches!(err, AppError::IdempotencyConflict));
    }

    #[tokio::test]
    async fn duplicate_name_with_new_key_is_already_exists() {
        let (uc, _, _) = sut();
        uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let err = uc.execute(input(KEY_B, "LEGOLAS")).await.unwrap_err();
        assert!(matches!(err, AppError::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn invalid_name_is_rejected_before_any_port_call() {
        let (uc, repo, bus) = sut();
        let err = uc.execute(input(KEY_A, "x")).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
        assert_eq!(repo.len(), 0);
        assert!(bus.published().is_empty());
    }
}
