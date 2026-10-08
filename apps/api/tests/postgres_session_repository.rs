//! Runs only when `DATABASE_URL` is set. `PgSessionRepository` (Story 1.4): idempotent issue,
//! single-use consume, generations, atomicity, and concurrency.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic
)]

mod common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use common::pg::{insert_account, migrated_pool};
use nightfall_api::application::ports::{ConsumeError, IssueOutcome, IssuedTicket, NewTicket};
use nightfall_api::application::{CharacterRepository, IdempotencyKey, SessionRepository};
use nightfall_api::domain::{AccountId, Character, CharacterId, CharacterName, PlayTicket, Race};
use nightfall_api::infrastructure::postgres::{PgCharacterRepository, PgSessionRepository};
use sqlx::PgPool;
use uuid::Uuid;

const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap()
}

fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}

struct Fixture {
    pool: PgPool,
    repo: PgSessionRepository,
    account: AccountId,
    character: CharacterId,
}

async fn fixture() -> Option<Fixture> {
    let pool = migrated_pool().await?;
    let account = AccountId::from_uuid(Uuid::now_v7());
    insert_account(&pool, account.as_uuid()).await;
    let c = Character::create(account, CharacterName::new("Aria").unwrap(), Race::Elf);
    PgCharacterRepository::new(pool.clone())
        .create_idempotent(&key("0190a7e2-0000-7000-8000-000000000000"), "seed", &c)
        .await
        .unwrap();
    Some(Fixture {
        repo: PgSessionRepository::new(pool.clone()),
        pool,
        account,
        character: c.id,
    })
}

fn ticket(f: &Fixture, byte: u8) -> NewTicket {
    let secret = PlayTicket::from_bytes([byte; 32]);
    NewTicket {
        account_id: f.account,
        character_id: f.character,
        hash: secret.hash(),
        issued_at: t0(),
        response: IssuedTicket {
            ticket: secret,
            expires_at: t0() + Duration::seconds(60),
            ws_url: "ws://test/ws".into(),
        },
    }
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn issue_stores_only_the_hash_and_bumps_the_generation() {
    let Some(f) = fixture().await else {
        return;
    };
    let t = ticket(&f, 1);
    let out = f
        .repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &t)
        .await
        .unwrap();
    let IssueOutcome::Issued {
        response,
        generation,
    } = out
    else {
        panic!("expected Issued, got {out:?}");
    };
    assert_eq!(response, t.response);
    assert_eq!(generation.get(), 1);

    let stored: Vec<u8> = sqlx::query_scalar("SELECT ticket_hash FROM play_tickets")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(stored, t.hash.as_bytes().to_vec());
    assert_ne!(stored, vec![1u8; 32], "the secret itself is not stored in play_tickets");

    let t2 = ticket(&f, 2);
    let out = f
        .repo
        .issue_ticket_idempotent(&key(KEY_B), "fp", &t2)
        .await
        .unwrap();
    assert!(matches!(out, IssueOutcome::Issued { generation, .. } if generation.get() == 2));
}

#[tokio::test]
async fn same_key_replays_the_stored_response_and_writes_nothing() {
    let Some(f) = fixture().await else {
        return;
    };
    let first = ticket(&f, 1);
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &first)
        .await
        .unwrap();
    let retry = ticket(&f, 9);
    let out = f
        .repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &retry)
        .await
        .unwrap();
    assert_eq!(out, IssueOutcome::Replayed(first.response));
    assert_eq!(count(&f.pool, "play_tickets").await, 1);
    let g: i32 = sqlx::query_scalar("SELECT generation FROM account_sessions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(g, 1);
}

#[tokio::test]
async fn same_key_different_fingerprint_is_key_reused() {
    let Some(f) = fixture().await else {
        return;
    };
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp1", &ticket(&f, 1))
        .await
        .unwrap();
    let out = f
        .repo
        .issue_ticket_idempotent(&key(KEY_A), "fp2", &ticket(&f, 2))
        .await
        .unwrap();
    assert_eq!(out, IssueOutcome::KeyReused);
}

