//! Read a character by id. Only its owner may read it (Story 1.6).

use std::sync::Arc;

use crate::application::{AppError, CharacterRepository};
use crate::domain::{AccountId, Character, CharacterId};

/// The get-character use case.
pub struct GetCharacter {
    characters: Arc<dyn CharacterRepository>,
}

impl GetCharacter {
    /// Builds the use case.
    #[must_use]
    pub fn new(characters: Arc<dyn CharacterRepository>) -> Self {
        Self { characters }
    }

    /// Executes. `caller` comes from the verified token; `raw_id` is parsed here so the
    /// transport stays thin.
    ///
    /// # Errors
    /// `InvalidArgument` for a malformed id, `NotFound`, or `PermissionDenied` when the
    /// character belongs to another account.
    pub async fn execute(&self, caller: AccountId, raw_id: &str) -> Result<Character, AppError> {
        let id: CharacterId = raw_id
            .parse()
            .map_err(|_| AppError::InvalidArgument("character_id must be a UUID".into()))?;
        let character = self
            .characters
            .get(id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: raw_id.to_owned(),
            })?;
        if character.account_id != caller {
            return Err(AppError::PermissionDenied(format!(
                "character {raw_id} belongs to another account"
            )));
        }
        Ok(character)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{CharacterName, Race};
    use crate::infrastructure::memory::InMemoryCharacterRepository;

    fn owner() -> AccountId {
        AccountId::from_uuid(Uuid::from_u128(1))
    }

    #[tokio::test]
    async fn returns_existing_character() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let c = Character::create(owner(), CharacterName::new("Bob").unwrap(), Race::Dwarf);
        repo.insert_for_test(c.clone());
        let got = GetCharacter::new(repo)
            .execute(owner(), &c.id.to_string())
            .await
            .unwrap();
        assert_eq!(got, c);
    }

    #[tokio::test]
    async fn another_accounts_character_is_permission_denied() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let c = Character::create(owner(), CharacterName::new("Bob").unwrap(), Race::Dwarf);
        repo.insert_for_test(c.clone());
        let intruder = AccountId::from_uuid(Uuid::from_u128(2));
        let err = GetCharacter::new(repo)
            .execute(intruder, &c.id.to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::PermissionDenied(_)), "{err:?}");
    }

    #[tokio::test]
    async fn not_found_for_unknown_id() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let err = GetCharacter::new(repo)
            .execute(owner(), &Uuid::nil().to_string())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            AppError::NotFound {
                entity: "character",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn invalid_argument_for_non_uuid() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let err = GetCharacter::new(repo)
            .execute(owner(), "abc")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
    }
}
