//! Runs only when `DATABASE_URL` is set. `PgAccountRepository` (Story 1.3).
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
use common::pg::migrated_pool;
use nightfall_api::application::ports::LoginOutcome;
use nightfall_api::application::AccountRepository;
use nightfall_api::domain::AccountId;
use nightfall_api::infrastructure::postgres::PgAccountRepository;
use uuid::Uuid;

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap()
}

const MIN: Duration = Duration::seconds(60);

async fn row(pool: &sqlx::PgPool, id: AccountId) -> (DateTime<Utc>, DateTime<Utc>) {
    sqlx::query_as("SELECT created_at, last_login_at FROM accounts WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM accounts")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn two_logins_create_one_row() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgAccountRepository::new(pool.clone());
    let id = AccountId::from_uuid(Uuid::now_v7());
    assert_eq!(repo.record_login(id, t0(), MIN).await.unwrap(), LoginOutcome::Created);
    assert_eq!(
        repo.record_login(id, t0() + Duration::seconds(1), MIN)
            .await
            .unwrap(),
        LoginOutcome::Unchanged
    );
    assert_eq!(count(&pool).await, 1);
    assert_eq!(row(&pool, id).await, (t0(), t0()), "inside the interval nothing is written");
}

#[tokio::test]
async fn last_login_is_bumped_after_the_interval_and_created_at_never_moves() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = PgAccountRepository::new(pool.clone());
    let id = AccountId::from_uuid(Uuid::now_v7());
    repo.record_login(id, t0(), MIN).await.unwrap();
    let later = t0() + MIN;
    assert_eq!(repo.record_login(id, later, MIN).await.unwrap(), LoginOutcome::Bumped);
    assert_eq!(row(&pool, id).await, (t0(), later));
}

#[tokio::test]
async fn concurrent_first_logins_create_exactly_one_row() {
    let Some(pool) = migrated_pool().await else {
        return;
    };
    let repo = Arc::new(PgAccountRepository::new(pool.clone()));
    let id = AccountId::from_uuid(Uuid::now_v7());
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let repo = repo.clone();
            tokio::spawn(async move { repo.record_login(id, t0(), MIN).await.unwrap() })
        })
        .collect();
    let mut created = 0;
    for h in handles {
        if h.await.unwrap() == LoginOutcome::Created {
            created += 1;
        }
    }
    assert_eq!(created, 1);
    assert_eq!(count(&pool).await, 1);
}
