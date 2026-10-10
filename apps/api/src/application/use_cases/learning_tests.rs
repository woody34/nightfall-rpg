//! Use-case contract tests with no I/O; the fake runtime serializes receipt settlement.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use crate as api;
#[path = "../../../tests/common/phase3.rs"]
mod support;
use super::{GetSkillCatalog, LearnSkill, LearnSkillInput, ListLearnableSkills};
use crate::application::{AppError, IdempotencyKey};
use crate::domain::{AccountId, Character, CharacterName, Race};
use crate::infrastructure::memory::InMemoryCharacterRepository;
use std::sync::Arc;
use support::{Catalogue, Runtime};
use uuid::Uuid;

struct Fixture {
    repo: Arc<InMemoryCharacterRepository>,
    character: Character,
    runtime: Arc<Runtime>,
    catalogue: Arc<Catalogue>,
}
impl Fixture {
    fn new() -> Self {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let character = Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Learner").unwrap(),
            Race::Human,
        );
        repo.insert_for_test(character.clone());
        let runtime = Arc::new(Runtime::new(character.account_id, character.id));
        Self {
            repo,
            character,
            runtime,
            catalogue: Arc::new(Catalogue { failure: None }),
        }
    }
    fn learn(&self) -> LearnSkill {
        LearnSkill::new(
            self.repo.clone(),
            self.runtime.clone(),
            self.catalogue.clone(),
            Some(self.runtime.clone()),
        )
    }
    fn input(&self) -> LearnSkillInput {
        LearnSkillInput {
            caller: self.character.account_id,
            character_id: self.character.id.to_string(),
            skill_id: 3,
            skill_level: 1,
            idempotency_key: IdempotencyKey::new().to_string(),
        }
    }
    fn untouched(&self) {
        let state = self.runtime.0.lock();
        assert_eq!((state.sp, state.rank, state.writes, state.events), (100, 0, 0, 0));
        assert!(state.receipts.is_empty());
        assert_eq!(self.repo.get_for_test(self.character.id), Some(self.character.clone()));
        assert!(self.repo.staged_events().is_empty());
    }
}

#[tokio::test]
async fn malformed_requests_touch_no_ports() {
    let f = Fixture::new();
    for case in 0..6 {
        let mut input = f.input();
        match case {
            0 => input.character_id = "bad".into(),
            1 => input.idempotency_key.clear(),
            2 => input.idempotency_key = "bad".into(),
            3 => input.skill_id = 0,
            4 => input.skill_level = 0,
            _ => input.character_id.clear(),
        }
        assert!(matches!(f.learn().execute(input).await, Err(AppError::InvalidArgument(_))));
    }
    assert_eq!(f.runtime.0.lock().lookups, 0);
    f.untouched();
}

#[tokio::test]
async fn identity_and_skill_errors_precede_live_routing() {
    let f = Fixture::new();
    let mut input = f.input();
    input.character_id = Uuid::from_u128(99).to_string();
    assert!(matches!(f.learn().execute(input).await, Err(AppError::NotFound { .. })));
    let mut input = f.input();
    input.caller = AccountId::from_uuid(Uuid::from_u128(99));
    assert!(matches!(f.learn().execute(input).await, Err(AppError::PermissionDenied(_))));
    for (id, rank) in [(999, 1), (3, 10)] {
        let mut input = f.input();
        input.skill_id = id;
        input.skill_level = rank;
        assert!(matches!(f.learn().execute(input).await, Err(AppError::InvalidArgument(_))));
    }
    let mut input = f.input();
    input.skill_id = 4;
    let error = f.learn().execute(input).await.unwrap_err();
    assert!(
        matches!(&error, AppError::FailedPrecondition(reason) if reason.contains("item_consume"))
    );
    assert_eq!(f.runtime.0.lock().generations, 0);
    f.untouched();
}

#[tokio::test]
async fn concurrent_duplicates_settle_once_and_replay_offline_after_progression() {
    let f = Fixture::new();
    let input = f.input();
    let uc = f.learn();
    let (a, b) = tokio::join!(uc.execute(input.clone()), uc.execute(input.clone()));
    let original = a.unwrap();
    assert_eq!(b.unwrap(), original);
    assert_eq!(
        (original.skill.rank, original.sp_spent, original.sp_remaining, original.tick.0),
        (1, 10, 90, 73)
    );
    {
        let mut state = f.runtime.0.lock();
        assert_eq!((state.writes, state.events, state.receipts.len()), (1, 1, 1));
        state.sp = 777;
        state.rank = 8;
        state.generation = None;
        state.receipt_failure = false;
    }
    let mut later = f.character.clone();
    later.level = 60;
    f.repo.insert_for_test(later);
    let offline = LearnSkill::new(
        f.repo.clone(),
        f.runtime.clone(),
        Arc::new(Catalogue {
            failure: Some("unavailable"),
        }),
        None,
    );
    let before = f.runtime.0.lock().generations;
    assert_eq!(offline.execute(input.clone()).await.unwrap(), original);
    assert_eq!(f.runtime.0.lock().generations, before);
    for case in 0..3 {
        let mut different = input.clone();
        match case {
            0 => different.skill_id = 4,
            1 => different.skill_level = 2,
            _ => different.character_id = Uuid::from_u128(7).to_string(),
        }
        assert!(matches!(offline.execute(different).await, Err(AppError::IdempotencyConflict)));
    }
    assert_eq!(f.runtime.0.lock().writes, 1);
}

