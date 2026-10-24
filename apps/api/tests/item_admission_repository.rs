//! Memory/Postgres pending-kit and exactly-once allocator parity; no DB-only owned grants.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use nightfall_api::application::ports::{
    ItemAdmissionRepository, PendingItemKind, PendingItemManifest, PendingItemStatus,
};
use nightfall_api::application::{CharacterRepository, IdempotencyKey};
use nightfall_api::domain::{zone::ZoneId, AccountId, Character, CharacterName, Race};
use nightfall_api::infrastructure::{
    memory::InMemoryCharacterRepository, postgres::PgCharacterRepository,
};
use sqlx::Executor;
use uuid::Uuid;

async fn parity<R: CharacterRepository + ItemAdmissionRepository>(repo: &R) {
    let character = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("KitOwner").unwrap(),
        Race::Orc,
    );
    let key = IdempotencyKey::from_uuid(Uuid::now_v7());
    repo.create_idempotent(&key, "same", &character)
        .await
        .unwrap();
    let first = repo.pending_item_requests(character.id).await.unwrap();
    assert_eq!(first.len(), 1);
    let request = first.first().unwrap();
    assert_eq!(request.kind, PendingItemKind::Starter);
    assert_eq!(request.status, PendingItemStatus::Pending);
    let PendingItemManifest::Kit(kit) = &request.manifest;
    assert_eq!(kit.source_class_id.0, 44);
    assert_eq!(kit.hash().unwrap(), request.manifest_hash);
    assert_eq!(kit.items.first().unwrap().template_id, 2369);
    assert!(!kit.items.first().unwrap().equipped);
    assert!(kit
        .items
        .iter()
        .any(|item| item.template_id == 2368 && item.equipped));
    repo.create_idempotent(&key, "same", &character)
        .await
        .unwrap();
    assert_eq!(repo.pending_item_requests(character.id).await.unwrap(), first);
    let (a, b) =
        tokio::join!(repo.reserve_item_range(ZoneId(1), 1), repo.reserve_item_range(ZoneId(1), 1));
    let a = a.unwrap();
    assert_eq!(b.unwrap(), a);
    assert_eq!(a.range_start, 1_u64 << 32);
    assert_eq!(a.next_reserved, a.range_start);
    let c = repo.reserve_item_range(ZoneId(2), 1).await.unwrap();
    assert_eq!(c.range_start, a.range_end_exclusive);
    assert!(repo.reserve_item_range(ZoneId(u32::MAX), 1).await.is_err());
    assert!(repo.reserve_item_range(ZoneId(1), u64::MAX).await.is_err());
}

#[tokio::test]
async fn memory_pending_and_ranges_are_atomic() {
    parity(&InMemoryCharacterRepository::default()).await;
}

#[tokio::test]
async fn postgres_pending_and_ranges_are_atomic_and_concurrent() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    parity(&PgCharacterRepository::new(pool.clone())).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pending_item_requests")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM item_instances")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM item_id_ranges")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    sqlx::query("UPDATE pending_item_requests SET manifest_hash=repeat('0',64)")
        .execute(&pool)
        .await
        .unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT id FROM characters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(PgCharacterRepository::new(pool)
        .pending_item_requests(nightfall_api::domain::CharacterId::from_uuid(owner))
        .await
        .is_err());
}

#[tokio::test]
async fn pending_step_failure_rolls_back_character_key_class_ledger_and_outbox() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    pool.execute(sqlx::raw_sql("CREATE FUNCTION reject_pending_item_request() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected pending failure'; END $$; CREATE TRIGGER reject_pending BEFORE INSERT ON pending_item_requests FOR EACH ROW EXECUTE FUNCTION reject_pending_item_request()" )).await.unwrap();
    let repo = PgCharacterRepository::new(pool.clone());
    let character = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("AtomicKit").unwrap(),
        Race::Human,
    );
    let key = IdempotencyKey::from_uuid(Uuid::now_v7());
    assert!(repo
        .create_idempotent(&key, "same", &character)
        .await
        .is_err());
    for query in [
        "SELECT count(*) FROM characters",
        "SELECT count(*) FROM idempotency_keys",
        "SELECT count(*) FROM character_class_slots",
        "SELECT count(*) FROM pending_item_requests",
        "SELECT count(*) FROM outbox",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    pool.execute(sqlx::raw_sql("DROP TRIGGER reject_pending ON pending_item_requests"))
        .await
        .unwrap();
    repo.create_idempotent(&key, "same", &character)
        .await
        .unwrap();
    assert_eq!(
        repo.pending_item_requests(character.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn frozen_backfill_hash_and_manifest_are_accepted_by_typed_admission_loader() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let character = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("BackfillOrc").unwrap(),
        Race::Orc,
    );
    repo.create_idempotent(&IdempotencyKey::from_uuid(Uuid::now_v7()), "backfill-seed", &character)
        .await
        .unwrap();
    sqlx::query("DELETE FROM pending_item_requests WHERE character_id=$1")
        .bind(character.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    pool.execute(sqlx::raw_sql(include_str!(
        "../src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql"
    )))
    .await
    .unwrap();
    let first = repo.pending_item_requests(character.id).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first.first().unwrap().kind, PendingItemKind::Backfill);
    pool.execute(sqlx::raw_sql(include_str!(
        "../src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql"
    )))
    .await
    .unwrap();
    assert_eq!(repo.pending_item_requests(character.id).await.unwrap(), first);
}
