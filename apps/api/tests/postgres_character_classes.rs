//! Phase 2 adapter atomicity/concurrency. Each test gets a private schema in `DATABASE_URL`.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;
use common::pg::migrated_pool;
use nightfall_api::application::ports::{
    transfer_fingerprint, MutationReceiptLookup, RepositoryError,
};
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, CreateOutcome,
    IdempotencyKey,
};
use nightfall_api::domain::character_progression::{
    ClassState, FrozenTransferResult, SuccessfulTransferReceipt,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::subclass::{LearnedSkill, Sex};
use nightfall_api::domain::{
    AccountId, Character, CharacterId, CharacterName, DomainEvent, EventMetadata, Position, Race,
};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use sqlx::{Executor, PgPool};
use std::sync::Arc;
use uuid::Uuid;

fn character(name: &str) -> Character {
    Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new(name).unwrap(),
        Race::Human,
    )
}
fn frozen(c: &Character, target: u32) -> FrozenTransferResult {
    FrozenTransferResult {
        character_id: c.id,
        identity: c.identity(),
        name: c.name.clone(),
        current_class_id: ClassId(target),
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
        token_tier_2_count: 1,
        granted_skill_keys: Vec::new(),
    }
}
fn checkpoint(c: &Character, key: IdempotencyKey) -> CharacterCheckpoint {
    let mut ledger = c.class_state.clone();
    ledger.current_class_id = ClassId(1);
    ledger.sp = 50;
    ledger.cp = 20;
    ledger.token_tier_2_count = 1;
    ledger
        .record_success(SuccessfulTransferReceipt {
            key: key.as_uuid(),
            target_class_id: ClassId(1),
            result: frozen(c, 1),
        })
        .unwrap();
    ledger.merge_learned_skills([LearnedSkill {
        key: "racial.adaptable".into(),
        level: 1,
    }]);
    CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level: 20,
        xp: 100,
        hp: 100,
        mp: 40,
        alive: true,
        position: Position { x: 126.0, y: 126.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
    }
}
fn event(c: &Character, key: IdempotencyKey) -> DomainEvent {
    DomainEvent::CharacterClassChanged {
        metadata: EventMetadata {
            event_id: Uuid::now_v7(),
            sequence: (1, 0),
        },
        character_id: c.id,
        old_class_id: ClassId(0),
        new_class_id: ClassId(1),
        tick: 7,
        request_key: key.as_uuid(),
    }
}
async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assert_level_40_catchup_ranks(
    repo: &PgCharacterRepository,
    pool: &PgPool,
    id: CharacterId,
) {
    // Literal ranks from the independent pinned XML oracle, not auto_get_metadata.
    let expected = vec![
        ("l2.skill.1320", 4),
        ("l2.skill.1322", 1),
        ("l2.skill.194", 1),
        ("l2.skill.239", 2),
        ("racial.adaptable", 1),
    ];
    let character = repo.get(id).await.unwrap().unwrap();
    let admission = repo.load_for_admission(id).await.unwrap().unwrap();
    for skills in [
        &character.class_state.learned_skills,
        &admission.class_state.learned_skills,
    ] {
        assert_eq!(
            skills
                .iter()
                .map(|skill| (skill.key.as_str(), skill.level))
                .collect::<Vec<_>>(),
            expected
        );
    }
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT skill_key,skill_level FROM character_learned_skills WHERE character_id=$1 AND slot=0 ORDER BY skill_key",
    )
    .bind(id.as_uuid())
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        rows.iter()
            .map(|(key, rank)| (key.as_str(), u32::try_from(*rank).unwrap()))
            .collect::<Vec<_>>(),
        expected
    );
}

#[tokio::test]
async fn mystic_creation_round_trips_identity_and_normalized_main_slot() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let mut c = character("Mystic");
    c.class_state = ClassState::new(ClassId(10));
    c.appearance.sex = Sex::Female;
    let classes = nightfall_api::infrastructure::class_data::load_classes(
        &nightfall_api::infrastructure::class_data::ClassSource::embedded(),
    )
    .unwrap();
    c.stats = classes.registry.get(ClassId(10)).unwrap().base_stats;
    c.class_state.merge_learned_skills([LearnedSkill {
        key: "racial.adaptable".into(),
        level: 1,
    }]);
    repo.create_idempotent(&IdempotencyKey::new(), "mystic", &c)
        .await
        .unwrap();
    assert_eq!(repo.get(c.id).await.unwrap().unwrap(), c);
    let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(loaded.identity, c.identity());
    assert_eq!(loaded.class_profile, "human_mystic");
    assert_eq!(loaded.class_state, c.class_state);
    let slot: (i16, i32, i32, i64, i64) = sqlx::query_as(
        "SELECT slot,class_id,level,exp,sp FROM character_class_slots WHERE character_id=$1",
    )
    .bind(c.id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(slot, (0, 10, 1, 0, 0));
    assert_eq!(count(&pool, "character_learned_skills").await, 1);
}

