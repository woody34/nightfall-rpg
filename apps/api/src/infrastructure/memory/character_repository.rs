use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::ports::RepositoryError;
use crate::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use crate::domain::{Character, CharacterId};

#[derive(Default)]
struct State {
    characters: HashMap<CharacterId, Character>,
    names: HashMap<String, CharacterId>,
    keys: HashMap<IdempotencyKey, (String, CharacterId)>,
}

/// Map-backed repository guarded by one mutex, so each method is atomic like a transaction.
#[derive(Default)]
pub struct InMemoryCharacterRepository {
    state: Mutex<State>,
}

impl InMemoryCharacterRepository {
    /// Seeds a character directly, bypassing idempotency. Test helper.
    pub fn insert_for_test(&self, c: Character) {
        let mut s = self.state.lock();
        s.names.insert(c.name.normalized(), c.id);
        s.characters.insert(c.id, c);
    }

    /// Number of stored characters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.lock().characters.len()
    }

    /// True when nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl CharacterRepository for InMemoryCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        Ok(self.state.lock().characters.get(&id).cloned())
    }

    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        let mut s = self.state.lock();
        if let Some((stored_fp, id)) = s.keys.get(key) {
            if stored_fp != fingerprint {
                return Ok(CreateOutcome::KeyReused);
            }
            let existing = s.characters.get(id).cloned().ok_or_else(|| {
                RepositoryError::Other(anyhow::anyhow!(
                    "idempotency key points at missing character"
                ))
            })?;
            return Ok(CreateOutcome::Replayed(existing));
        }
        let norm = character.name.normalized();
        if s.names.contains_key(&norm) {
            return Err(RepositoryError::NameTaken);
        }
        s.names.insert(norm, character.id);
        s.keys
            .insert(key.clone(), (fingerprint.to_owned(), character.id));
        s.characters.insert(character.id, character.clone());
        Ok(CreateOutcome::Created(character.clone()))
    }
}
