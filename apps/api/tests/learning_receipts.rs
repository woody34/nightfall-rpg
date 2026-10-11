//! Memory/Postgres parity for account/operation-scoped learning receipts and atomic fences.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api::application::ports::{
    learn_skill_fingerprint, FrozenLearnResult, LearnReceiptLookup, LearnReceiptRepository,
    LearnedEffectiveStats, LearningCheckpointOutcome, LearningCheckpointRepository,
    SuccessfulLearnReceipt,
};
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::subclass::LearnedSkill;
use nightfall_api::domain::zone::skill::{SkillId, SkillRef};
use nightfall_api::domain::zone::Tick;
use nightfall_api::domain::{
    AccountId, Character, CharacterName, DomainEvent, EventMetadata, Position, Race,
};
use nightfall_api::infrastructure::memory::InMemoryCharacterRepository;
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use std::sync::Arc;
use uuid::Uuid;

async fn seed<R: CharacterRepository>(repo: &R, name: &str, key: IdempotencyKey) -> Character {
    let mut c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    c.level = 5;
    c.class_state.sp = 100;
    repo.create_idempotent(&key, name, &c).await.unwrap();
    c
}
// Transaction fixtures use legal existing auto-get ledger rows so domain policy is not
// bypassed. They test storage bookkeeping, not auto-get charging or manual gameplay guards.
fn commit(
    c: &Character,
    key: IdempotencyKey,
    rank: u32,
) -> (CharacterCheckpoint, SuccessfulLearnReceipt, Vec<DomainEvent>) {
    let skill = SkillRef {
        id: SkillId(239),
        rank,
    };
    let mut ledger = c.class_state.clone();
    ledger.sp = 50;
    ledger.merge_learned_skills([LearnedSkill {
        key: "l2.skill.239".into(),
        level: rank,
    }]);
    let result = FrozenLearnResult {
        character_id: c.id,
        skill,
        sp_spent: 50,
        sp_remaining: 50,
        tick: Tick(73),
        hp: 80,
        max_hp: 100,
        mp: 40,
        max_mp: 60,
        cp: 20,
        max_cp: 40,
        effective_stats: LearnedEffectiveStats {
            p_atk_q: 7_000_001,
            p_def_q: 0,
            m_atk_q: 0,
            m_def_q: 0,
            attack_speed: 0,
            cast_speed: 0,
            speed_q: 0,
            accuracy_q: 0,
            evasion_q: 0,
            crit_permille: 0,
        },
        incarnation: 4,
    };
    let receipt = SuccessfulLearnReceipt {
        account_id: c.account_id,
        key,
        request_fingerprint: learn_skill_fingerprint(c.id, skill),
        result,
    };
    let cp = CharacterCheckpoint {
        phase3: None,
        character_id: c.id,
        revision_seen: 0,
        level: c.level,
        xp: c.xp,
        hp: 80,
        mp: 40,
        alive: true,
        position: Position { x: 126.0, y: 128.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
    };
    let events = vec![DomainEvent::CharacterSkillLearned {
        metadata: EventMetadata::default(),
        character_id: c.id,
        skill_id: skill.id.0,
        skill_level: skill.rank,
        sp_spent: receipt.result.sp_spent,
        sp_remaining: receipt.result.sp_remaining,
        tick: receipt.result.tick.0,
        request_key: key.as_uuid(),
    }];
    (cp, receipt, events)
}
async fn lookup<R: LearnReceiptRepository>(
    repo: &R,
    receipt: &SuccessfulLearnReceipt,
) -> LearnReceiptLookup {
    repo.learn_receipt_lookup(receipt.account_id, &receipt.key, &receipt.request_fingerprint)
        .await
        .unwrap()
}

async fn round_trip<R: CharacterRepository + LearningCheckpointRepository>(repo: &R) {
    let key = IdempotencyKey::new();
    let c = seed(repo, "Learner", key).await;
    let (cp, receipt, events) = commit(&c, key, 1); // Same UUID as create; operation scopes differ.
    assert_eq!(lookup(repo, &receipt).await, LearnReceiptLookup::Unknown);
    assert_eq!(
        repo.checkpoint_learning(&cp, &events, &receipt)
            .await
            .unwrap(),
        LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Applied(1))
    );
    let read = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(read.class_state.sp, 50);
    assert!(read
        .class_state
        .learned_skills
        .iter()
        .any(|s| s.key == "l2.skill.239" && s.level == 1));
    assert!(read.class_state.successful_transfer_receipts.is_empty());
    assert_eq!(lookup(repo, &receipt).await, LearnReceiptLookup::Known(receipt.result.clone()));
    assert_eq!(
        repo.checkpoint_learning(&cp, &events, &receipt)
            .await
            .unwrap(),
        LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Replayed(1))
    );
    let mut later = cp.clone();
    later.revision_seen = 1;
    later.level = 20;
    later.xp = 999;
    later.idempotency.1 = IdempotencyKey::new();
    later.class_state.as_mut().unwrap().sp = 7;
    later
        .class_state
        .as_mut()
        .unwrap()
        .merge_learned_skills([LearnedSkill {
            key: "l2.skill.239".into(),
            level: 3,
        }]);
    assert_eq!(repo.checkpoint(&later, &[]).await.unwrap(), CheckpointOutcome::Applied(2));
    assert_eq!(lookup(repo, &receipt).await, LearnReceiptLookup::Known(receipt.result.clone()));
    let mut changed = receipt.clone();
    changed.result.skill.rank = 2;
    changed.request_fingerprint = learn_skill_fingerprint(c.id, changed.result.skill);
    assert_eq!(lookup(repo, &changed).await, LearnReceiptLookup::Conflict);
    assert!(matches!(
        repo.checkpoint_learning(&cp, &[], &changed).await,
        Err(CheckpointError::KeyReused)
    ));
    let other_account = AccountId::from_uuid(Uuid::from_u128(9));
    assert_eq!(
        repo.learn_receipt_lookup(other_account, &key, &receipt.request_fingerprint)
            .await
            .unwrap(),
        LearnReceiptLookup::Unknown
    );
    let other = seed(repo, "Other", IdempotencyKey::new()).await;
    let (other_cp, mut other_receipt, _) = commit(&other, key, 1);
    assert_eq!(lookup(repo, &other_receipt).await, LearnReceiptLookup::Conflict);
    assert!(matches!(
        repo.checkpoint_learning(&other_cp, &[], &other_receipt)
            .await,
        Err(CheckpointError::KeyReused)
    ));
    other_receipt.account_id = other_account;
    assert!(matches!(
        repo.checkpoint_learning(&other_cp, &[], &other_receipt)
            .await,
        Err(CheckpointError::Constraint(_))
    ));
    assert_eq!(
        repo.load_for_admission(other.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}

async fn rollback<R: CharacterRepository + LearningCheckpointRepository>(repo: &R) {
    let c = seed(repo, "Rollback", IdempotencyKey::new()).await;
    let (cp, receipt, events) = commit(&c, IdempotencyKey::new(), 1);
    let before = repo.load_for_admission(c.id).await.unwrap();
    let mut stale = cp.clone();
    stale.revision_seen = 7;
    assert_eq!(
        repo.checkpoint_learning(&stale, &events, &receipt)
            .await
            .unwrap(),
        LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Stale)
    );
    assert_eq!(lookup(repo, &receipt).await, LearnReceiptLookup::Unknown);
    let mut invalid = cp.clone();
    invalid.level = 86;
    assert!(matches!(
        repo.checkpoint_learning(&invalid, &events, &receipt).await,
        Err(CheckpointError::Constraint(_))
    ));
    let mut invalid = cp.clone();
    invalid
        .class_state
        .as_mut()
        .unwrap()
        .learned_skills
        .push(LearnedSkill {
            key: "l2.skill.999999".into(),
            level: 1,
        });
    assert!(matches!(
        repo.checkpoint_learning(&invalid, &events, &receipt).await,
        Err(CheckpointError::Constraint(_))
    ));
    assert_eq!(lookup(repo, &receipt).await, LearnReceiptLookup::Unknown);
    assert_eq!(repo.load_for_admission(c.id).await.unwrap(), before);
    assert_eq!(
        repo.checkpoint_learning(&cp, &events, &receipt)
            .await
            .unwrap(),
        LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Applied(1))
    );
    // A new learning claim with a reused checkpoint key must roll back on fingerprint conflict.
    let mut new_receipt = receipt.clone();
    new_receipt.key = IdempotencyKey::new();
    assert!(matches!(
        repo.checkpoint_learning(&cp, &events, &new_receipt).await,
        Err(CheckpointError::KeyReused)
    ));
    assert_eq!(lookup(repo, &new_receipt).await, LearnReceiptLookup::Unknown);
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}

