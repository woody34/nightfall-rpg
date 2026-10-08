//! List the caller's characters (Story 1.6).

use std::sync::Arc;

use crate::application::{AppError, CharacterRepository};
use crate::domain::{AccountId, Character};

/// The list-my-characters use case.
pub struct ListMyCharacters {
    characters: Arc<dyn CharacterRepository>,
}

impl ListMyCharacters {
    /// Builds the use case.
    #[must_use]
    pub fn new(characters: Arc<dyn CharacterRepository>) -> Self {
        Self { characters }
    }

    /// Executes. `caller` comes from the verified token; an account with no characters gets
    /// an empty list.
    pub async fn execute(&self, caller: AccountId) -> Result<Vec<Character>, AppError> {
        Ok(self.characters.list_by_account(caller).await?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{CharacterName, Race};
    use crate::infrastructure::memory::InMemoryCharacterRepository;

    fn account(n: u128) -> AccountId {
        AccountId::from_uuid(Uuid::from_u128(n))
    }

    #[tokio::test]
    async fn returns_only_the_callers_characters_in_creation_order() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let a1 = Character::create(account(1), CharacterName::new("Alpha").unwrap(), Race::Elf);
        let b1 = Character::create(account(2), CharacterName::new("Bravo").unwrap(), Race::Orc);
        let a2 = Character::create(account(1), CharacterName::new("Charlie").unwrap(), Race::Orc);
        for c in [&a2, &b1, &a1] {
            repo.insert_for_test(c.clone());
        }

        let got = ListMyCharacters::new(repo)
            .execute(account(1))
            .await
            .unwrap();
        assert_eq!(got, vec![a1, a2]);
    }

    #[tokio::test]
    async fn account_without_characters_gets_an_empty_list() {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let got = ListMyCharacters::new(repo)
            .execute(account(9))
            .await
            .unwrap();
        assert!(got.is_empty());
    }
}
