//! Create a character. Idempotent on the client key; atomic in the repository; publishes
//! `CharacterCreated` in the outbox (inside the same transaction); the outbox relay is the only
//! publisher of domain events.

use crate::domain::character_progression::{auto_get_metadata, CharacterAppearance, ClassState};
use crate::domain::class::{ClassId, ClassRegistry};
use std::collections::BTreeSet;
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
    /// Absent preserves the race's fighter default; explicit zero is valid.
    pub base_class_id: Option<ClassId>,
    /// Valid sex and asset indices parsed by the interface.
    pub appearance: CharacterAppearance,
}

/// The create-character use case.
pub struct CreateCharacter {
    characters: Arc<dyn CharacterRepository>,
    classes: Arc<ClassRegistry>,
    blocked_names: Arc<BTreeSet<String>>,
}

impl CreateCharacter {
    /// Builds the use case.
    #[must_use]
    pub fn new(characters: Arc<dyn CharacterRepository>, classes: Arc<ClassRegistry>) -> Self {
        Self {
            characters,
            classes,
            blocked_names: Arc::default(),
        }
    }

    /// Uses a curated, operator-provided case-insensitive exact-name blocklist.
    #[must_use]
    pub fn with_blocked_names(mut self, blocked_names: Arc<BTreeSet<String>>) -> Self {
        self.blocked_names = blocked_names;
        self
    }

