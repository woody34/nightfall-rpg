//! D41 pending account/operation/key identity, preserved manifests and nonlossy rollback.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use nightfall_api::application::{
    ports::{ItemAdmissionRepository, ItemMutationOperation, PendingItemKind},
    CharacterRepository, IdempotencyKey,
};
use nightfall_api::domain::{AccountId, Character, CharacterName, Race};
use nightfall_api::infrastructure::postgres::{
    connection_from_pool, Migrator, PgCharacterRepository,
};
use sea_orm_migration::MigratorTrait;
use sqlx::PgPool;
use uuid::Uuid;

async fn historical_character(pool: &PgPool, account: Uuid, name: &str) -> Character {
    let c = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,base_class_id,current_class_id,class_profile,alive) VALUES($1,$2,$3,$4,'human',40,30,43,21,11,25,0,0,'human_fighter',true)")
        .bind(c.id.as_uuid()).bind(account).bind(name).bind(c.name.normalized()).execute(pool).await.unwrap();
    c
}
async fn current_character(repo: &PgCharacterRepository, account: Uuid, name: &str) -> Character {
    let c = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), name, &c)
        .await
        .unwrap();
    c
}
fn code(error: &sqlx::Error) -> String {
    error
        .as_database_error()
        .unwrap()
        .code()
        .unwrap()
        .into_owned()
}

#[tokio::test]
async fn upgrade_preserves_every_manifest_byte_and_legacy_backfill_reruns() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(10)).await.unwrap();
    let c = historical_character(&pool, Uuid::from_u128(1), "ScopeUpgrade").await;
    Migrator::up(&db, Some(1)).await.unwrap();
    let before: Vec<(Uuid,String,serde_json::Value,String,String)>=sqlx::query_as("SELECT request_id,manifest_hash,payload,request_fingerprint,status FROM pending_item_requests ORDER BY request_id").fetch_all(&pool).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let repo = PgCharacterRepository::new(pool.clone());
    let pending = repo.pending_item_requests(c.id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].account_id, c.account_id);
    assert_eq!(pending[0].operation, ItemMutationOperation::Backfill);
    assert_eq!(pending[0].kind, PendingItemKind::Backfill);
    let after: Vec<(Uuid,String,serde_json::Value,String,String)>=sqlx::query_as("SELECT request_id,manifest_hash,payload,request_fingerprint,status FROM pending_item_requests ORDER BY request_id").fetch_all(&pool).await.unwrap();
    assert_eq!(before, after);
    sqlx::raw_sql(include_str!(
        "../src/infrastructure/postgres/migrations/m20261012_000011_pending_item_backfill.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(repo.pending_item_requests(c.id).await.unwrap(), pending);
    Migrator::down(&db, Some(1)).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(repo.pending_item_requests(c.id).await.unwrap(), pending);
}
#[tokio::test]
async fn shared_uuid_is_legal_across_accounts_and_operations_and_owner_scope_is_checked() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let one = Uuid::from_u128(1);
    let two = Uuid::from_u128(2);
    let a = current_character(&repo, one, "ScopeAlpha").await;
    let b = current_character(&repo, two, "ScopeBeta").await;
    let c = current_character(&repo, one, "ScopeGamma").await;
    let d = current_character(&repo, one, "ScopeDelta").await;
    let key = Uuid::now_v7();
    for id in [a.id, b.id] {
        sqlx::query("UPDATE pending_item_requests SET request_id=$1 WHERE character_id=$2")
            .bind(key)
            .bind(id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE pending_item_requests SET request_id=$1,kind='backfill',operation='backfill' WHERE character_id=$2").bind(key).bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    for (character, expected) in [
        (&a, ItemMutationOperation::Starter),
        (&b, ItemMutationOperation::Starter),
        (&c, ItemMutationOperation::Backfill),
    ] {
        let read = repo.pending_item_requests(character.id).await.unwrap();
        assert_eq!(read[0].request_id, key);
        assert_eq!(read[0].account_id, character.account_id);
        assert_eq!(read[0].operation, expected);
    }
    let original = repo.pending_item_requests(d.id).await.unwrap();
    let error = sqlx::query("UPDATE pending_item_requests SET request_id=$1 WHERE character_id=$2")
        .bind(key)
        .bind(d.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&error), "23505");
    assert_eq!(repo.pending_item_requests(d.id).await.unwrap(), original);
    let error = sqlx::query("UPDATE pending_item_requests SET account_id=$1 WHERE character_id=$2")
        .bind(Uuid::from_u128(3))
        .bind(a.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&error), "23503");
    let error =
        sqlx::query("UPDATE pending_item_requests SET operation='backfill' WHERE character_id=$1")
            .bind(a.id.as_uuid())
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(code(&error), "23514");
    let error=sqlx::query("UPDATE pending_item_requests SET kind='grant',operation='grant_item' WHERE character_id=$1").bind(a.id.as_uuid()).execute(&pool).await.unwrap_err();
    assert_eq!(code(&error), "23514");
    let db = connection_from_pool(&pool);
    // 000013 (D76 row ordinals) rolls back first; 000012 then refuses.
    assert!(Migrator::down(&db, Some(2)).await.is_err());
    assert_eq!(repo.pending_item_requests(a.id).await.unwrap()[0].request_id, key);
    let version: i64 =
        sqlx::query_scalar("SELECT count(*) FROM seaql_migrations WHERE version LIKE '%000012%'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, 1);
}
#[tokio::test]
async fn concurrent_same_triple_has_one_winner_without_partial_updates() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let repo = PgCharacterRepository::new(pool.clone());
    let a = current_character(&repo, Uuid::from_u128(1), "ScopeParallelA").await;
    let b = current_character(&repo, Uuid::from_u128(1), "ScopeParallelB").await;
    let key = Uuid::now_v7();
    let left = sqlx::query("UPDATE pending_item_requests SET request_id=$1 WHERE character_id=$2")
        .bind(key)
        .bind(a.id.as_uuid())
        .execute(&pool);
    let right = sqlx::query("UPDATE pending_item_requests SET request_id=$1 WHERE character_id=$2")
        .bind(key)
        .bind(b.id.as_uuid())
        .execute(&pool);
    let (left, right) = tokio::join!(left, right);
    let outcomes = [left, right];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| result.as_ref().is_err_and(|error| code(error) == "23505"))
            .count(),
        1
    );
    let rows: i64=sqlx::query_scalar("SELECT count(*) FROM pending_item_requests WHERE account_id=$1 AND operation='starter' AND request_id=$2").bind(a.account_id.as_uuid()).bind(key).fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 1);
    assert_eq!(repo.pending_item_requests(a.id).await.unwrap().len(), 1);
    assert_eq!(repo.pending_item_requests(b.id).await.unwrap().len(), 1);
}
#[tokio::test]
async fn invalid_legacy_grant_pending_row_aborts_upgrade_without_losing_data() {
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(11)).await.unwrap();
    let c = historical_character(&pool, Uuid::from_u128(1), "ScopeInvalid").await;
    sqlx::query("INSERT INTO pending_item_requests(request_id,character_id,kind,request_fingerprint,manifest_version,manifest_hash,payload) VALUES($1,$2,'grant','legacy',1,repeat('0',64),'{\"template_id\":57,\"count\":1}')").bind(Uuid::now_v7()).bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    assert!(Migrator::up(&db, None).await.is_err());
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pending_item_requests WHERE kind='grant'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 1);
    let columns: i64=sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='pending_item_requests' AND column_name='account_id'").fetch_one(&pool).await.unwrap();
    assert_eq!(columns, 0);
}