async fn concurrency<R: CharacterRepository + LearningCheckpointRepository>(
    repo: Arc<R>,
    different_body: bool,
    same_key: bool,
) {
    let c = seed(repo.as_ref(), "Concurrent", IdempotencyKey::new()).await;
    let (cp, receipt, events) = commit(&c, IdempotencyKey::new(), 1);
    let (mut other_cp, mut other_receipt, _) =
        commit(&c, receipt.key, if different_body { 2 } else { 1 });
    if !same_key {
        other_receipt.key = IdempotencyKey::new();
    }
    if !different_body && same_key {
        other_cp = cp.clone();
    }
    let (a, b) = tokio::join!(
        repo.checkpoint_learning(&cp, &events, &receipt),
        repo.checkpoint_learning(&other_cp, &events, &other_receipt)
    );
    let applied = [a.as_ref(), b.as_ref()]
        .iter()
        .filter(|result| {
            matches!(
                result,
                Ok(LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Applied(1)))
            )
        })
        .count();
    assert_eq!(applied, 1);
    if same_key && !different_body {
        assert!([a.as_ref(), b.as_ref()].iter().any(|r| matches!(
            r,
            Ok(LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Replayed(1)))
        )));
    } else if same_key {
        assert!([a.as_ref(), b.as_ref()]
            .iter()
            .any(|r| matches!(r, Err(CheckpointError::KeyReused))));
    } else {
        assert!([a.as_ref(), b.as_ref()].iter().any(|r| matches!(
            r,
            Ok(LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Stale))
        )));
        let receipts = [
            lookup(repo.as_ref(), &receipt).await,
            lookup(repo.as_ref(), &other_receipt).await,
        ];
        assert_eq!(
            receipts
                .iter()
                .filter(|r| matches!(r, LearnReceiptLookup::Known(_)))
                .count(),
            1
        );
    }
    let read = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!((read.revision, read.class_state.sp), (1, 50));
}

