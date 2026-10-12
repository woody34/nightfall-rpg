//! Phase 3 schema defaults, strict retention thresholds and transactional rollback.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use nightfall_api::application::{CharacterRepository, IdempotencyKey};
use nightfall_api::domain::{AccountId, Character, CharacterName, Race};
use nightfall_api::infrastructure::postgres::PgCharacterRepository;
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn seed(pool: &PgPool) -> Character {
    let character = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("CombatSchema").unwrap(),
        Race::Human,
    );
    PgCharacterRepository::new(pool.clone())
        .create_idempotent(&IdempotencyKey::new(), "phase3-schema", &character)
        .await
        .unwrap();
    character
}

#[tokio::test]
async fn defaults_keep_legacy_progression_and_reject_invalid_scalars() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let character = seed(&pool).await;
    let row = sqlx::query("SELECT phase3_timer_policy_version,karma,pk_count,pvp_count,flag_remaining_ticks,death_penalty_level,recoverable_death_xp_loss,hp_fraction_q,mp_fraction_q,cp_fraction_q,sp FROM characters WHERE id=$1")
        .bind(character.id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<Option<i32>, _>("phase3_timer_policy_version"), None);
    for column in [
        "karma",
        "pk_count",
        "pvp_count",
        "flag_remaining_ticks",
        "recoverable_death_xp_loss",
        "hp_fraction_q",
        "mp_fraction_q",
        "cp_fraction_q",
        "sp",
    ] {
        assert_eq!(row.get::<i64, _>(column), 0);
    }
    assert_eq!(row.get::<i16, _>("death_penalty_level"), 0);
    for sql in [
        "UPDATE characters SET karma=-1 WHERE id=$1",
        "UPDATE characters SET pk_count=4294967296 WHERE id=$1",
        "UPDATE characters SET pvp_count=-1 WHERE id=$1",
        "UPDATE characters SET flag_remaining_ticks=-1 WHERE id=$1",
        "UPDATE characters SET death_penalty_level=6 WHERE id=$1",
        "UPDATE characters SET hp_fraction_q=1000000 WHERE id=$1",
        "UPDATE characters SET mp_fraction_q=-1 WHERE id=$1",
        "UPDATE characters SET cp_fraction_q=1000000 WHERE id=$1",
        "UPDATE characters SET recoverable_death_xp_loss=1 WHERE id=$1",
        "UPDATE characters SET death_incarnation=1 WHERE id=$1",
        "UPDATE characters SET modifier_install_order='{}'::jsonb WHERE id=$1",
        "UPDATE characters SET phase3_timer_policy_version=2 WHERE id=$1",
    ] {
        let error = sqlx::query(sql)
            .bind(character.id.as_uuid())
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23514")
        );
    }
    let ledger_sp: i64 =
        sqlx::query_scalar("SELECT sp FROM character_class_slots WHERE character_id=$1 AND slot=0")
            .bind(character.id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ledger_sp, 0);
}

const EFFECT_INSERT: &str = "INSERT INTO character_effects(character_id,retained_ordinal,skill_id,level,remaining_ticks,caster_kind,caster_id,caster_incarnation,pool,abnormal_type,abnormal_level,stay_after_death,stay_on_subclass_change,handlers) VALUES($1,0,$2,1,$3,'player',$1,1,'buff','NONE',0,false,false,'[]'::jsonb)";

#[tokio::test]
async fn strict_thresholds_and_child_failure_roll_back_scalar_and_other_children() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let character = seed(&pool).await;
    let id = character.id.as_uuid();
    assert!(sqlx::query("INSERT INTO character_skill_reuse VALUES($1,1,50)")
        .bind(id)
        .execute(&pool)
        .await
        .is_err());
    sqlx::query("INSERT INTO character_skill_reuse VALUES($1,1,51)")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query(EFFECT_INSERT)
        .bind(id)
        .bind(1_i64)
        .bind(100_i64)
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query(EFFECT_INSERT)
        .bind(id)
        .bind(5076_i64)
        .bind(101_i64)
        .execute(&pool)
        .await
        .is_err());
    sqlx::query(EFFECT_INSERT)
        .bind(id)
        .bind(1_i64)
        .bind(101_i64)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE characters SET phase3_timer_policy_version=1,karma=123 WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM character_skill_reuse WHERE character_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM character_effects WHERE character_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(sqlx::query(EFFECT_INSERT)
        .bind(id)
        .bind(1_i64)
        .bind(100_i64)
        .execute(&mut *tx)
        .await
        .is_err());
    tx.rollback().await.unwrap();
    let karma: i64 = sqlx::query_scalar("SELECT karma FROM characters WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(karma, 0);
    for table in ["character_skill_reuse", "character_effects"] {
        let sql =
            sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table} WHERE character_id=$1"));
        let count: i64 = sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
    // Creation now retains a kit marker; this fixture deletes it before testing child cascades.
    let removed =
        sqlx::query("DELETE FROM pending_item_requests WHERE character_id=$1 AND status='pending'")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected();
    assert_eq!(removed, 1);
    sqlx::query("DELETE FROM characters WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    for table in ["character_skill_reuse", "character_effects"] {
        let sql = sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"));
        let count: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }
}
