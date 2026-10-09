//! Create a character. Idempotent on the client key; atomic in the repository; publishes
//! `CharacterCreated` in the outbox (inside the same transaction); the outbox relay is the only
//! publisher of domain events.

use std::sync::Arc;

use crate::application::ports::RepositoryError;
use crate::application::{AppError, CharacterRepository, CreateOutcome, IdempotencyKey};
use crate::domain::{AccountId, Character, CharacterName, Race};

/// Validated input. Built by the transport adapter from the wire request.
#[derive(Debug, Clone)]
pub struct CreateCharacterInput {
    /// Client retry key.
    pub idempotency_key: IdempotencyKey,
    /// Owning account: always the verified caller, never a request field.
    pub account_id: AccountId,
    /// Raw name; validated here.
    pub name: String,
    /// Chosen race.
    pub race: Race,
}

/// The create-character use case.
pub struct CreateCharacter {
    characters: Arc<dyn CharacterRepository>,
}

impl CreateCharacter {
    /// Builds the use case.
    #[must_use]
    pub fn new(characters: Arc<dyn CharacterRepository>) -> Self {
        Self { characters }
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
            CreateOutcome::Created(c) => Ok(c),
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
    use uuid::Uuid;

    use super::*;
    use crate::domain::DomainEvent;
    use crate::infrastructure::memory::InMemoryCharacterRepository;

    fn sut() -> (CreateCharacter, Arc<InMemoryCharacterRepository>) {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        (CreateCharacter::new(repo.clone()), repo)
    }

    fn input(key: &str, name: &str) -> CreateCharacterInput {
        CreateCharacterInput {
            idempotency_key: IdempotencyKey::parse(key).unwrap(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            name: name.to_owned(),
            race: Race::Elf,
        }
    }

    const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
    const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

    #[tokio::test]
    async fn creates_and_stages_event() {
        let (uc, repo) = sut();
        let c = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        assert_eq!(c.name.as_str(), "Legolas");
        assert_eq!(repo.len(), 1);
        let events = repo.staged_events();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], DomainEvent::CharacterCreated { character_id, .. } if *character_id == c.id)
        );
    }

    #[tokio::test]
    async fn same_key_replays_without_second_write_or_event() {
        let (uc, repo) = sut();
        let first = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let second = uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(repo.len(), 1);
        assert_eq!(repo.staged_events().len(), 1);
    }

    #[tokio::test]
    async fn same_key_different_body_is_a_conflict() {
        let (uc, _) = sut();
        uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let err = uc.execute(input(KEY_A, "Gimli")).await.unwrap_err();
        assert!(matches!(err, AppError::IdempotencyConflict));
    }

    #[tokio::test]
    async fn duplicate_name_with_new_key_is_already_exists() {
        let (uc, _) = sut();
        uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let err = uc.execute(input(KEY_B, "LEGOLAS")).await.unwrap_err();
        assert!(matches!(err, AppError::AlreadyExists(_)));
    }

    struct Down;

    #[async_trait::async_trait]
    impl CharacterRepository for Down {
        async fn get(&self, _id: crate::domain::CharacterId) -> anyhow::Result<Option<Character>> {
            anyhow::bail!("down")
        }

        async fn list_by_account(&self, _account: AccountId) -> anyhow::Result<Vec<Character>> {
            anyhow::bail!("down")
        }

        async fn create_idempotent(
            &self,
            _key: &IdempotencyKey,
            _fingerprint: &str,
            _character: &Character,
        ) -> Result<CreateOutcome, RepositoryError> {
            Err(RepositoryError::Other(anyhow::anyhow!("down")))
        }

        async fn load_for_admission(
            &self,
            _id: crate::domain::CharacterId,
        ) -> anyhow::Result<Option<crate::application::ProgressionState>> {
            anyhow::bail!("down")
        }

        async fn checkpoint(
            &self,
            _checkpoint: &crate::application::CharacterCheckpoint,
            _events: &[crate::domain::DomainEvent],
        ) -> Result<crate::application::CheckpointOutcome, crate::application::CheckpointError>
        {
            Err(crate::application::CheckpointError::Other(anyhow::anyhow!("down")))
        }
    }

    #[tokio::test]
    async fn repository_failure_is_an_infrastructure_error() {
        let uc = CreateCharacter::new(Arc::new(Down));
        let err = uc.execute(input(KEY_A, "Legolas")).await.unwrap_err();
        assert!(matches!(err, AppError::Infrastructure(_)), "{err:?}");
    }

    #[tokio::test]
    async fn same_key_from_another_account_is_independent() {
        let (uc, repo) = sut();
        uc.execute(input(KEY_A, "Legolas")).await.unwrap();
        let mut other = input(KEY_A, "Gimli");
        other.account_id = AccountId::from_uuid(Uuid::from_u128(2));
        let c = uc.execute(other).await.unwrap();
        assert_eq!(c.name.as_str(), "Gimli", "keys are scoped to the account");
        assert_eq!(repo.len(), 2);
    }

    #[tokio::test]
    async fn invalid_name_is_rejected_before_any_port_call() {
        let (uc, repo) = sut();
        let err = uc.execute(input(KEY_A, "x")).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
        assert_eq!(repo.len(), 0);
        assert!(repo.staged_events().is_empty());
    }
}
