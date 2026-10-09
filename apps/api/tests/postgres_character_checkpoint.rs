//! Story E4.1: the progression checkpoint transaction. Runs only when `DATABASE_URL` is set.
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::pg::migrated_pool as fresh_pool;
use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::{AccountId, Character, CharacterName, DomainEvent, Position, Race};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use sqlx::{Executor, PgPool};
use uuid::Uuid;

const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";

fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}

async fn seeded(pool: &PgPool) -> (PgCharacterRepository, Character) {
    let repo = PgCharacterRepository::new(pool.clone());
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Durin").unwrap(),
        Race::Dwarf,
    );
    repo.create_idempotent(&key("0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d90"), "fp", &c)
        .await
        .unwrap();
    (repo, c)
}

fn checkpoint(c: &Character, revision_seen: u64, k: &str) -> CharacterCheckpoint {
    CharacterCheckpoint {
        character_id: c.id,
        revision_seen,
        level: 3,
        xp: 1_234,
        hp: 77,
        mp: 31,
        alive: true,
        position: Position { x: 12.5, y: -3.25 },
        idempotency: ("save_checkpoint".to_owned(), key(k)),
    }
}

fn leveled(c: &Character, level: u32) -> DomainEvent {
    DomainEvent::CharacterLeveled {
        metadata: nightfall_api::domain::EventMetadata::default(),
        character_id: c.id,
        level,
    }
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn new_character_admits_with_defaults() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!((p.level, p.xp, p.hp, p.mp, p.alive, p.revision), (1, 0, None, None, true, 0));
    assert_eq!(p.class_profile, "dwarven_fighter");
    assert_eq!(repo.load_for_admission(Uuid::nil().into()).await.unwrap(), None);
}

#[tokio::test]
async fn checkpoint_round_trips_and_stages_events() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let outbox_before = count(&pool, "outbox").await;
    let events = [leveled(&c, 2), leveled(&c, 3)];

    let out = repo
        .checkpoint(&checkpoint(&c, 0, KEY_A), &events)
        .await
        .unwrap();
    assert_eq!(out, CheckpointOutcome::Applied(1));

    let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(
        (p.level, p.xp, p.hp, p.mp, p.alive, p.revision),
        (3, 1_234, Some(77), Some(31), true, 1)
    );
    let stored = repo.get(c.id).await.unwrap().unwrap();
    assert_eq!(stored.level, 3);
    assert_eq!(stored.position, Position { x: 12.5, y: -3.25 });

    let subjects: Vec<String> =
        sqlx::query_scalar("SELECT subject FROM outbox WHERE subject LIKE '%leveled' ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(subjects, ["nightfall.character.leveled"; 2]);
    assert_eq!(count(&pool, "outbox").await, outbox_before + 2);

    // The next checkpoint continues from the new revision; a dead state persists.
    let mut dead = checkpoint(&c, 1, KEY_B);
    dead.alive = false;
    dead.hp = 0;
    let died = DomainEvent::CharacterDied {
        metadata: nightfall_api::domain::EventMetadata::default(),
        level: 1,
        xp: 0,
        xp_lost: 0,
        character_id: c.id,
        killer: "npc:1".to_owned(),
    };
    assert_eq!(repo.checkpoint(&dead, &[died]).await.unwrap(), CheckpointOutcome::Applied(2));
    let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!((p.alive, p.hp, p.revision), (false, Some(0), 2));
}

#[tokio::test]
async fn out_of_range_values_are_typed_constraint_errors() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    for (level, xp, name) in [
        (0, 0, "characters_level_range"),
        (86, 0, "characters_level_range"),
        (1, u64::MAX, "characters_xp_nonneg"),
    ] {
        let mut cp = checkpoint(&c, 0, KEY_A);
        cp.level = level;
        cp.xp = xp;
        let err = repo.checkpoint(&cp, &[]).await.unwrap_err();
        assert!(matches!(&err, CheckpointError::Constraint(n) if n == name), "{err:?}");
    }
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        0
    );

    // The database itself also refuses negative resources and bad levels.
    for sql in [
        "UPDATE characters SET hp = -1",
        "UPDATE characters SET mp = -1",
        "UPDATE characters SET xp = -1",
        "UPDATE characters SET level = 86",
        "UPDATE characters SET level = 0",
    ] {
        assert!(pool.execute(sql).await.is_err(), "{sql}");
    }
    pool.execute("UPDATE characters SET level = 85")
        .await
        .unwrap();
}

