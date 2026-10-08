use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::ports::RepositoryError;
use crate::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use crate::domain::{AccountId, Character, CharacterId, DomainEvent};

#[derive(Default)]
struct State {
    characters: HashMap<CharacterId, Character>,
    names: HashMap<String, CharacterId>,
    /// `(account, key)`: keys are scoped to the account, like the Postgres primary key.
    keys: HashMap<(AccountId, IdempotencyKey), (String, CharacterId)>,
    /// Events staged with each create, standing in for the `outbox` table.
    outbox: Vec<DomainEvent>,
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

    /// Reads a character directly. Test helper.
    #[must_use]
    pub fn get_for_test(&self, id: CharacterId) -> Option<Character> {
        self.state.lock().characters.get(&id).cloned()
    }

    /// Events staged for publication, in order (what the Postgres adapter writes to `outbox`).
    #[must_use]
    pub fn staged_events(&self) -> Vec<DomainEvent> {
        self.state.lock().outbox.clone()
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

    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>> {
        let mut out: Vec<Character> = self
            .state
            .lock()
            .characters
            .values()
            .filter(|c| c.account_id == account)
            .cloned()
            .collect();
        // Ids are uuid v7, so id order is creation order (the Postgres adapter sorts the same).
        out.sort_by_key(|c| c.id.as_uuid());
        Ok(out)
    }

    async fn create_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        let mut s = self.state.lock();
        let scoped = (character.account_id, *key);
        if let Some((stored_fp, id)) = s.keys.get(&scoped) {
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
            .insert(scoped, (fingerprint.to_owned(), character.id));
        s.characters.insert(character.id, character.clone());
        s.outbox.push(DomainEvent::CharacterCreated {
            character_id: character.id,
            account_id: character.account_id,
            race: character.race,
        });
        Ok(CreateOutcome::Created(character.clone()))
    }
}
