//! Typed paused state, fencing, immutable retries and atomic child replacement in both adapters.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;

use nightfall_api::application::{
    CharacterCheckpoint, CharacterRepository, CheckpointError, CheckpointOutcome, IdempotencyKey,
};
use nightfall_api::domain::zone::effects::{
    compile_periodic, continuous_handler, ActiveEffect, ActiveHandler, ActivePeriodic, EffectFlags,
    EffectId, EffectPool, EffectSource, EffectSourceKind, PulseSchedule, RetainedEffect,
    RetainedModifierSource,
};
use nightfall_api::domain::zone::skill::{RankValue, SkillId, SkillRef};
use nightfall_api::domain::zone::{
    DeathRecord, Phase3Checkpoint, Phase3Rules, ResourceFractions, Tick,
};
use nightfall_api::domain::{
    AccountId, Character, CharacterName, DomainEvent, EventMetadata, Position, Race,
};
use nightfall_api::infrastructure::{
    memory::InMemoryCharacterRepository,
    postgres::PgCharacterRepository,
    skill_data::{load_phase3_rules, Phase3DataSource},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

fn rules() -> Arc<Phase3Rules> {
    Arc::new(load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap())
}
fn retained(rules: &Phase3Rules, id: SkillId) -> RetainedEffect {
    let skill = rules.skills().get(id).unwrap();
    let abnormal = skill.abnormal.as_ref().unwrap();
    let flag = |value: &RankValue<bool>| *value.at(1, skill.max_level).unwrap();
    ActiveEffect {
        id: EffectId(10),
        skill: SkillRef {
            id: skill.id,
            rank: 1,
        },
        source: EffectSource {
            binding: None,
            kind: EffectSourceKind::System,
        },
        target_incarnation: 8,
        applied_at: Tick(0),
        expires_at: Some(Tick(101)),
        pool: if abnormal.is_debuff {
            EffectPool::Debuff
        } else {
            EffectPool::Buff
        },
        abnormal_type: abnormal.r#type.clone(),
        abnormal_level: abnormal.level[0],
        flags: EffectFlags {
            stay_after_death: flag(&skill.stay_after_death),
            stay_on_subclass_change: flag(&skill.stay_on_subclass_change),
            removed_on_damage: flag(&skill.removed_on_damage),
            removed_on_action_except_move: flag(&skill.removed_on_action_except_move),
            irreplaceable: flag(&skill.irreplaceable_buff),
        },
        handlers: skill
            .effects
            .iter()
            .enumerate()
            .filter(|(_, h)| continuous_handler(&h.name))
            .map(|(i, handler)| ActiveHandler {
                xml_ordinal: u32::try_from(i).unwrap(),
                periodic: compile_periodic(handler, 1, skill.max_level).unwrap().map(
                    |(resource, power, interval)| ActivePeriodic {
                        resource,
                        power,
                        schedule: PulseSchedule::new(Tick(0), interval).unwrap(),
                    },
                ),
            })
            .collect(),
    }
    .retain(Tick(0))
    .unwrap()
    .unwrap()
}
fn payload(rules: &Phase3Rules) -> Phase3Checkpoint {
    let mut payload = Phase3Checkpoint::new();
    payload.karma = 47;
    payload.pk_count = 8;
    payload.pvp_count = 9;
    payload.flag_remaining_ticks = 1;
    payload.last_death = Some(DeathRecord {
        incarnation: 7,
        pre_death_xp: 200,
        recoverable_death_xp_loss: 30,
    });
    payload.resource_fractions = ResourceFractions {
        hp: 1,
        mp: 999_999,
        cp: 123,
    };
    payload.reuse_remainders.insert(SkillId(3), 51);
    payload
        .effect_remainders
        .push(retained(rules, SkillId(110)));
    let periodic = rules
        .skills()
        .skills()
        .values()
        .find(|skill| {
            skill.runtime_status == nightfall_api::domain::class::RuntimeSkillStatus::Executable
                && skill.abnormal.is_some()
                && skill
                    .effects
                    .iter()
                    .any(|handler| matches!(handler.name.as_str(), "TickHp" | "TickMp"))
        })
        .unwrap();
    payload.effect_remainders.push(retained(rules, periodic.id));
    payload
        .modifier_install_order
        .push(RetainedModifierSource::Effect { effect_ordinal: 0 });
    payload
        .modifier_install_order
        .push(RetainedModifierSource::Effect { effect_ordinal: 1 });
    payload
}
async fn seed(repo: &dyn CharacterRepository) -> Character {
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("PausedState").unwrap(),
        Race::Human,
    );
    repo.create_idempotent(&IdempotencyKey::new(), "phase3-checkpoint-seed", &c)
        .await
        .unwrap();
    c
}
fn checkpoint(c: &Character, phase3: Phase3Checkpoint) -> CharacterCheckpoint {
    CharacterCheckpoint {
        phase4: None,
        character_id: c.id,
        revision_seen: 0,
        level: c.level,
        xp: c.xp,
        hp: 20,
        mp: 10,
        alive: true,
        position: Position { x: 126.0, y: 128.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(c.class_state.clone()),
        phase3: Some(phase3),
    }
}
fn event(c: &Character) -> DomainEvent {
    DomainEvent::CharacterLeveled {
        metadata: EventMetadata::default(),
        character_id: c.id,
        level: c.level,
    }
}
async fn parity(repo: &dyn CharacterRepository, rules: &Phase3Rules) {
    let c = seed(repo).await;
    let legacy = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(legacy.phase3, None);
    let cp = checkpoint(&c, payload(rules));
    cp.phase3
        .as_ref()
        .unwrap()
        .validate(rules, &c.class_state.learned_skills, true)
        .unwrap();
    let events = [event(&c)];
    assert_eq!(repo.checkpoint(&cp, &events).await.unwrap(), CheckpointOutcome::Applied(1));
    let first = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(first.phase3, cp.phase3);
    assert_eq!(first.class_state, cp.class_state.clone().unwrap());
    assert_eq!(repo.get(c.id).await.unwrap().unwrap().class_state, first.class_state);
    assert_eq!(repo.list_by_account(c.account_id).await.unwrap().len(), 1);
    // The same immutable request survives a lost acknowledgement, regardless of elapsed time.
    assert_eq!(repo.checkpoint(&cp, &events).await.unwrap(), CheckpointOutcome::Replayed(1));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), first);
    let mut stale = cp.clone();
    stale.idempotency.1 = IdempotencyKey::new();
    stale.phase3.as_mut().unwrap().karma = 500;
    assert_eq!(repo.checkpoint(&stale, &events).await.unwrap(), CheckpointOutcome::Stale);
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), first);
    let mut conflict = cp.clone();
    conflict.phase3.as_mut().unwrap().resource_fractions.hp = 2;
    assert!(matches!(
        repo.checkpoint(&conflict, &events).await,
        Err(CheckpointError::KeyReused)
    ));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), first);
    // Paused values rebase only to the supplied admission tick; neither elapsed time nor hooks participate.
    let saved = first.phase3.as_ref().unwrap();
    for admission in [Tick(100), Tick(1_000_000)] {
        let restored = saved
            .stage_restore(rules, &first.class_state.learned_skills, true, 12, admission, 20, 30)
            .unwrap();
        assert_eq!(restored.flag_until, Some(Tick(admission.0.checked_add(1).unwrap())));
        assert_eq!(restored.reuse[&SkillId(3)], Tick(admission.0.checked_add(51).unwrap()));
        for (ordinal, effect) in saved.effect_remainders.iter().enumerate() {
            let id = EffectId(20_u64.checked_add(u64::try_from(ordinal).unwrap()).unwrap());
            let live = &restored.effects.entries()[&id];
            assert_eq!(live.target_incarnation, 12);
            assert_eq!(
                live.expires_at,
                Some(Tick(admission.0.checked_add(effect.remaining_ticks).unwrap()))
            );
            assert_eq!(live.retain(admission).unwrap().unwrap().handlers, effect.handlers);
        }
    }
    // Invalid retained payloads never consume a revision or alter previously stored children.
    for mutation in 0..5 {
        let mut invalid = cp.clone();
        invalid.revision_seen = 1;
        invalid.idempotency.1 = IdempotencyKey::new();
        let p = invalid.phase3.as_mut().unwrap();
        match mutation {
            0 => {
                p.reuse_remainders.insert(SkillId(3), 50);
            },
            1 => p.effect_remainders[0].remaining_ticks = 100,
            2 => p.timer_policy_version = 2,
            3 => p.resource_fractions.cp = 1_000_000,
            _ => p.modifier_install_order.clear(),
        }
        assert!(matches!(
            repo.checkpoint(&invalid, &[]).await,
            Err(CheckpointError::Constraint(_))
        ));
        assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), first);
    }
    // A successful new revision replaces both child collections, preserving only the supplied subset.
    let mut replacement = cp.clone();
    replacement.revision_seen = 1;
    replacement.idempotency.1 = IdempotencyKey::new();
    let p = replacement.phase3.as_mut().unwrap();
    p.reuse_remainders.clear();
    p.effect_remainders.clear();
    p.modifier_install_order.clear();
    p.flag_remaining_ticks = 0;
    assert_eq!(repo.checkpoint(&replacement, &[]).await.unwrap(), CheckpointOutcome::Applied(2));
    let second = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(second.phase3, replacement.phase3);
    let mut downgrade = replacement;
    downgrade.revision_seen = 2;
    downgrade.idempotency.1 = IdempotencyKey::new();
    downgrade.phase3 = None;
    assert!(matches!(
        repo.checkpoint(&downgrade, &[]).await,
        Err(CheckpointError::Constraint(_))
    ));
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), second);
    let mut dead = cp;
    dead.revision_seen = 2;
    dead.idempotency.1 = IdempotencyKey::new();
    dead.hp = 0;
    dead.alive = false;
    dead.class_state.as_mut().unwrap().sp = 90;
    let p = dead.phase3.as_mut().unwrap();
    p.resource_fractions = ResourceFractions::default();
    p.effect_remainders.clear();
    p.modifier_install_order = vec![RetainedModifierSource::DeathPenalty];
    p.death_penalty_level = 1;
    assert_eq!(repo.checkpoint(&dead, &[]).await.unwrap(), CheckpointOutcome::Applied(3));
    let offline = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert!(!offline.alive);
    assert_eq!(offline.hp, Some(0));
    assert_eq!(offline.class_state.sp, 90);
    assert_eq!(offline.phase3, dead.phase3);
}

