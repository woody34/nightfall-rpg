//! Authenticated class transfer: ownership, forever-retained receipt first, then live routing.

use std::sync::Arc;

use crate::application::ports::{
    transfer_fingerprint, ClassTransferRuntime, MutationReceiptLookup,
};
use crate::application::{parse_id, AppError, CharacterRepository, IdempotencyKey};
use crate::domain::character_progression::FrozenTransferResult;
use crate::domain::class::{ClassId, ClassRegistry};
use crate::domain::AccountId;

/// One transfer use case. The actor and its checkpoint lane are the only mutable writers.
pub struct ChangeClass {
    characters: Arc<dyn CharacterRepository>,
    classes: Arc<ClassRegistry>,
    runtime: Option<Arc<dyn ClassTransferRuntime>>,
}

impl ChangeClass {
    /// Binds persistence reads and the live runtime; no runtime means an offline-only server.
    #[must_use]
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        classes: Arc<ClassRegistry>,
        runtime: Option<Arc<dyn ClassTransferRuntime>>,
    ) -> Self {
        Self {
            characters,
            classes,
            runtime,
        }
    }

    /// Validates ownership, replays known successes even offline, or routes to the live actor.
    pub async fn execute(
        &self,
        caller: AccountId,
        character_id: &str,
        target: u32,
        key: IdempotencyKey,
    ) -> Result<FrozenTransferResult, AppError> {
        let id = parse_id("character_id", character_id)?;
        let character = self
            .characters
            .get(id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: character_id.to_owned(),
            })?;
        if character.account_id != caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        let target = ClassId(target);
        let fingerprint = transfer_fingerprint(id, target);
        match self
            .characters
            .mutation_receipt_lookup(caller, &key, &fingerprint)
            .await?
        {
            MutationReceiptLookup::Known(result) => return Ok(result),
            MutationReceiptLookup::Conflict => return Err(AppError::IdempotencyConflict),
            MutationReceiptLookup::Unknown => {},
        }
        if self.classes.get(target).is_none() {
            return Err(AppError::InvalidArgument("unknown target_class_id".into()));
        }
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            AppError::FailedPrecondition("character has no live zone session".into())
        })?;
        runtime.change_class(caller, id, target, key).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::application::ports::{TransferEligibility, TransferOptionsState};
    use crate::domain::character_progression::SuccessfulTransferReceipt;
    use crate::domain::{Character, CharacterName, Race};
    use crate::infrastructure::memory::InMemoryCharacterRepository;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    struct Runtime {
        calls: AtomicUsize,
        result: FrozenTransferResult,
        failure: u8,
    }
    #[async_trait::async_trait]
    impl ClassTransferRuntime for Runtime {
        async fn transfer_options(
            &self,
            _caller: AccountId,
            _character: crate::domain::CharacterId,
        ) -> Result<TransferOptionsState, AppError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(TransferOptionsState {
                options: vec![TransferEligibility {
                    class_id: ClassId(1),
                    eligible: true,
                    unmet: Vec::new(),
                }],
                current_class_id: ClassId(0),
                token_tier_1_count: 1,
                token_tier_2_count: 0,
            })
        }
        async fn change_class(
            &self,
            _caller: AccountId,
            _character: crate::domain::CharacterId,
            _target: ClassId,
            _key: IdempotencyKey,
        ) -> Result<FrozenTransferResult, AppError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match self.failure {
                1 => Err(AppError::FailedPrecondition("token required".into())),
                2 => Err(AppError::Unavailable("zone stopped".into())),
                3 => Err(AppError::Infrastructure(anyhow::anyhow!("checkpoint failed"))),
                _ => Ok(self.result.clone()),
            }
        }
    }
    fn owner() -> AccountId {
        AccountId::from_uuid(Uuid::nil())
    }
    fn fixture() -> (
        Arc<InMemoryCharacterRepository>,
        Character,
        FrozenTransferResult,
        Arc<ClassRegistry>,
    ) {
        let repo = Arc::new(InMemoryCharacterRepository::default());
        let c = Character::create(owner(), CharacterName::new("Hero").unwrap(), Race::Human);
        let result = FrozenTransferResult {
            character_id: c.id,
            identity: c.identity(),
            name: c.name.clone(),
            current_class_id: ClassId(1),
            level: 20,
            xp: 100,
            sp: 50,
            stats: c.stats,
            position_millitiles: [126_000, 126_000],
            hp: 100,
            mp: 40,
            cp: 20,
            max_hp: 100,
            max_mp: 40,
            max_cp: 20,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            granted_skill_keys: Vec::new(),
        };
        repo.insert_for_test(c.clone());
        let registry = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap()
        .registry;
        (repo, c, result, registry)
    }

    #[tokio::test]
    async fn validates_identity_and_target_before_live_routing() {
        let (repo, c, result, classes) = fixture();
        let runtime = Arc::new(Runtime {
            calls: AtomicUsize::new(0),
            result,
            failure: 0,
        });
        let uc = ChangeClass::new(repo.clone(), classes, Some(runtime.clone()));
        assert!(matches!(
            uc.execute(owner(), "bad", 1, IdempotencyKey::new()).await,
            Err(AppError::InvalidArgument(_))
        ));
        assert!(matches!(
            uc.execute(owner(), &Uuid::from_u128(7).to_string(), 1, IdempotencyKey::new())
                .await,
            Err(AppError::NotFound { .. })
        ));
        assert!(matches!(
            uc.execute(
                AccountId::from_uuid(Uuid::from_u128(9)),
                &c.id.to_string(),
                1,
                IdempotencyKey::new()
            )
            .await,
            Err(AppError::PermissionDenied(_))
        ));
        assert!(matches!(
            uc.execute(owner(), &c.id.to_string(), 123, IdempotencyKey::new())
                .await,
            Err(AppError::InvalidArgument(_))
        ));
        assert_eq!(runtime.calls.load(Ordering::Relaxed), 0);
        assert!(repo.staged_events().is_empty());
        assert_eq!(
            uc.execute(owner(), &c.id.to_string(), 1, IdempotencyKey::new())
                .await
                .unwrap(),
            runtime.result
        );
        assert_eq!(runtime.calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn prior_receipt_replays_frozen_response_offline_and_conflicts_account_wide() {
        let (repo, mut c, result, classes) = fixture();
        let key = IdempotencyKey::new();
        c.class_state
            .record_success(SuccessfulTransferReceipt {
                key: key.as_uuid(),
                target_class_id: ClassId(1),
                result: result.clone(),
            })
            .unwrap();
        c.class_state.current_class_id = ClassId(1);
        c.level = 40;
        c.position = crate::domain::Position { x: 5.0, y: 6.0 };
        repo.insert_for_test(c.clone());
        let uc = ChangeClass::new(repo.clone(), classes, None);
        assert_eq!(
            uc.execute(owner(), &c.id.to_string(), 1, key)
                .await
                .unwrap(),
            result
        );
        assert!(matches!(
            uc.execute(owner(), &c.id.to_string(), 4, key).await,
            Err(AppError::IdempotencyConflict)
        ));
        let second = Character::create(owner(), CharacterName::new("Other").unwrap(), Race::Human);
        repo.insert_for_test(second.clone());
        assert!(matches!(
            uc.execute(owner(), &second.id.to_string(), 1, key).await,
            Err(AppError::IdempotencyConflict)
        ));
        assert!(matches!(
            uc.execute(owner(), &c.id.to_string(), 1, IdempotencyKey::new())
                .await,
            Err(AppError::FailedPrecondition(_))
        ));
    }

    #[tokio::test]
    async fn runtime_failures_do_not_claim_a_key_or_write_events() {
        for failure in 1..=3 {
            let (repo, c, result, classes) = fixture();
            let runtime = Arc::new(Runtime {
                calls: AtomicUsize::new(0),
                result,
                failure,
            });
            let uc = ChangeClass::new(repo.clone(), classes, Some(runtime));
            let key = IdempotencyKey::new();
            let error = uc
                .execute(owner(), &c.id.to_string(), 1, key)
                .await
                .unwrap_err();
            assert!(matches!(
                (failure, error),
                (1, AppError::FailedPrecondition(_))
                    | (2, AppError::Unavailable(_))
                    | (3, AppError::Infrastructure(_))
            ));
            assert!(matches!(
                repo.mutation_receipt_lookup(
                    owner(),
                    &key,
                    &transfer_fingerprint(c.id, ClassId(1))
                )
                .await
                .unwrap(),
                MutationReceiptLookup::Unknown
            ));
            assert!(repo.staged_events().is_empty());
        }
    }

    #[tokio::test]
    async fn options_checks_ownership_and_returns_all_actor_fields() {
        let (repo, c, result, _) = fixture();
        let runtime = Arc::new(Runtime {
            calls: AtomicUsize::new(0),
            result,
            failure: 0,
        });
        let uc = crate::application::use_cases::TransferOptions::new(
            repo.clone(),
            Some(runtime.clone()),
        );
        let out = uc.execute(owner(), &c.id.to_string()).await.unwrap();
        assert_eq!(
            (out.current_class_id, out.token_tier_1_count, out.token_tier_2_count),
            (ClassId(0), 1, 0)
        );
        assert_eq!(out.options.len(), 1);
        assert!(out.options[0].eligible);
        assert!(matches!(uc.execute(owner(), "bad").await, Err(AppError::InvalidArgument(_))));
        assert!(matches!(
            uc.execute(owner(), &Uuid::now_v7().to_string()).await,
            Err(AppError::NotFound { .. })
        ));
        assert!(matches!(
            uc.execute(AccountId::from_uuid(Uuid::from_u128(9)), &c.id.to_string())
                .await,
            Err(AppError::PermissionDenied(_))
        ));
        assert_eq!(runtime.calls.load(Ordering::Relaxed), 1);
        let offline = crate::application::use_cases::TransferOptions::new(repo, None);
        assert!(matches!(
            offline.execute(owner(), &c.id.to_string()).await,
            Err(AppError::FailedPrecondition(_))
        ));
    }
}