#[tokio::test]
async fn distinct_concurrent_create_keys_never_exceed_seven_slots_and_retry_replays_at_capacity() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
    let first = character("Firsthero");
    let first_key = IdempotencyKey::new();
    repo.create_idempotent(&first_key, "first", &first)
        .await
        .unwrap();
    let mut tasks = Vec::new();
    for suffix in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l'] {
        let repo = repo.clone();
        tasks.push(tokio::spawn(async move {
            let c = character(&format!("Hero{suffix}"));
            repo.create_idempotent(&IdempotencyKey::new(), c.name.as_str(), &c)
                .await
        }));
    }
    let mut made = 0;
    let mut full = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(CreateOutcome::Created(_)) => made += 1,
            out => {
                assert!(matches!(out, Err(RepositoryError::SlotsFull)), "unexpected {out:?}");
                full += 1;
            },
        }
    }
    assert_eq!((made, full), (6, 6));
    assert_eq!(count(&pool, "characters").await, 7);
    assert_eq!(count(&pool, "character_class_slots").await, 7);
    assert_eq!(count(&pool, "idempotency_keys").await, 7);
    assert_eq!(count(&pool, "outbox").await, 7);
    assert_eq!(
        repo.create_idempotent(&first_key, "first", &first)
            .await
            .unwrap(),
        CreateOutcome::Replayed(first)
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One two-tier persisted history proves frozen retry cannot mutate the later ledger.
async fn transfer_checkpoint_atomically_round_trips_ledger_receipt_skills_and_outbox() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let mut c = character("Hero");
    c.level = 40;
    c.xp = 500;
    c.class_state.token_tier_1_count = 1;
    c.class_state.token_tier_2_count = 1;
    c.class_state.learned_skills = vec![LearnedSkill {
        key: "racial.adaptable".into(),
        level: 1,
    }];
    repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
        .await
        .unwrap();
    let key = IdempotencyKey::new();
    assert_eq!(repo.get(c.id).await.unwrap().unwrap(), c);
    let mut first_result = frozen(&c, 1);
    first_result.level = 40;
    first_result.xp = 500;
    first_result.granted_skill_keys = vec![
        "l2.skill.1320".into(),
        "l2.skill.1322".into(),
        "l2.skill.194".into(),
        "l2.skill.239".into(),
    ];
    let mut cp = checkpoint(&c, key);
    cp.level = 40;
    cp.xp = 500;
    let ledger = cp.class_state.as_mut().unwrap();
    ledger.token_tier_1_count = 0;
    ledger.successful_transfer_receipts[0].result = first_result.clone();
    // Sparse fixture catch-up from the independent XML oracle; no production learning helper.
    ledger.merge_learned_skills([
        LearnedSkill {
            key: "l2.skill.1320".into(),
            level: 4,
        },
        LearnedSkill {
            key: "l2.skill.1322".into(),
            level: 1,
        },
        LearnedSkill {
            key: "l2.skill.194".into(),
            level: 1,
        },
        LearnedSkill {
            key: "l2.skill.239".into(),
            level: 2,
        },
    ]);
    let fact = event(&c, key);
    assert_eq!(
        repo.checkpoint(&cp, std::slice::from_ref(&fact))
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
    assert_eq!(
        repo.checkpoint(&cp, std::slice::from_ref(&fact))
            .await
            .unwrap(),
        CheckpointOutcome::Replayed(1)
    );
    let loaded = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(loaded.class_state, cp.class_state.clone().unwrap());
    assert_eq!(
        (loaded.level, loaded.xp, loaded.hp, loaded.mp, loaded.revision),
        (40, 500, Some(100), Some(40), 1)
    );
    assert_level_40_catchup_ranks(&repo, &pool, c.id).await;
    assert_eq!(
        repo.mutation_receipt_lookup(c.account_id, &key, &transfer_fingerprint(c.id, ClassId(1)))
            .await
            .unwrap(),
        MutationReceiptLookup::Known(first_result.clone())
    );
    assert_eq!(
        repo.mutation_receipt_lookup(c.account_id, &key, &transfer_fingerprint(c.id, ClassId(4)))
            .await
            .unwrap(),
        MutationReceiptLookup::Conflict
    );
    assert_eq!(
        repo.mutation_receipt_lookup(
            AccountId::from_uuid(Uuid::from_u128(8)),
            &key,
            &transfer_fingerprint(c.id, ClassId(1))
        )
        .await
        .unwrap(),
        MutationReceiptLookup::Unknown
    );
    let second_key = IdempotencyKey::new();
    let mut second_result = first_result.clone();
    second_result.current_class_id = ClassId(2);
    second_result.token_tier_2_count = 0;
    second_result.granted_skill_keys.clear();
    let mut second = CharacterCheckpoint {
        revision_seen: 1,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        ..cp.clone()
    };
    let ledger = second.class_state.as_mut().unwrap();
    ledger.current_class_id = ClassId(2);
    ledger.token_tier_2_count = 0;
    ledger
        .record_success(SuccessfulTransferReceipt {
            key: second_key.as_uuid(),
            target_class_id: ClassId(2),
            result: second_result.clone(),
        })
        .unwrap();
    let second_fact = DomainEvent::CharacterClassChanged {
        metadata: EventMetadata {
            event_id: Uuid::now_v7(),
            sequence: (2, 0),
        },
        character_id: c.id,
        old_class_id: ClassId(1),
        new_class_id: ClassId(2),
        tick: 8,
        request_key: second_key.as_uuid(),
    };
    assert_eq!(
        repo.checkpoint(&second, &[second_fact]).await.unwrap(),
        CheckpointOutcome::Applied(2)
    );
    assert_level_40_catchup_ranks(&repo, &pool, c.id).await;
    let before_retry = repo.get(c.id).await.unwrap().unwrap();
    assert_eq!(before_retry.class_state.current_class_id, ClassId(2));
    assert_eq!(before_retry.class_state, second.class_state.clone().unwrap());
    // A new repository instance re-admits from normalized DB state, not an in-memory ledger.
    let reconnected = PgCharacterRepository::new(pool.clone());
    let readmitted = reconnected.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(readmitted.class_state, before_retry.class_state);
    assert_level_40_catchup_ranks(&reconnected, &pool, c.id).await;
    for (request_key, target, result) in [(key, 1, &first_result), (second_key, 2, &second_result)]
    {
        assert_eq!(
            reconnected
                .mutation_receipt_lookup(
                    c.account_id,
                    &request_key,
                    &transfer_fingerprint(c.id, ClassId(target))
                )
                .await
                .unwrap(),
            MutationReceiptLookup::Known(result.clone())
        );
    }
    assert_eq!(reconnected.get(c.id).await.unwrap().unwrap(), before_retry);
    assert_level_40_catchup_ranks(&reconnected, &pool, c.id).await;
    let later = CharacterCheckpoint {
        revision_seen: 2,
        level: 40,
        xp: 500,
        hp: 80,
        position: Position { x: 1.0, y: 2.0 },
        class_state: None,
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        ..second.clone()
    };
    repo.checkpoint(&later, &[]).await.unwrap();
    assert_eq!(
        repo.mutation_receipt_lookup(c.account_id, &key, &transfer_fingerprint(c.id, ClassId(1)))
            .await
            .unwrap(),
        MutationReceiptLookup::Known(first_result)
    );
    let after = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(after.class_state, second.class_state.unwrap());
    assert_level_40_catchup_ranks(&repo, &pool, c.id).await;
    assert_eq!(count(&pool, "character_transfer_receipts").await, 2);
    assert_eq!(count(&pool, "character_learned_skills").await, 5);
    assert_eq!(count(&pool, "outbox").await, 3);
    let slot: (i32, i32, i64, i64) = sqlx::query_as(
        "SELECT class_id,level,exp,sp FROM character_class_slots WHERE character_id=$1 AND slot=0",
    )
    .bind(c.id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(slot, (2, 40, 500, 50));
}

#[tokio::test]
async fn failed_outbox_rolls_back_transfer_resources_receipt_skill_and_retry_key() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = character("Hero");
    repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
        .await
        .unwrap();
    pool.execute("ALTER TABLE outbox ADD CONSTRAINT outbox_reject_transfers CHECK (subject != 'nightfall.character.class_changed') NOT VALID").await.unwrap();
    let key = IdempotencyKey::new();
    let cp = checkpoint(&c, key);
    let fact = event(&c, key);
    assert!(matches!(
        repo.checkpoint(&cp, std::slice::from_ref(&fact)).await,
        Err(CheckpointError::Constraint(name)) if name == "outbox_reject_transfers"
    ));
    assert_eq!(repo.get(c.id).await.unwrap().unwrap(), c);
    assert_eq!(count(&pool, "character_transfer_receipts").await, 0);
    assert_eq!(count(&pool, "character_learned_skills").await, 0);
    assert_eq!(count(&pool, "idempotency_keys").await, 1);
    assert_eq!(count(&pool, "outbox").await, 1);
    assert_eq!(
        repo.mutation_receipt_lookup(c.account_id, &key, &transfer_fingerprint(c.id, ClassId(1)))
            .await
            .unwrap(),
        MutationReceiptLookup::Unknown
    );
    pool.execute("ALTER TABLE outbox DROP CONSTRAINT outbox_reject_transfers")
        .await
        .unwrap();
    repo.checkpoint(&cp, std::slice::from_ref(&fact))
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_identical_transfer_checkpoints_store_one_immutable_receipt() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = Arc::new(PgCharacterRepository::new(pool.clone()));
    let c = character("Hero");
    repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
        .await
        .unwrap();
    let key = IdempotencyKey::new();
    let cp = checkpoint(&c, key);
    let fact = event(&c, key);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let repo = repo.clone();
        let cp = cp.clone();
        let fact = fact.clone();
        tasks.push(tokio::spawn(async move { repo.checkpoint(&cp, &[fact]).await.unwrap() }));
    }
    let mut applied = 0;
    for task in tasks {
        if matches!(task.await.unwrap(), CheckpointOutcome::Applied(_)) {
            applied += 1;
        }
    }
    assert_eq!(applied, 1);
    assert_eq!(count(&pool, "character_transfer_receipts").await, 1);
    assert_eq!(count(&pool, "outbox").await, 2);
}

