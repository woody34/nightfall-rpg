//! Typed Phase 3 rows, read under the parent lock and replaced inside the fenced transaction.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use uuid::Uuid;

use super::entities::characters;
use crate::application::CheckpointError;
use crate::domain::zone::effects::{
    EffectFlags, EffectPool, EffectSource, EffectSourceKind, RetainedEffect, TargetBinding,
};
use crate::domain::zone::skill::{SkillId, SkillRef};
use crate::domain::zone::{DeathRecord, EntityId, Phase3Checkpoint, ResourceFractions};
use crate::domain::CharacterId;

pub(super) async fn load<C: ConnectionTrait>(
    db: &C,
    model: &characters::Model,
) -> anyhow::Result<Option<Phase3Checkpoint>> {
    let reuse = db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT skill_id,remaining_ticks FROM character_skill_reuse WHERE character_id=$1 ORDER BY skill_id",
        [model.id.into()])).await?;
    let effects = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT * FROM character_effects WHERE character_id=$1 ORDER BY retained_ordinal",
            [model.id.into()],
        ))
        .await?;
    let Some(version) = model.phase3_timer_policy_version else {
        anyhow::ensure!(
            model.karma == 0
                && model.pk_count == 0
                && model.pvp_count == 0
                && model.flag_remaining_ticks == 0
                && model.death_penalty_level == 0
                && model.recoverable_death_xp_loss == 0
                && model.death_incarnation.is_none()
                && model.pre_death_xp.is_none()
                && model.hp_fraction_q == 0
                && model.mp_fraction_q == 0
                && model.cp_fraction_q == 0
                && model.modifier_install_order == serde_json::json!([])
                && reuse.is_empty()
                && effects.is_empty(),
            "retained Phase3 data on a legacy row"
        );
        return Ok(None);
    };
    let last_death = match (model.death_incarnation, model.pre_death_xp) {
        (None, None) => {
            anyhow::ensure!(model.recoverable_death_xp_loss == 0, "loss without death record");
            None
        },
        (Some(incarnation), Some(xp)) => Some(DeathRecord {
            incarnation: u32::try_from(incarnation)?,
            pre_death_xp: u64::try_from(xp)?,
            recoverable_death_xp_loss: u64::try_from(model.recoverable_death_xp_loss)?,
        }),
        _ => anyhow::bail!("incomplete death record"),
    };
    let reuse_remainders: BTreeMap<SkillId, u64> = reuse
        .into_iter()
        .map(|row| {
            Ok((
                SkillId(u32::try_from(row.try_get::<i64>("", "skill_id")?)?),
                u64::try_from(row.try_get::<i64>("", "remaining_ticks")?)?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;
    let effect_remainders = effects
        .into_iter()
        .enumerate()
        .map(|(index, row)| decode_effect(index, &row))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Some(Phase3Checkpoint {
        timer_policy_version: u32::try_from(version)?,
        karma: u32::try_from(model.karma)?,
        pk_count: u32::try_from(model.pk_count)?,
        pvp_count: u32::try_from(model.pvp_count)?,
        flag_remaining_ticks: u64::try_from(model.flag_remaining_ticks)?,
        death_penalty_level: u8::try_from(model.death_penalty_level)?,
        last_death,
        resource_fractions: ResourceFractions {
            hp: model.hp_fraction_q,
            mp: model.mp_fraction_q,
            cp: model.cp_fraction_q,
        },
        reuse_remainders,
        effect_remainders,
        modifier_install_order: serde_json::from_value(model.modifier_install_order.clone())?,
    }))
}

fn signed(value: u64) -> Result<i64, CheckpointError> {
    i64::try_from(value)
        .map_err(|_| CheckpointError::Constraint("characters_phase3_integer_range".into()))
}

/// Parent revision CAS must already have succeeded. All replacements share that row lock.
pub(super) async fn persist(
    tx: &DatabaseTransaction,
    character: CharacterId,
    phase3: &Phase3Checkpoint,
) -> Result<(), CheckpointError> {
    let death_incarnation = phase3.last_death.as_ref().map(|r| i64::from(r.incarnation));
    let pre_death_xp = phase3
        .last_death
        .as_ref()
        .map(|r| signed(r.pre_death_xp))
        .transpose()?;
    let loss = phase3
        .last_death
        .as_ref()
        .map_or(Ok(0), |r| signed(r.recoverable_death_xp_loss))?;
    let order =
        serde_json::to_value(&phase3.modifier_install_order).map_err(anyhow::Error::from)?;
    tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "WITH clear_reuse AS (DELETE FROM character_skill_reuse WHERE character_id=$1), \
         clear_effects AS (DELETE FROM character_effects WHERE character_id=$1) \
         UPDATE characters SET phase3_timer_policy_version=$2,karma=$3,pk_count=$4,pvp_count=$5,flag_remaining_ticks=$6,death_penalty_level=$7,death_incarnation=$8,pre_death_xp=$9,recoverable_death_xp_loss=$10,hp_fraction_q=$11,mp_fraction_q=$12,cp_fraction_q=$13,modifier_install_order=$14 WHERE id=$1",
        [character.as_uuid().into(),i32::try_from(phase3.timer_policy_version).map_err(anyhow::Error::from)?.into(),i64::from(phase3.karma).into(),i64::from(phase3.pk_count).into(),i64::from(phase3.pvp_count).into(),signed(phase3.flag_remaining_ticks)?.into(),i16::from(phase3.death_penalty_level).into(),death_incarnation.into(),pre_death_xp.into(),loss.into(),phase3.resource_fractions.hp.into(),phase3.resource_fractions.mp.into(),phase3.resource_fractions.cp.into(),order.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
    for (skill, remaining) in &phase3.reuse_remainders {
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "INSERT INTO character_skill_reuse(character_id,skill_id,remaining_ticks) VALUES($1,$2,$3)",
            [character.as_uuid().into(),i64::from(skill.0).into(),signed(*remaining)?.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
    }
    for (ordinal, effect) in phase3.effect_remainders.iter().enumerate() {
        let source_kind = match effect.source.kind {
            EffectSourceKind::Player => "player",
            EffectSourceKind::Npc => "npc",
            EffectSourceKind::System => "system",
        };
        let pool = match effect.pool {
            EffectPool::Buff => "buff",
            EffectPool::SongDance => "song_dance",
            EffectPool::Debuff => "debuff",
            EffectPool::System => {
                return Err(CheckpointError::Constraint("character_effects_pool".into()))
            },
        };
        let source_id = effect.source.binding.map(|b| b.entity.as_uuid());
        let source_incarnation = effect.source.binding.map(|b| i64::from(b.incarnation));
        let handlers = serde_json::to_value(&effect.handlers).map_err(anyhow::Error::from)?;
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "INSERT INTO character_effects(character_id,retained_ordinal,skill_id,level,remaining_ticks,caster_kind,caster_id,caster_incarnation,pool,abnormal_type,abnormal_level,stay_after_death,stay_on_subclass_change,handlers,target_incarnation,removed_on_damage,removed_on_action_except_move,irreplaceable) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
            [character.as_uuid().into(),i64::try_from(ordinal).map_err(anyhow::Error::from)?.into(),i64::from(effect.skill.id.0).into(),i64::from(effect.skill.rank).into(),signed(effect.remaining_ticks)?.into(),source_kind.into(),source_id.into(),source_incarnation.into(),pool.into(),effect.abnormal_type.clone().into(),i64::from(effect.abnormal_level).into(),effect.flags.stay_after_death.into(),effect.flags.stay_on_subclass_change.into(),handlers.into(),i64::from(effect.target_incarnation).into(),effect.flags.removed_on_damage.into(),effect.flags.removed_on_action_except_move.into(),effect.flags.irreplaceable.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
    }
    Ok(())
}

fn decode_effect(index: usize, row: &sea_orm::QueryResult) -> anyhow::Result<RetainedEffect> {
    anyhow::ensure!(
        u64::try_from(row.try_get::<i64>("", "retained_ordinal")?)? == u64::try_from(index)?,
        "sparse retained effect ordinals"
    );
    let kind = match row.try_get::<String>("", "caster_kind")?.as_str() {
        "player" => EffectSourceKind::Player,
        "npc" => EffectSourceKind::Npc,
        "system" => EffectSourceKind::System,
        _ => anyhow::bail!("invalid source kind"),
    };
    let binding = match (
        row.try_get::<Option<Uuid>>("", "caster_id")?,
        row.try_get::<Option<i64>>("", "caster_incarnation")?,
    ) {
        (Some(id), Some(incarnation)) => Some(TargetBinding {
            entity: EntityId::from_uuid(id),
            incarnation: u32::try_from(incarnation)?,
        }),
        (None, None) => None,
        _ => anyhow::bail!("incomplete source binding"),
    };
    let pool = match row.try_get::<String>("", "pool")?.as_str() {
        "buff" => EffectPool::Buff,
        "song_dance" => EffectPool::SongDance,
        "debuff" => EffectPool::Debuff,
        _ => anyhow::bail!("invalid retained pool"),
    };
    Ok(RetainedEffect {
        skill: SkillRef {
            id: SkillId(u32::try_from(row.try_get::<i64>("", "skill_id")?)?),
            rank: u32::try_from(row.try_get::<i64>("", "level")?)?,
        },
        source: EffectSource { binding, kind },
        target_incarnation: u32::try_from(row.try_get::<i64>("", "target_incarnation")?)?,
        pool,
        abnormal_type: row.try_get("", "abnormal_type")?,
        abnormal_level: u32::try_from(row.try_get::<i64>("", "abnormal_level")?)?,
        remaining_ticks: u64::try_from(row.try_get::<i64>("", "remaining_ticks")?)?,
        flags: EffectFlags {
            stay_after_death: row.try_get("", "stay_after_death")?,
            stay_on_subclass_change: row.try_get("", "stay_on_subclass_change")?,
            removed_on_damage: row.try_get("", "removed_on_damage")?,
            removed_on_action_except_move: row.try_get("", "removed_on_action_except_move")?,
            irreplaceable: row.try_get("", "irreplaceable")?,
        },
        handlers: serde_json::from_value(row.try_get::<serde_json::Value>("", "handlers")?)?,
    })
}
