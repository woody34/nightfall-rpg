use crate::domain::class::ClassRegistry;
use crate::domain::zone::{Phase3Checkpoint, Phase3Rules};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::ports::{transfer_fingerprint, RepositoryError};
use crate::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey, ProgressionState,
};
use crate::domain::character_progression::base_class_profile;
use crate::domain::{AccountId, Character, CharacterId, DomainEvent};

#[derive(Default)]
struct State {
    characters: HashMap<CharacterId, Character>,
    names: HashMap<String, CharacterId>,
    /// `(account, key)`: keys are scoped to the account, like the Postgres primary key.
    keys: HashMap<(AccountId, IdempotencyKey), (String, CharacterId)>,
    /// Events staged with each create, standing in for the `outbox` table.
    outbox: Vec<DomainEvent>,
    /// Progression columns that live beside the aggregate, keyed by character.
    progression: HashMap<CharacterId, ProgressionState>,
    /// `(account, operation, key)` -> `(fingerprint, revision)` for checkpoints.
    checkpoint_keys: HashMap<(AccountId, String, IdempotencyKey), (String, u64)>,
    learn_receipts: BTreeMap<
        (AccountId, IdempotencyKey),
        (String, crate::application::ports::FrozenLearnResult),
    >,
}

impl State {
    fn put(&mut self, c: &Character) {
        self.progression.insert(
            c.id,
            ProgressionState {
                phase3: None,
                position: c.position,
                level: c.level,
                xp: c.xp,
                hp: None,
                mp: None,
                alive: true,
                class_profile: base_class_profile(c.class_state.base_class_id)
                    .unwrap_or("invalid")
                    .to_owned(),
                identity: c.identity(),
                name: c.name.clone(),
                class_state: c.class_state.clone(),
                revision: 0,
            },
        );
    }
}

/// Map-backed repository guarded by one mutex, so each method is atomic like a transaction.
#[derive(Default)]
pub struct InMemoryCharacterRepository {
    state: Mutex<State>,
    classes: Option<Arc<ClassRegistry>>,
    phase3_rules: Option<Arc<Phase3Rules>>,
}

impl InMemoryCharacterRepository {
    /// Uses the same validated startup catalogue as creation and the zone.
    #[must_use]
    pub fn with_classes(mut self, classes: Arc<ClassRegistry>) -> Self {
        self.classes = Some(classes);
        self
    }
    /// Injects the full immutable V5 bundle used by the actor; no timer-only policy.
    #[must_use]
    pub fn with_phase3_rules(mut self, rules: Arc<Phase3Rules>) -> Self {
        self.phase3_rules = Some(rules);
        self
    }

