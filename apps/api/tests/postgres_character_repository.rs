//! Runs only when `DATABASE_URL` is set (docker compose up -d). Each test uses a fresh
//! schema so tests are independent and can run in parallel.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use nightfall_api::domain::{Character, CharacterName, Race};
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, MIGRATOR};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

async fn fresh_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("test_{}", Uuid::now_v7().simple());
    admin
        .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}"))))
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect({
            let schema = schema.clone();
            move |conn, _| {
                let schema = schema.clone();
                Box::pin(async move {
                    conn.execute(sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                        "SET search_path TO {schema}"
                    ))))
                    .await?;
                    Ok(())
                })
            }
        })
        .connect(&url)
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    Some(pool)
}

fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}

const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

#[tokio::test]
async fn create_then_get_round_trips() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);

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
    let c = Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp", &c).await.unwrap();

    let retry = Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
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
    let c = Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
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
    let a = Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
    repo.create_idempotent(&key(KEY_A), "fp", &a).await.unwrap();

    let b = Character::create(Uuid::nil(), CharacterName::new("DURIN").unwrap(), Race::Elf);
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
            let c =
                Character::create(Uuid::nil(), CharacterName::new("Durin").unwrap(), Race::Dwarf);
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