    /// Executes. Returns the character, whether created now or replayed from a previous call
    /// with the same key.
    pub async fn execute(&self, input: CreateCharacterInput) -> Result<Character, AppError> {
        let name = CharacterName::new(input.name)?;
        if self.blocked_names.contains(&name.normalized()) {
            return Err(AppError::InvalidArgument("name is unavailable".into()));
        }
        let class_id = input
            .base_class_id
            .unwrap_or(input.race.starting_class_id());
        let class = self
            .classes
            .get(class_id)
            .ok_or_else(|| AppError::InvalidArgument("unknown base_class_id".into()))?;
        if class.tier != 0 || class.race != input.race {
            return Err(AppError::InvalidArgument(
                "base_class_id must be a base class of the selected race".into(),
            ));
        }
        let race = self
            .classes
            .race(input.race)
            .ok_or_else(|| AppError::InvalidArgument("race is not selectable".into()))?;
        if !race.base_class_ids.contains(&class_id) {
            return Err(AppError::InvalidArgument(
                "race does not support this starting path".into(),
            ));
        }
        if input.appearance.hair_style != 0
            || input.appearance.hair_color != 0
            || input.appearance.face != 0
        {
            return Err(AppError::InvalidArgument("appearance indices must be zero".into()));
        }
        let mut character = Character::create(input.account_id, name, input.race);
        character.stats = class.base_stats;
        character.appearance = input.appearance;
        character.class_state = ClassState::new(class_id);
        character.class_state.merge_learned_skills(
            auto_get_metadata(&self.classes, class_id, 1).map_err(anyhow::Error::from)?,
        );
        character
            .class_state
            .merge_learned_skills(race.passive_skill_keys.iter().map(|key| {
                crate::domain::subclass::LearnedSkill {
                    key: key.clone(),
                    level: 1,
                }
            }));
        // The current production fixture starts at origin for every race (Phase 0b contract).
        // Reference village points are metadata for future world content.
        character.position = crate::domain::Position::default();
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
                RepositoryError::SlotsFull => {
                    AppError::ResourceExhausted("all seven character slots are occupied".into())
                },
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
        let legacy = format!("v1|{}|{}|{}", c.account_id, c.name.normalized(), c.race.as_str());
        if c.class_state.base_class_id == c.race.starting_class_id()
            && c.appearance == CharacterAppearance::default()
        {
            legacy
        } else {
            format!(
                "v2|{legacy}|{}|{:?}|{}|{}|{}",
                c.class_state.base_class_id.0,
                c.appearance.sex,
                c.appearance.hair_style,
                c.appearance.hair_color,
                c.appearance.face
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::DomainEvent;
    use crate::infrastructure::memory::InMemoryCharacterRepository;

    fn registry() -> Arc<ClassRegistry> {
        crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap()
        .registry
    }

    fn sut() -> (CreateCharacter, Arc<InMemoryCharacterRepository>) {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        (CreateCharacter::new(repo.clone(), registry()), repo)
    }

    fn input(key: &str, name: &str) -> CreateCharacterInput {
        CreateCharacterInput {
            idempotency_key: IdempotencyKey::parse(key).unwrap(),
            account_id: AccountId::from_uuid(Uuid::nil()),
            name: name.to_owned(),
            race: Race::Elf,
            base_class_id: None,
            appearance: CharacterAppearance::default(),
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
        let uc = CreateCharacter::new(Arc::new(Down), registry());
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
    #[tokio::test]
    async fn all_nine_base_paths_use_registry_stats_and_persist_appearance_and_metadata() {
        for class in registry().classes().iter().filter(|c| c.tier == 0) {
            let (uc, repo) = sut();
            let mut request = input(KEY_A, "Hero");
            request.race = class.race;
            request.base_class_id = Some(class.id);
            request.appearance.sex = crate::domain::subclass::Sex::Female;
            let created = uc.execute(request).await.unwrap();
            assert_eq!(created.stats, class.base_stats);
            assert_eq!(created.class_state.base_class_id, class.id);
            assert_eq!(created.class_state.current_class_id, class.id);
            assert_eq!(created.position, crate::domain::Position::default());
            assert_eq!(created.appearance.sex, crate::domain::subclass::Sex::Female);
            let saved = repo.load_for_admission(created.id).await.unwrap().unwrap();
            assert_eq!(saved.class_state, created.class_state);
            assert_eq!(saved.identity, created.identity());
            for key in &registry().race(class.race).unwrap().passive_skill_keys {
                assert!(saved
                    .class_state
                    .learned_skills
                    .iter()
                    .any(|skill| &skill.key == key && skill.level == 1));
            }
        }
    }

    #[tokio::test]
    async fn invalid_class_path_appearance_and_blocked_name_write_nothing() {
        let (uc, repo) = sut();
        for (race, id) in [
            (Race::Elf, 0),
            (Race::Human, 1),
            (Race::Dwarf, 10),
            (Race::Human, 123),
            (Race::Human, 999),
        ] {
            let mut request = input(KEY_A, "Hero");
            request.race = race;
            request.base_class_id = Some(ClassId(id));
            assert!(matches!(uc.execute(request).await, Err(AppError::InvalidArgument(_))));
        }
        for field in 0..3 {
            let mut request = input(KEY_A, "Hero");
            match field {
                0 => request.appearance.hair_style = 1,
                1 => request.appearance.hair_color = 1,
                _ => request.appearance.face = 1,
            }
            assert!(matches!(uc.execute(request).await, Err(AppError::InvalidArgument(_))));
        }
        let uc = uc.with_blocked_names(Arc::new(BTreeSet::from(["hero".into()])));
        assert!(matches!(
            uc.execute(input(KEY_A, "Hero")).await,
            Err(AppError::InvalidArgument(_))
        ));
        assert!(repo.is_empty());
        assert!(repo.staged_events().is_empty());
    }

    #[tokio::test]
    async fn seven_slots_are_atomic_and_a_known_key_still_replays() {
        let (uc, repo) = sut();
        let first = uc.execute(input(KEY_A, "Hero")).await.unwrap();
        for suffix in ['a', 'b', 'c', 'd', 'e', 'f'] {
            let mut request = input(KEY_B, &format!("Hero{suffix}"));
            request.idempotency_key = IdempotencyKey::new();
            uc.execute(request).await.unwrap();
        }
        let mut eighth = input(KEY_B, "Eighth");
        eighth.idempotency_key = IdempotencyKey::new();
        assert!(matches!(uc.execute(eighth).await, Err(AppError::ResourceExhausted(_))));
        assert_eq!(uc.execute(input(KEY_A, "Hero")).await.unwrap(), first);
        assert_eq!(repo.len(), 7);
        assert_eq!(repo.staged_events().len(), 7);
    }
}