    fn phase3_rules(&self) -> anyhow::Result<Arc<Phase3Rules>> {
        static RULES: std::sync::OnceLock<Result<Arc<Phase3Rules>, String>> =
            std::sync::OnceLock::new();
        if let Some(rules) = &self.phase3_rules {
            return Ok(rules.clone());
        }
        RULES
            .get_or_init(|| {
                crate::infrastructure::skill_data::load_phase3_rules(
                    &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
                    &[],
                )
                .map(Arc::new)
                .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(anyhow::Error::msg)
    }

    fn validate_versioned(
        &self,
        c: &Character,
        ledger: &crate::domain::character_progression::ClassState,
        phase3: Option<&Phase3Checkpoint>,
        alive: bool,
    ) -> anyhow::Result<()> {
        let registry =
            crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
        if let Some(phase3) = phase3 {
            let rules = self.phase3_rules()?;
            rules
                .validate_catalogue(&registry)
                .map_err(anyhow::Error::msg)?;
            phase3.validate(&rules, &ledger.learned_skills, alive)?;
            ledger.validate_for_phase3(&registry, &c.identity(), c.id)?;
        } else {
            ledger.validate_for(&registry, &c.identity(), c.id)?;
        }
        Ok(())
    }

    fn validate(&self, c: &Character) -> anyhow::Result<()> {
        let registry =
            crate::infrastructure::character_validation::registry(self.classes.as_ref())?;
        c.class_state.validate_for(&registry, &c.identity(), c.id)?;
        Ok(())
    }

    /// Seeds a character directly, bypassing idempotency. Test helper.
    pub fn insert_for_test(&self, c: Character) {
        let mut s = self.state.lock();
        s.names.insert(c.name.normalized(), c.id);
        s.put(&c);
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

impl InMemoryCharacterRepository {
    fn checkpoint_locked(
        &self,
        s: &mut State,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        fingerprint: String,
    ) -> Result<CheckpointOutcome, CheckpointError> {
        let account = s
            .characters
            .get(&checkpoint.character_id)
            .ok_or(CheckpointError::NotFound)?
            .account_id;
        let scoped = (account, checkpoint.idempotency.0.clone(), checkpoint.idempotency.1);
        if let Some((stored_fp, revision)) = s.checkpoint_keys.get(&scoped) {
            return if *stored_fp == fingerprint {
                Ok(CheckpointOutcome::Replayed(*revision))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        let current = s
            .progression
            .get(&checkpoint.character_id)
            .map(|p| p.revision)
            .ok_or(CheckpointError::NotFound)?;
        if current != checkpoint.revision_seen {
            return Ok(CheckpointOutcome::Stale);
        }
        if let Some(name) = checkpoint.violated_constraint() {
            return Err(CheckpointError::Constraint(name.to_owned()));
        }
        if checkpoint.phase3.is_none()
            && s.progression
                .get(&checkpoint.character_id)
                .is_some_and(|p| p.phase3.is_some())
        {
            return Err(CheckpointError::Constraint("characters_phase3_downgrade".into()));
        }
        if let Some(ledger) = &checkpoint.class_state {
            let character = s
                .characters
                .get(&checkpoint.character_id)
                .ok_or(CheckpointError::NotFound)?;
            self.validate_versioned(
                character,
                ledger,
                checkpoint.phase3.as_ref(),
                checkpoint.alive,
            )
            .map_err(|_| CheckpointError::Constraint("characters_class_state_valid".into()))?;
            if ledger.base_class_id != character.class_state.base_class_id {
                return Err(CheckpointError::Constraint("characters_base_class_immutable".into()));
            }
            for known in &character.class_state.learned_skills {
                if !ledger
                    .learned_skills
                    .iter()
                    .any(|skill| skill.key == known.key && skill.level >= known.level)
                {
                    return Err(CheckpointError::Constraint(
                        "character_learned_skills_monotone".into(),
                    ));
                }
            }
            for known in &character.class_state.successful_transfer_receipts {
                if ledger.receipt(known.key) != Some(known) {
                    return Err(CheckpointError::Constraint(
                        "character_transfer_receipts_immutable".into(),
                    ));
                }
            }
            for receipt in &ledger.successful_transfer_receipts {
                if receipt.result.character_id != checkpoint.character_id
                    || receipt.result.identity != character.identity()
                {
                    return Err(CheckpointError::Constraint(
                        "character_transfer_receipts_identity".into(),
                    ));
                }
                let fp = transfer_fingerprint(checkpoint.character_id, receipt.target_class_id);
                for other in s.characters.values().filter(|c| c.account_id == account) {
                    if let Some(known) = other.class_state.receipt(receipt.key) {
                        if fp
                            != transfer_fingerprint(
                                known.result.character_id,
                                known.target_class_id,
                            )
                            || known != receipt
                        {
                            return Err(CheckpointError::KeyReused);
                        }
                    }
                }
            }
        }
        let revision = current
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
        if let Some(p) = s.progression.get_mut(&checkpoint.character_id) {
            p.position = checkpoint.position;
            p.level = checkpoint.level;
            p.xp = checkpoint.xp;
            p.hp = Some(checkpoint.hp);
            p.mp = Some(checkpoint.mp);
            p.alive = checkpoint.alive;
            p.phase3.clone_from(&checkpoint.phase3);
            p.revision = revision;
            if let Some(ledger) = &checkpoint.class_state {
                p.class_state = ledger.clone();
            }
        }
        if let Some(c) = s.characters.get_mut(&checkpoint.character_id) {
            c.level = checkpoint.level;
            c.position = checkpoint.position;
            c.xp = checkpoint.xp;
            if let Some(ledger) = &checkpoint.class_state {
                c.class_state = ledger.clone();
            }
        }
        s.outbox.extend_from_slice(events);
        s.checkpoint_keys.insert(scoped, (fingerprint, revision));
        Ok(CheckpointOutcome::Applied(revision))
    }
}

#[async_trait]
impl crate::application::ports::LearnReceiptRepository for InMemoryCharacterRepository {
    async fn learn_receipt_lookup(
        &self,
        caller: AccountId,
        key: &IdempotencyKey,
        fingerprint: &str,
    ) -> anyhow::Result<crate::application::ports::LearnReceiptLookup> {
        use crate::application::ports::LearnReceiptLookup;
        Ok(match self.state.lock().learn_receipts.get(&(caller, *key)) {
            None => LearnReceiptLookup::Unknown,
            Some((stored, result)) if stored == fingerprint => {
                LearnReceiptLookup::Known(result.clone())
            },
            Some(_) => LearnReceiptLookup::Conflict,
        })
    }
}

#[async_trait]
impl crate::application::ports::LearningCheckpointRepository for InMemoryCharacterRepository {
    async fn checkpoint_learning(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
        receipt: &crate::application::ports::SuccessfulLearnReceipt,
    ) -> Result<crate::application::ports::LearningCheckpointOutcome, CheckpointError> {
        use crate::application::ports::LearningCheckpointOutcome;
        let mut state = self.state.lock();
        let owner = state
            .characters
            .get(&checkpoint.character_id)
            .ok_or(CheckpointError::NotFound)?
            .account_id;
        if receipt.account_id != owner {
            return Err(CheckpointError::Constraint("character_learn_receipt_valid".into()));
        }
        if let Some((stored, result)) = state.learn_receipts.get(&(receipt.account_id, receipt.key))
        {
            return if *stored == receipt.request_fingerprint {
                Ok(LearningCheckpointOutcome::ReceiptReplayed(result.clone()))
            } else {
                Err(CheckpointError::KeyReused)
            };
        }
        receipt.validate(checkpoint, owner)?;
        let fingerprint = receipt.checkpoint_fingerprint(checkpoint, events)?;
        let outcome = self.checkpoint_locked(&mut state, checkpoint, events, fingerprint)?;
        if matches!(outcome, CheckpointOutcome::Applied(_) | CheckpointOutcome::Replayed(_)) {
            state.learn_receipts.insert(
                (receipt.account_id, receipt.key),
                (receipt.request_fingerprint.clone(), receipt.result.clone()),
            );
        }
        Ok(LearningCheckpointOutcome::Checkpoint(outcome))
    }
}

#[async_trait]
impl CharacterRepository for InMemoryCharacterRepository {
    async fn get(&self, id: CharacterId) -> anyhow::Result<Option<Character>> {
        let state = self.state.lock();
        if let Some(c) = state.characters.get(&id) {
            let progression = state
                .progression
                .get(&id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                progression.phase3.as_ref(),
                progression.alive,
            )?;
        }
        Ok(state.characters.get(&id).cloned())
    }

    async fn list_by_account(&self, account: AccountId) -> anyhow::Result<Vec<Character>> {
        let state = self.state.lock();
        let mut out = Vec::new();
        for c in state
            .characters
            .values()
            .filter(|c| c.account_id == account)
        {
            let progression = state
                .progression
                .get(&c.id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                progression.phase3.as_ref(),
                progression.alive,
            )?;
            out.push(c.clone());
        }
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
        self.validate(character)?;
        let norm = character.name.normalized();
        if s.names.contains_key(&norm) {
            return Err(RepositoryError::NameTaken);
        }
        if s.characters
            .values()
            .filter(|c| c.account_id == character.account_id)
            .count()
            >= 7
        {
            return Err(RepositoryError::SlotsFull);
        }
        s.names.insert(norm, character.id);
        s.keys
            .insert(scoped, (fingerprint.to_owned(), character.id));
        s.put(character);
        s.characters.insert(character.id, character.clone());
        s.outbox.push(DomainEvent::CharacterCreated {
            character_id: character.id,
            account_id: character.account_id,
            race: character.race,
        });
        Ok(CreateOutcome::Created(character.clone()))
    }

    async fn load_for_admission(
        &self,
        character_id: CharacterId,
    ) -> anyhow::Result<Option<ProgressionState>> {
        let state = self.state.lock();
        if let Some(c) = state.characters.get(&character_id) {
            let progression = state
                .progression
                .get(&character_id)
                .ok_or_else(|| anyhow::anyhow!("progression missing"))?;
            self.validate_versioned(
                c,
                &c.class_state,
                progression.phase3.as_ref(),
                progression.alive,
            )?;
        }
        Ok(state.progression.get(&character_id).cloned())
    }

    async fn checkpoint(
        &self,
        checkpoint: &CharacterCheckpoint,
        events: &[DomainEvent],
    ) -> Result<CheckpointOutcome, CheckpointError> {
        let mut s = self.state.lock();
        self.checkpoint_locked(&mut s, checkpoint, events, checkpoint.fingerprint(events))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::{CharacterName, Position, Race};

    const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
    const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

    fn setup() -> (InMemoryCharacterRepository, Character) {
        let repo = InMemoryCharacterRepository::default();
        let c = Character::create(
            AccountId::from_uuid(uuid::Uuid::nil()),
            CharacterName::new("Durin").unwrap(),
            Race::Dwarf,
        );
        repo.insert_for_test(c.clone());
        (repo, c)
    }

    fn cp(c: &Character, seen: u64, key: &str) -> CharacterCheckpoint {
        CharacterCheckpoint {
            phase3: None,
            character_id: c.id,
            revision_seen: seen,
            level: 2,
            xp: 100,
            hp: 50,
            mp: 20,
            alive: true,
            position: Position { x: 1.5, y: 2.5 },
            idempotency: ("save_checkpoint".to_owned(), IdempotencyKey::parse(key).unwrap()),
            class_state: None,
        }
    }

    #[tokio::test]
    async fn checkpoint_applies_replays_and_fences() {
        let (repo, c) = setup();
        let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((p.xp, p.hp, p.revision), (0, None, 0));
        assert_eq!(p.class_profile, "dwarven_fighter");

        let ev = [DomainEvent::CharacterLeveled {
            metadata: crate::domain::EventMetadata::default(),
            character_id: c.id,
            level: 2,
        }];
        let a = cp(&c, 0, KEY_A);
        assert_eq!(repo.checkpoint(&a, &ev).await.unwrap(), CheckpointOutcome::Applied(1));
        assert_eq!(repo.checkpoint(&a, &ev).await.unwrap(), CheckpointOutcome::Replayed(1));
        assert_eq!(repo.staged_events(), ev);
        let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
        assert_eq!((p.level, p.xp, p.hp, p.mp, p.revision), (2, 100, Some(50), Some(20), 1));

        let mut changed = a.clone();
        changed.xp = 101;
        assert!(matches!(repo.checkpoint(&changed, &ev).await, Err(CheckpointError::KeyReused)));
        assert_eq!(
            repo.checkpoint(&cp(&c, 0, KEY_B), &ev).await.unwrap(),
            CheckpointOutcome::Stale
        );
        assert_eq!(repo.staged_events().len(), 1, "stale wrote nothing");

        let mut bad = cp(&c, 1, KEY_B);
        bad.level = 86;
        assert!(matches!(
            repo.checkpoint(&bad, &[]).await,
            Err(CheckpointError::Constraint(n)) if n == "characters_level_range"
        ));
    }
}