#[tokio::test]
async fn memory_round_trip_replay_scopes_and_atomic_rollback() {
    let repo = InMemoryCharacterRepository::default();
    round_trip(&repo).await;
    assert_eq!(repo.staged_events().len(), 3); // two creates + one committed learning outbox event
    let repo = InMemoryCharacterRepository::default();
    rollback(&repo).await;
    assert_eq!(repo.staged_events().len(), 2);
}
#[tokio::test]
async fn memory_concurrent_same_key_different_body_and_revision_fences() {
    for (different, same) in [(false, true), (true, true), (false, false)] {
        concurrency(Arc::new(InMemoryCharacterRepository::default()), different, same).await;
    }
}
#[tokio::test]
async fn postgres_round_trip_replay_scopes_and_atomic_rollback() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    round_trip(&PgCharacterRepository::new(pool.clone())).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    rollback(&PgCharacterRepository::new(pool.clone())).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}
#[tokio::test]
async fn postgres_concurrent_same_key_different_body_and_revision_fences() {
    for (different, same) in [(false, true), (true, true), (false, false)] {
        let Some(pool) = common::pg::migrated_pool().await else {
            return;
        };
        concurrency(Arc::new(PgCharacterRepository::new(pool.clone())), different, same).await;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }
}

#[tokio::test]
async fn postgres_outbox_failure_rolls_back_ledger_receipt_keys_and_revision() {
    use sqlx::Executor as _;
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = seed(&repo, "OutboxFail", IdempotencyKey::new()).await;
    let (cp, receipt, events) = commit(&c, IdempotencyKey::new(), 1);
    let before = repo.load_for_admission(c.id).await.unwrap();
    // Derive the exact event subject from the typed fixture, not an invented production name.
    let sql = sqlx::AssertSqlSafe(format!(
        "ALTER TABLE outbox ADD CONSTRAINT fail_learn_event CHECK (subject <> '{}')",
        events[0].subject()
    ));
    pool.execute(sqlx::raw_sql(sql)).await.unwrap();
    assert!(matches!(
        repo.checkpoint_learning(&cp, &events, &receipt).await,
        Err(CheckpointError::Constraint(_))
    ));
    assert_eq!(lookup(&repo, &receipt).await, LearnReceiptLookup::Unknown);
    assert_eq!(repo.load_for_admission(c.id).await.unwrap(), before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys WHERE operation IN ('save_checkpoint','learn_skill')").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
    pool.execute("ALTER TABLE outbox DROP CONSTRAINT fail_learn_event")
        .await
        .unwrap();
    assert!(repo
        .checkpoint_learning(&cp, &events, &receipt)
        .await
        .is_ok());
}

#[test]
fn receipt_binding_and_frozen_checkpoint_fingerprints_are_typed_and_sensitive() {
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Fingerprint").unwrap(),
        Race::Human,
    );
    let (cp, receipt, events) = commit(&c, IdempotencyKey::new(), 1);
    let legacy = cp.fingerprint(&events);
    let fingerprint = receipt.checkpoint_fingerprint(&cp, &events).unwrap();
    assert_ne!(legacy, fingerprint);
    for field in 0..5 {
        let mut changed = receipt.clone();
        match field {
            0 => changed.key = IdempotencyKey::new(),
            1 => changed.result.tick = Tick(74),
            2 => changed.result.sp_remaining = 49,
            3 => changed.result.effective_stats.p_atk_q = 7_000_002,
            _ => changed.result.incarnation = 5,
        }
        assert_ne!(changed.checkpoint_fingerprint(&cp, &events).unwrap(), fingerprint);
    }
    assert_eq!(cp.fingerprint(&events), legacy);
    let mut invalid = receipt.clone();
    invalid.result.skill.rank = 2;
    assert!(invalid.validate(&cp, c.account_id).is_err());
    let mut invalid = receipt.clone();
    invalid.result.sp_remaining = u64::MAX;
    assert!(invalid.validate(&cp, c.account_id).is_err());
    let mut value = serde_json::to_value(receipt).unwrap();
    value["result"]["arbitrary_payload"] = serde_json::json!({});
    assert!(serde_json::from_value::<SuccessfulLearnReceipt>(value).is_err());
}