#[tokio::test]
async fn failed_issue_leaves_no_key_no_ticket_and_no_generation() {
    let Some(f) = fixture().await else {
        return;
    };
    let mut t = ticket(&f, 1);
    t.character_id = CharacterId::new(); // violates play_tickets_character_id_fkey
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &t)
        .await
        .unwrap_err();
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM idempotency_keys WHERE operation = 'issue_play_ticket'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(keys, 0);
    assert_eq!(count(&f.pool, "play_tickets").await, 0);
    assert_eq!(count(&f.pool, "account_sessions").await, 0);
}

#[tokio::test]
async fn consume_once_then_consumed() {
    let Some(f) = fixture().await else {
        return;
    };
    let t = ticket(&f, 1);
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &t)
        .await
        .unwrap();
    let a = f.repo.consume_ticket(&t.hash, t0()).await.unwrap();
    assert_eq!((a.account_id, a.character_id, a.generation.get()), (f.account, f.character, 1));
    assert!(matches!(
        f.repo.consume_ticket(&t.hash, t0()).await,
        Err(ConsumeError::Consumed)
    ));
}

#[tokio::test]
async fn expired_and_unknown_tickets_are_refused() {
    let Some(f) = fixture().await else {
        return;
    };
    let t = ticket(&f, 1);
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &t)
        .await
        .unwrap();
    assert!(matches!(
        f.repo
            .consume_ticket(&t.hash, t0() + Duration::seconds(60))
            .await,
        Err(ConsumeError::Expired)
    ));
    assert!(matches!(
        f.repo
            .consume_ticket(&PlayTicket::from_bytes([7; 32]).hash(), t0())
            .await,
        Err(ConsumeError::Unknown)
    ));
}

#[tokio::test]
async fn older_generation_is_superseded_and_still_spent() {
    let Some(f) = fixture().await else {
        return;
    };
    let old = ticket(&f, 1);
    let new = ticket(&f, 2);
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &old)
        .await
        .unwrap();
    f.repo
        .issue_ticket_idempotent(&key(KEY_B), "fp", &new)
        .await
        .unwrap();
    assert!(matches!(
        f.repo.consume_ticket(&old.hash, t0()).await,
        Err(ConsumeError::Superseded)
    ));
    assert!(matches!(
        f.repo.consume_ticket(&old.hash, t0()).await,
        Err(ConsumeError::Consumed)
    ));
    assert_eq!(
        f.repo
            .consume_ticket(&new.hash, t0())
            .await
            .unwrap()
            .generation
            .get(),
        2
    );
}

#[tokio::test]
async fn concurrent_consumes_of_one_ticket_admit_exactly_one() {
    let Some(f) = fixture().await else {
        return;
    };
    let t = ticket(&f, 1);
    f.repo
        .issue_ticket_idempotent(&key(KEY_A), "fp", &t)
        .await
        .unwrap();
    let repo = Arc::new(f.repo.clone());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let repo = repo.clone();
            let hash = t.hash;
            tokio::spawn(async move { repo.consume_ticket(&hash, t0()).await })
        })
        .collect();
    let mut admitted = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => admitted += 1,
            Err(ConsumeError::Consumed) => {},
            Err(e) => panic!("unexpected {e:?}"),
        }
    }
    assert_eq!(admitted, 1);
}

#[tokio::test]
async fn concurrent_retries_with_same_key_issue_exactly_one_ticket() {
    let Some(f) = fixture().await else {
        return;
    };
    let repo = Arc::new(f.repo.clone());
    let handles: Vec<_> = (0..8u8)
        .map(|i| {
            let repo = repo.clone();
            let t = ticket(&f, i);
            tokio::spawn(async move { repo.issue_ticket_idempotent(&key(KEY_A), "fp", &t).await })
        })
        .collect();
    let mut issued = Vec::new();
    let mut replayed = Vec::new();
    for h in handles {
        match h.await.unwrap().unwrap() {
            IssueOutcome::Issued { response, .. } => issued.push(response),
            IssueOutcome::Replayed(r) => replayed.push(r),
            IssueOutcome::KeyReused => panic!("same fingerprint"),
        }
    }
    assert_eq!(issued.len(), 1);
    assert!(replayed.iter().all(|r| *r == issued[0]), "every retry sees the winner's ticket");
    assert_eq!(count(&f.pool, "play_tickets").await, 1);
}
