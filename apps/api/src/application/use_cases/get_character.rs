//! Read a character by id.

use std::sync::Arc;

use crate::application::{AppError, CharacterRepository};
use crate::domain::{Character, CharacterId};

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

    /// Executes. `raw_id` is parsed here so the transport stays thin.
    pub async fn execute(&self, raw_id: &str) -> Result<Character, AppError> {
        let id: CharacterId = raw_id
            .parse()
            .map_err(|_| AppError::InvalidArgument("character_id must be a UUID".into()))?;
        self.characters
            .get(id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: raw_id.to_owned(),
            })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{CharacterName, Race};
    use crate::infrastructure::memory::InMemoryCharacterRepository;

    #[tokio::test]
    async fn returns_existing_character() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let c = Character::create(Uuid::nil(), CharacterName::new("Bob").unwrap(), Race::Dwarf);
        repo.insert_for_test(c.clone());
        let got = GetCharacter::new(repo)
            .execute(&c.id.to_string())
            .await
            .unwrap();
        assert_eq!(got, c);
    }

    #[tokio::test]
    async fn not_found_for_unknown_id() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let err = GetCharacter::new(repo)
            .execute(&Uuid::nil().to_string())
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
        let err = GetCharacter::new(repo).execute("abc").await.unwrap_err();
        assert!(matches!(err, AppError::InvalidArgument(_)));
    }
}