#[tokio::test]
async fn memory_paused_checkpoint_fence_retry_and_replacement() {
    let rules = rules();
    parity(&InMemoryCharacterRepository::default().with_phase3_rules(rules.clone()), &rules).await;
}
#[tokio::test]
async fn postgres_paused_checkpoint_fence_retry_and_replacement() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let rules = rules();
    parity(&PgCharacterRepository::new(pool).with_phase3_rules(rules.clone()), &rules).await;
}

#[tokio::test]
async fn postgres_child_failure_rolls_back_revision_receipt_outbox_and_other_children() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let rules = rules();
    let repo = PgCharacterRepository::new(pool.clone()).with_phase3_rules(rules.clone());
    let c = seed(&repo).await;
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let cp = checkpoint(&c, payload(&rules));
    let outbox_before: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION refuse_effect() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected effect insert failure'; END $$; CREATE TRIGGER refuse_effect BEFORE INSERT ON character_effects FOR EACH ROW EXECUTE FUNCTION refuse_effect()").execute(&pool).await.unwrap();
    assert!(repo.checkpoint(&cp, &[event(&c)]).await.is_err());
    assert_eq!(repo.load_for_admission(c.id).await.unwrap().unwrap(), before);
    for table in ["character_skill_reuse", "character_effects"] {
        let sql = sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM idempotency_keys WHERE operation='save_checkpoint'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox")
            .fetch_one(&pool)
            .await
            .unwrap(),
        outbox_before
    );
    sqlx::raw_sql("DROP TRIGGER refuse_effect ON character_effects; DROP FUNCTION refuse_effect()")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(repo.checkpoint(&cp, &[event(&c)]).await.unwrap(), CheckpointOutcome::Applied(1));
}

