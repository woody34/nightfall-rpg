//! Runs only when `DATABASE_URL` is set (docker compose up -d). Each test uses a fresh
//! schema so tests are independent and can run in parallel.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::pg::migrated_pool as fresh_pool;
use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use nightfall_api::domain::{AccountId, Character, CharacterName, Race};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

fn owner() -> AccountId {
    AccountId::from_uuid(Uuid::nil())
}

fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}

const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

#[tokio::test]
async fn idempotency_record_is_scoped_to_account_and_operation() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap();

    let (account, operation, response): (Uuid, String, serde_json::Value) = sqlx::query_as(
        "SELECT account_id, operation, response FROM idempotency_keys WHERE key = $1",
    )
    .bind(Uuid::parse_str(KEY_A).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(account, owner().as_uuid());
    assert_eq!(operation, "create_character");
    assert_eq!(response, serde_json::json!({ "character_id": c.id.as_uuid() }));

    // The same key from another account is a different request.
    let other = AccountId::from_uuid(Uuid::from_u128(2));
    let theirs = Character::create(other, CharacterName::new("Balin").unwrap(), Race::Dwarf);
    let out = repo
        .create_idempotent(&key(KEY_A), "fp-other", &theirs)
        .await
        .unwrap();
    assert_eq!(out, CreateOutcome::Created(theirs));
}

#[tokio::test]
async fn list_by_account_returns_own_characters_in_creation_order() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool);
    let other = AccountId::from_uuid(Uuid::from_u128(2));
    let a = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    let b = Character::create(other, CharacterName::new("Balin").unwrap(), Race::Dwarf);
    let c = Character::create(owner(), CharacterName::new("Thorin").unwrap(), Race::Dwarf);
    // Insert out of order: the result is ordered by id (uuid v7 = creation time), not insert.
    repo.create_idempotent(&key(KEY_A), "a", &c).await.unwrap();
    repo.create_idempotent(&key(KEY_B), "b", &b).await.unwrap();
    repo.create_idempotent(&key("0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d90"), "c", &a)
        .await
        .unwrap();

    assert_eq!(repo.list_by_account(owner()).await.unwrap(), vec![a, c]);
    assert_eq!(repo.list_by_account(other).await.unwrap(), vec![b]);
    assert!(repo
        .list_by_account(AccountId::from_uuid(Uuid::from_u128(3)))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn create_then_get_round_trips() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);

    let out = repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap();
    assert_eq!(out, CreateOutcome::Created(c.clone()));
    assert_eq!(repo.get(c.id).await.unwrap(), Some(c.clone()));

    let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox WHERE published_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(outbox, 1, "event staged in the same transaction");
}

#[tokio::test]
async fn same_key_replays_and_writes_nothing() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap();

    let retry = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    let out = repo
        .create_idempotent(&key(KEY_A), "fp", &retry)
        .await
        .unwrap();
    assert_eq!(out, CreateOutcome::Replayed(c));

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM characters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn same_key_different_fingerprint_is_key_reused() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool);
    let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp1", &c)
        .await
        .unwrap();
    let out = repo
        .create_idempotent(&key(KEY_A), "fp2", &c)
        .await
        .unwrap();
    assert_eq!(out, CreateOutcome::KeyReused);
}

#[tokio::test]
async fn duplicate_name_rolls_back_key_and_character() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let a = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp", &a).await.unwrap();

    let b = Character::create(owner(), CharacterName::new("DURIN").unwrap(), Race::Elf);
    let err = repo
        .create_idempotent(&key(KEY_B), "fp-b", &b)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::NameTaken));

    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM idempotency_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(keys, 1, "the failed request's key was rolled back atomically");
}

#[tokio::test]
async fn concurrent_retries_with_same_key_create_exactly_one() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = std::sync::Arc::new(PgCharacterRepository::new(pool.clone()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let repo = repo.clone();
        handles.push(tokio::spawn(async move {
            let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
            repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap()
        }));
    }
    let mut created = 0;
    for h in handles {
        if matches!(h.await.unwrap(), CreateOutcome::Created(_)) {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM characters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The outbox insert is the last write of the transaction. Forcing it to fail (a CHECK that
/// rejects every new row, in this test's private schema) must undo the character and the key.
#[tokio::test]
async fn outbox_insert_failure_rolls_back_character_key_and_event() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    pool.execute("ALTER TABLE outbox ADD CONSTRAINT outbox_reject_all CHECK (false) NOT VALID")
        .await
        .unwrap();
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(owner(), CharacterName::new("Durin").unwrap(), Race::Dwarf);

    let err = repo
        .create_idempotent(&key(KEY_A), "fp", &c)
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Other(_)), "{err:?}");

    assert_eq!(count(&pool, "characters").await, 0, "character rolled back");
    assert_eq!(count(&pool, "idempotency_keys").await, 0, "key rolled back");
    assert_eq!(count(&pool, "outbox").await, 0, "no event staged");
    assert_eq!(repo.get(c.id).await.unwrap(), None);

    // With the fault removed the same key is usable: nothing was left half-claimed.
    pool.execute("ALTER TABLE outbox DROP CONSTRAINT outbox_reject_all")
        .await
        .unwrap();
    let out = repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap();
    assert_eq!(out, CreateOutcome::Created(c));
    assert_eq!(count(&pool, "outbox").await, 1);
}