#[tokio::test]
async fn failure_after_row_update_leaves_no_partial_state() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let outbox_before = count(&pool, "outbox").await;
    let keys_before = count(&pool, "idempotency_keys").await;

    pool.execute("ALTER TABLE outbox ADD CONSTRAINT outbox_reject_all CHECK (false) NOT VALID")
        .await
        .unwrap();
    let err = repo
        .checkpoint(&checkpoint(&c, 0, KEY_A), &[leveled(&c, 3)])
        .await
        .unwrap_err();
    assert!(matches!(err, CheckpointError::Other(_)), "{err:?}");

    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before);
    assert_eq!(repo.get(c.id).await.unwrap().unwrap().level, 1);
    assert_eq!(count(&pool, "outbox").await, outbox_before);
    assert_eq!(count(&pool, "idempotency_keys").await, keys_before);

    // With the fault gone the same key and revision apply cleanly.
    pool.execute("ALTER TABLE outbox DROP CONSTRAINT outbox_reject_all")
        .await
        .unwrap();
    assert_eq!(
        repo.checkpoint(&checkpoint(&c, 0, KEY_A), &[leveled(&c, 3)])
            .await
            .unwrap(),
        CheckpointOutcome::Applied(1)
    );
}

#[tokio::test]
async fn stale_revision_is_rejected_and_writes_nothing() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    repo.checkpoint(&checkpoint(&c, 0, KEY_A), &[])
        .await
        .unwrap();
    let outbox_before = count(&pool, "outbox").await;
    let keys_before = count(&pool, "idempotency_keys").await;

    let mut stale = checkpoint(&c, 0, KEY_B);
    stale.xp = 9_999;
    assert_eq!(
        repo.checkpoint(&stale, &[leveled(&c, 9)]).await.unwrap(),
        CheckpointOutcome::Stale
    );
    let p = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!((p.xp, p.revision), (1_234, 1));
    assert_eq!(count(&pool, "outbox").await, outbox_before);
    assert_eq!(count(&pool, "idempotency_keys").await, keys_before, "stale key not consumed");
}

#[tokio::test]
async fn same_key_replays_and_different_body_is_rejected() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let cp = checkpoint(&c, 0, KEY_A);
    assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Applied(1));
    // Replay works even though revision_seen is now behind the stored revision.
    assert_eq!(repo.checkpoint(&cp, &[]).await.unwrap(), CheckpointOutcome::Replayed(1));

    let mut changed = cp.clone();
    changed.xp += 1;
    let err = repo.checkpoint(&changed, &[]).await.unwrap_err();
    assert!(matches!(err, CheckpointError::KeyReused), "{err:?}");
    // Same checkpoint but different staged events is also a different body.
    let err = repo.checkpoint(&cp, &[leveled(&c, 3)]).await.unwrap_err();
    assert!(matches!(err, CheckpointError::KeyReused), "{err:?}");

    let missing = CharacterCheckpoint {
        character_id: Uuid::from_u128(9).into(),
        ..cp
    };
    let err = repo.checkpoint(&missing, &[]).await.unwrap_err();
    assert!(matches!(err, CheckpointError::NotFound), "{err:?}");
}

#[tokio::test]
async fn eight_concurrent_identical_checkpoints_apply_exactly_once() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let outbox_before = count(&pool, "outbox").await;

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let repo = repo.clone();
        let cp = checkpoint(&c, 0, KEY_A);
        let events = [leveled(&c, 2)];
        tasks.push(tokio::spawn(async move { repo.checkpoint(&cp, &events).await.unwrap() }));
    }
    let mut outcomes = Vec::new();
    for t in tasks {
        outcomes.push(t.await.unwrap());
    }
    let applied = outcomes
        .iter()
        .filter(|o| **o == CheckpointOutcome::Applied(1))
        .count();
    let replayed = outcomes
        .iter()
        .filter(|o| **o == CheckpointOutcome::Replayed(1))
        .count();
    assert_eq!((applied, replayed), (1, 7));
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(count(&pool, "outbox").await, outbox_before + 1);
}

#[tokio::test]
async fn concurrent_different_keys_at_one_revision_apply_once() {
    let Some(pool) = fresh_pool().await else {
        return;
    };
    let (repo, c) = seeded(&pool).await;
    let a = {
        let repo = repo.clone();
        let cp = checkpoint(&c, 0, KEY_A);
        tokio::spawn(async move { repo.checkpoint(&cp, &[]).await.unwrap() })
    };
    let b = {
        let repo = repo.clone();
        let cp = checkpoint(&c, 0, KEY_B);
        tokio::spawn(async move { repo.checkpoint(&cp, &[]).await.unwrap() })
    };
    let mut got = [a.await.unwrap(), b.await.unwrap()];
    got.sort_by_key(|o| matches!(o, CheckpointOutcome::Stale));
    assert_eq!(got, [CheckpointOutcome::Applied(1), CheckpointOutcome::Stale]);
}