#[tokio::test]
async fn postgres_unknown_retained_children_and_legacy_markers_fail_closed() {
    let Some(pool) = common::pg::migrated_pool().await else {
        return;
    };
    let rules = rules();
    let repo = PgCharacterRepository::new(pool.clone()).with_phase3_rules(rules.clone());
    let c = seed(&repo).await;
    sqlx::query("UPDATE characters SET karma=1 WHERE id=$1")
        .bind(c.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.get(c.id).await.is_err());
    assert!(repo.load_for_admission(c.id).await.is_err());
    sqlx::query("UPDATE characters SET karma=0 WHERE id=$1")
        .bind(c.id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let cp = checkpoint(&c, payload(&rules));
    repo.checkpoint(&cp, &[]).await.unwrap();
    sqlx::query("UPDATE character_effects SET handlers='[{\"xml_ordinal\":0,\"periodic\":null,\"extension\":1}]' WHERE character_id=$1").bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    assert!(repo.get(c.id).await.is_err());
    assert!(repo.list_by_account(c.account_id).await.is_err());
    assert!(repo.load_for_admission(c.id).await.is_err());
}

#[test]
fn every_retained_field_participates_in_the_checkpoint_fingerprint_and_none_preserves_legacy() {
    let rules = rules();
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("Fingerprint").unwrap(),
        Race::Human,
    );
    let cp = checkpoint(&c, payload(&rules));
    let fingerprint = cp.fingerprint(&[]);
    for change in 0..12 {
        let mut other = cp.clone();
        let p = other.phase3.as_mut().unwrap();
        match change {
            0 => p.karma = 48,
            1 => p.pk_count = 10,
            2 => p.pvp_count = 10,
            3 => p.flag_remaining_ticks = 2,
            4 => p.death_penalty_level = 1,
            5 => p.last_death.as_mut().unwrap().pre_death_xp = 201,
            6 => p.resource_fractions.mp = 0,
            7 => {
                p.reuse_remainders.insert(SkillId(3), 52);
            },
            8 => p.effect_remainders[0].target_incarnation = 9,
            9 => {
                p.effect_remainders[0].flags.irreplaceable =
                    !p.effect_remainders[0].flags.irreplaceable;
            },
            10 => p.effect_remainders[0].remaining_ticks = 102,
            _ => p.modifier_install_order.clear(),
        }
        assert_ne!(fingerprint, other.fingerprint(&[]));
    }
    let mut legacy = cp;
    legacy.phase3 = None;
    let body = serde_json::json!({"character_id":legacy.character_id.as_uuid(),"revision_seen":legacy.revision_seen,"level":legacy.level,"xp":legacy.xp,"hp":legacy.hp,"mp":legacy.mp,"alive":legacy.alive,"position":[legacy.position.x,legacy.position.y],"events":[],"class_state":legacy.class_state});
    assert_eq!(
        legacy.fingerprint(&[]),
        format!("{:x}", Sha256::digest(body.to_string().as_bytes()))
    );
}