#[tokio::test]
async fn paid_skill_checkpoint_round_trip_with_phase3_ledger_validator() {
    paid_round_trip(&InMemoryCharacterRepository::default()).await;
}

#[tokio::test]
async fn postgres_paid_skill_checkpoint_round_trip_with_phase3_ledger_validator() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    paid_round_trip(&PgCharacterRepository::new(pool)).await;
}

async fn paid_round_trip<R: CharacterRepository + LearningCheckpointRepository>(repo: &R) {
    let c = seed(repo, "PaidLearn", IdempotencyKey::new()).await;
    let (mut cp, mut receipt, events) = commit(&c, IdempotencyKey::new(), 1);
    cp.class_state
        .as_mut()
        .unwrap()
        .learned_skills
        .retain(|skill| skill.key != "l2.skill.239");
    cp.class_state
        .as_mut()
        .unwrap()
        .merge_learned_skills([LearnedSkill {
            key: "l2.skill.3".into(),
            level: 1,
        }]);
    cp.phase3 = Some(nightfall_api::domain::zone::Phase3Checkpoint::new());
    receipt.result.skill.id = SkillId(3);
    receipt.request_fingerprint = learn_skill_fingerprint(c.id, receipt.result.skill);
    assert_eq!(
        repo.checkpoint_learning(&cp, &events, &receipt)
            .await
            .unwrap(),
        LearningCheckpointOutcome::Checkpoint(CheckpointOutcome::Applied(1))
    );
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .class_state
            .sp,
        50
    );
}