#[tokio::test]
async fn conflicting_receipt_key_across_characters_rolls_back_second_checkpoint() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let a = character("Hero");
    let b = character("Otherhero");
    for c in [&a, &b] {
        repo.create_idempotent(&IdempotencyKey::new(), c.name.as_str(), c)
            .await
            .unwrap();
    }
    let key = IdempotencyKey::new();
    repo.checkpoint(&checkpoint(&a, key), &[event(&a, key)])
        .await
        .unwrap();
    assert!(matches!(
        repo.checkpoint(&checkpoint(&b, key), &[event(&b, key)])
            .await,
        Err(CheckpointError::KeyReused)
    ));
    assert_eq!(repo.get(b.id).await.unwrap().unwrap(), b);
    assert_eq!(
        repo.load_for_admission(b.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );
    assert_eq!(count(&pool, "character_transfer_receipts").await, 1);
    assert_eq!(count(&pool, "outbox").await, 3);
}

#[tokio::test]
async fn forged_persisted_learning_metadata_fails_closed_at_read_and_admission() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = character("Hero");
    repo.create_idempotent(&IdempotencyKey::new(), "create", &c)
        .await
        .unwrap();
    let registry = nightfall_api::infrastructure::class_data::load_classes(
        &nightfall_api::infrastructure::class_data::ClassSource::embedded(),
    )
    .unwrap()
    .registry;
    let allowed =
        nightfall_api::domain::character_progression::auto_get_metadata(&registry, ClassId(0), 85)
            .unwrap();
    let foreign =
        nightfall_api::domain::character_progression::auto_get_metadata(&registry, ClassId(10), 85)
            .unwrap()
            .into_iter()
            .find(|s| !allowed.iter().any(|a| a == s))
            .unwrap();
    for (key, level) in [
        ("l2.skill.999999".to_owned(), 1),
        ("l2.skill.3".to_owned(), 10),
        ("racial.forest_step".to_owned(), 1),
        (foreign.key, foreign.level),
    ] {
        sqlx::query("INSERT INTO character_learned_skills (character_id, slot, skill_key, skill_level) VALUES ($1, 0, $2, $3)").bind(c.id.as_uuid()).bind(&key).bind(i32::try_from(level).unwrap()).execute(&pool).await.unwrap();
        assert!(repo.get(c.id).await.is_err(), "{key} level {level}");
        assert!(repo.load_for_admission(c.id).await.is_err(), "{key} level {level}");
        sqlx::query("DELETE FROM character_learned_skills WHERE character_id=$1")
            .bind(c.id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(repo.get(c.id).await.unwrap().unwrap(), c);
}