#[tokio::test]
async fn migration_preserves_existing_rows_and_refuses_lossy_rollback() {
    use nightfall_api::infrastructure::postgres::{connection_from_pool, Migrator};
    use sea_orm_migration::MigratorTrait;
    let Some(pool) = common::pg::empty_schema_pool().await else {
        return;
    };
    let db = connection_from_pool(&pool);
    Migrator::up(&db, Some(8)).await.unwrap();
    let c = Character::create(
        AccountId::from_uuid(Uuid::nil()),
        CharacterName::new("PausedState").unwrap(),
        Race::Human,
    );
    // Version8 fixture: current creation requires the version10 pending-request table.
    sqlx::query("INSERT INTO characters(id,account_id,name,name_normalized,race,str,dex,con,\"int\",wit,men,class_profile,base_class_id,current_class_id) VALUES($1,$2,'PausedState','pausedstate','human',1,1,1,1,1,1,'human_fighter',0,0)")
        .bind(c.id.as_uuid()).bind(c.account_id.as_uuid()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO character_effects(character_id,retained_ordinal,skill_id,level,remaining_ticks,caster_kind,pool,abnormal_type,abnormal_level,stay_after_death,stay_on_subclass_change,handlers) VALUES($1,0,110,1,101,'system','buff','NONE',0,true,true,'[]')").bind(c.id.as_uuid()).execute(&pool).await.unwrap();
    Migrator::up(&db, Some(1)).await.unwrap();
    let row: (i64, bool, bool, bool, bool, bool) = sqlx::query_as("SELECT target_incarnation,removed_on_damage,removed_on_action_except_move,irreplaceable,stay_after_death,stay_on_subclass_change FROM character_effects WHERE character_id=$1").bind(c.id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(row, (0, false, false, false, true, true));
    assert!(Migrator::down(&db, Some(1))
        .await
        .unwrap_err()
        .to_string()
        .contains("losing retained state"));
    let remaining: i64 =
        sqlx::query_scalar("SELECT remaining_ticks FROM character_effects WHERE character_id=$1")
            .bind(c.id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 101);
    Migrator::up(&db, None).await.unwrap();
}