#[tokio::test]
async fn concurrent_different_bodies_conflict_and_different_keys_cannot_buy_same_rank_twice() {
    for same_key in [true, false] {
        let f = Fixture::new();
        let a = f.input();
        let mut b = f.input();
        if same_key {
            b.idempotency_key.clone_from(&a.idempotency_key);
            b.skill_id = 5;
        }
        let uc = f.learn();
        let (a, b) = tokio::join!(uc.execute(a), uc.execute(b));
        let failure = match (a, b) {
            (Ok(_), Err(error)) | (Err(error), Ok(_)) => Some(error),
            _ => None,
        };
        assert!(matches!(
            (same_key, failure),
            (true, Some(AppError::IdempotencyConflict))
                | (false, Some(AppError::FailedPrecondition(_)))
        ));
        let state = f.runtime.0.lock();
        assert_eq!((state.writes, state.sp), (1, 90));
    }
}

#[tokio::test]
async fn account_scope_and_failed_request_reuses_key_without_consumption() {
    let f = Fixture::new();
    let input = f.input();
    f.runtime.0.lock().failure = Some("trainer required");
    assert!(matches!(
        f.learn().execute(input.clone()).await,
        Err(AppError::FailedPrecondition(_))
    ));
    f.untouched();
    f.runtime.0.lock().failure = None;
    f.learn().execute(input.clone()).await.unwrap();
    let mut other = input;
    other.caller = AccountId::from_uuid(Uuid::from_u128(3));
    assert!(matches!(f.learn().execute(other).await, Err(AppError::PermissionDenied(_))));
    assert_eq!(f.runtime.0.lock().writes, 1);
}

#[tokio::test]
async fn runtime_failures_and_generation_races_are_fail_closed() {
    for reason in [
        "dead",
        "engaged",
        "casting",
        "trainer required",
        "lineage",
        "rank",
        "required level",
        "SP",
        "learning item prerequisite",
        "unavailable",
        "infrastructure",
        "resource_exhausted",
    ] {
        let f = Fixture::new();
        f.runtime.0.lock().failure = Some(reason);
        let error = f.learn().execute(f.input()).await.unwrap_err();
        assert!(matches!(
            (reason, error),
            ("unavailable", AppError::Unavailable(_))
                | ("infrastructure", AppError::Infrastructure(_))
                | ("resource_exhausted", AppError::ResourceExhausted(_))
                | (_, AppError::FailedPrecondition(_))
        ));
        f.untouched();
    }
    let f = Fixture::new();
    f.runtime.0.lock().stale_after_capture = true;
    assert!(matches!(
        f.learn().execute(f.input()).await,
        Err(AppError::FailedPrecondition(_))
    ));
    f.untouched();
    let f = Fixture::new();
    f.runtime.0.lock().drop_reply = true;
    assert!(matches!(f.learn().execute(f.input()).await, Err(AppError::Unavailable(_))));
    f.untouched();
    let f = Fixture::new();
    f.runtime.0.lock().generation = None;
    assert!(matches!(
        f.learn().execute(f.input()).await,
        Err(AppError::FailedPrecondition(_))
    ));
    f.untouched();
    let f = Fixture::new();
    let uc = LearnSkill::new(f.repo.clone(), f.runtime.clone(), f.catalogue.clone(), None);
    assert!(matches!(uc.execute(f.input()).await, Err(AppError::FailedPrecondition(_))));
    f.untouched();
}

#[tokio::test]
async fn receipt_and_catalogue_failures_consume_nothing() {
    let f = Fixture::new();
    f.runtime.0.lock().receipt_failure = true;
    assert!(matches!(f.learn().execute(f.input()).await, Err(AppError::Infrastructure(_))));
    assert_eq!(f.runtime.0.lock().generations, 0);
    f.untouched();
    for reason in ["unavailable", "infrastructure"] {
        let f = Fixture::new();
        let catalogue = Arc::new(Catalogue {
            failure: Some(reason),
        });
        let uc = LearnSkill::new(
            f.repo.clone(),
            f.runtime.clone(),
            catalogue.clone(),
            Some(f.runtime.clone()),
        );
        assert!(matches!(
            (reason, uc.execute(f.input()).await),
            ("unavailable", Err(AppError::Unavailable(_)))
                | ("infrastructure", Err(AppError::Infrastructure(_)))
        ));
        assert!(GetSkillCatalog::new(Some(catalogue)).execute().is_err());
        f.untouched();
    }
}

#[tokio::test]
async fn live_query_checks_owner_generation_and_preserves_all_eligibility_fields() {
    let f = Fixture::new();
    let uc = ListLearnableSkills::new(f.repo.clone(), Some(f.runtime.clone()));
    let out = uc
        .execute(f.character.account_id, &f.character.id.to_string())
        .await
        .unwrap();
    assert_eq!((out.sp, out.current_class_id), (100, crate::domain::class::ClassId(0)));
    let option = out.skills.first().unwrap();
    assert!(
        option.effect_implemented
            && option.affordable
            && option.trainer_required
            && option.eligible
            && !option.learned
    );
    assert_eq!(
        (option.skill.skill_id, option.skill.skill_level, option.skill.sp_cost),
        (3, 1, 10)
    );
    assert!(option.unmet.is_empty());
    assert!(matches!(
        uc.execute(f.character.account_id, "bad").await,
        Err(AppError::InvalidArgument(_))
    ));
    assert!(matches!(
        uc.execute(f.character.account_id, &Uuid::nil().to_string())
            .await,
        Err(AppError::NotFound { .. })
    ));
    assert!(matches!(
        uc.execute(AccountId::from_uuid(Uuid::from_u128(9)), &f.character.id.to_string())
            .await,
        Err(AppError::PermissionDenied(_))
    ));
    f.runtime.0.lock().stale_after_capture = true;
    assert!(matches!(
        uc.execute(f.character.account_id, &f.character.id.to_string())
            .await,
        Err(AppError::FailedPrecondition(_))
    ));
    f.untouched();
}
