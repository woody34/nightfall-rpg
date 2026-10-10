//! Atomic typed class ledger/receipt projection beside the legacy progression columns.

use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};
use uuid::Uuid;

use super::entities::characters;
use super::idempotency::{self, operation, Claim};
use crate::application::ports::{transfer_fingerprint, MutationReceiptLookup};
use crate::application::{CharacterCheckpoint, CheckpointError, IdempotencyKey};
use crate::domain::character_progression::{
    CharacterAppearance, ClassState, FrozenTransferResult, SuccessfulTransferReceipt,
};
use crate::domain::class::ClassId;
use crate::domain::subclass::{LearnedSkill, Sex};
use crate::domain::{AccountId, Character, CharacterId};

pub(super) fn appearance(m: &characters::Model) -> anyhow::Result<CharacterAppearance> {
    Ok(CharacterAppearance {
        sex: match m.sex.as_str() {
            "male" => Sex::Male,
            "female" => Sex::Female,
            _ => anyhow::bail!("unknown character sex in database"),
        },
        hair_style: u32::try_from(m.hair_style)?,
        hair_color: u32::try_from(m.hair_color)?,
        face: u32::try_from(m.face)?,
    })
}

pub(super) fn state(m: &characters::Model) -> anyhow::Result<ClassState> {
    Ok(ClassState {
        base_class_id: ClassId(u32::try_from(m.base_class_id)?),
        current_class_id: ClassId(u32::try_from(m.current_class_id)?),
        sp: u64::try_from(m.sp)?,
        learned_skills: Vec::new(),
        cp: u32::try_from(m.cp)?,
        token_tier_1_count: u32::try_from(m.token_tier_1_count)?,
        token_tier_2_count: u32::try_from(m.token_tier_2_count)?,
        milestone_claimed_mask: u8::try_from(m.milestone_claimed_mask)?,
        successful_transfer_receipts: Vec::new(),
    })
}

pub(super) async fn receipts<C: ConnectionTrait>(
    db: &C,
    id: CharacterId,
) -> anyhow::Result<Vec<SuccessfulTransferReceipt>> {
    let rows = db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT r.key, r.target_class_id, i.response FROM character_transfer_receipts r \
         JOIN idempotency_keys i ON i.account_id = r.account_id AND i.operation = r.operation AND i.key = r.key \
         WHERE r.character_id = $1 ORDER BY r.ordinal", [id.as_uuid().into()])).await?;
    rows.into_iter()
        .map(|row| {
            let response: serde_json::Value = row.try_get("", "response")?;
            Ok(SuccessfulTransferReceipt {
                key: row.try_get("", "key")?,
                target_class_id: ClassId(u32::try_from(
                    row.try_get::<i32>("", "target_class_id")?,
                )?),
                result: serde_json::from_value(response)?,
            })
        })
        .collect()
}

pub(super) async fn learned_skills<C: ConnectionTrait>(
    db: &C,
    id: CharacterId,
) -> anyhow::Result<Vec<LearnedSkill>> {
    let rows = db.query_all_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT skill_key, skill_level FROM character_learned_skills WHERE character_id=$1 AND slot=0 ORDER BY skill_key",
        [id.as_uuid().into()])).await?;
    rows.into_iter()
        .map(|row| {
            Ok(LearnedSkill {
                key: row.try_get("", "skill_key")?,
                level: u32::try_from(row.try_get::<i32>("", "skill_level")?)?,
            })
        })
        .collect()
}

pub(super) async fn hydrate<C: ConnectionTrait>(
    db: &C,
    character: &mut Character,
) -> anyhow::Result<()> {
    character.class_state.successful_transfer_receipts = receipts(db, character.id).await?;
    character.class_state.learned_skills = learned_skills(db, character.id).await?;
    character.class_state.validate()?;
    Ok(())
}

async fn persist_skills(
    tx: &DatabaseTransaction,
    id: CharacterId,
    skills: &[LearnedSkill],
) -> Result<(), CheckpointError> {
    for known in learned_skills(tx, id).await? {
        if !skills
            .iter()
            .any(|skill| skill.key == known.key && skill.level >= known.level)
        {
            return Err(CheckpointError::Constraint("character_learned_skills_monotone".into()));
        }
    }
    for skill in skills {
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "INSERT INTO character_learned_skills(character_id,slot,skill_key,skill_level) VALUES($1,0,$2,$3) \
             ON CONFLICT(character_id,slot,skill_key) DO UPDATE SET skill_level=EXCLUDED.skill_level,updated_at=now()",
            [id.as_uuid().into(), skill.key.clone().into(), i32::try_from(skill.level).map_err(anyhow::Error::from)?.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
    }
    Ok(())
}

pub(super) async fn receipt_lookup<C: ConnectionTrait>(
    db: &C,
    caller: AccountId,
    key: &IdempotencyKey,
    fingerprint: &str,
) -> anyhow::Result<MutationReceiptLookup> {
    let row = db.query_one_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "SELECT fingerprint, response FROM idempotency_keys WHERE account_id = $1 AND operation = 'change_class' AND key = $2",
        [caller.as_uuid().into(), key.as_uuid().into()])).await?;
    match row {
        None => Ok(MutationReceiptLookup::Unknown),
        Some(row) => {
            let stored: String = row.try_get("", "fingerprint")?;
            if stored != fingerprint {
                return Ok(MutationReceiptLookup::Conflict);
            }
            let result: FrozenTransferResult =
                serde_json::from_value(row.try_get::<serde_json::Value>("", "response")?)?;
            Ok(MutationReceiptLookup::Known(result))
        },
    }
}

/// Runs after the character CAS UPDATE acquired its row lock, before outbox/commit.
pub(super) async fn persist(
    tx: &DatabaseTransaction,
    cp: &CharacterCheckpoint,
    original: &Character,
) -> Result<(), CheckpointError> {
    let known = receipts(tx, cp.character_id).await?;
    let Some(ledger) = &cp.class_state else {
        // Legacy checkpoints still keep normalized slot zero in step with the old columns.
        tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "UPDATE character_class_slots SET level = $2, exp = $3, updated_at = now() WHERE character_id = $1 AND slot = 0",
            [cp.character_id.as_uuid().into(), i32::try_from(cp.level).map_err(anyhow::Error::from)?.into(), i64::try_from(cp.xp).map_err(anyhow::Error::from)?.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
        return Ok(());
    };
    if ledger.base_class_id != original.class_state.base_class_id {
        return Err(CheckpointError::Constraint("characters_base_class_immutable".into()));
    }
    for receipt in &known {
        if ledger.receipt(receipt.key) != Some(receipt) {
            return Err(CheckpointError::Constraint(
                "character_transfer_receipts_immutable".into(),
            ));
        }
    }
    for (ordinal, receipt) in ledger.successful_transfer_receipts.iter().enumerate() {
        if receipt.result.character_id != cp.character_id
            || receipt.result.identity != original.identity()
            || receipt.result.name != original.name
            || receipt.result.stats != original.stats
        {
            return Err(CheckpointError::Constraint("character_transfer_receipts_identity".into()));
        }
        let fingerprint = transfer_fingerprint(cp.character_id, receipt.target_class_id);
        let response = serde_json::to_value(&receipt.result).map_err(anyhow::Error::from)?;
        let claim = idempotency::claim(
            tx,
            original.account_id,
            operation::CHANGE_CLASS,
            &IdempotencyKey::from_uuid(receipt.key),
            &fingerprint,
            response.clone(),
        )
        .await?;
        if let Claim::Existing {
            fingerprint: stored,
            response: stored_response,
        } = claim
        {
            if stored != fingerprint || stored_response != response {
                return Err(CheckpointError::KeyReused);
            }
        }
        let ordinal = i16::try_from(ordinal).map_err(anyhow::Error::from)?;
        // A receipt already saved at this ordinal is immutable, including account/key/target.
        let row = tx.query_one_raw(Statement::from_sql_and_values(DbBackend::Postgres,
            "SELECT account_id, key, target_class_id FROM character_transfer_receipts WHERE character_id = $1 AND ordinal = $2",
            [cp.character_id.as_uuid().into(), ordinal.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
        if let Some(row) = row {
            if row
                .try_get::<Uuid>("", "account_id")
                .map_err(anyhow::Error::from)?
                != original.account_id.as_uuid()
                || row
                    .try_get::<Uuid>("", "key")
                    .map_err(anyhow::Error::from)?
                    != receipt.key
                || row
                    .try_get::<i32>("", "target_class_id")
                    .map_err(anyhow::Error::from)?
                    != i32::try_from(receipt.target_class_id.0).map_err(anyhow::Error::from)?
            {
                return Err(CheckpointError::KeyReused);
            }
        } else {
            tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
                "INSERT INTO character_transfer_receipts (character_id, ordinal, account_id, key, target_class_id) VALUES ($1,$2,$3,$4,$5)",
                [cp.character_id.as_uuid().into(), ordinal.into(), original.account_id.as_uuid().into(), receipt.key.into(), i32::try_from(receipt.target_class_id.0).map_err(anyhow::Error::from)?.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
        }
    }
    persist_skills(tx, cp.character_id, &ledger.learned_skills).await?;
    tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "UPDATE character_class_slots SET class_id = $2, level = $3, exp = $4, sp = $5, updated_at = now() WHERE character_id = $1 AND slot = 0",
        [cp.character_id.as_uuid().into(), i32::try_from(ledger.current_class_id.0).map_err(anyhow::Error::from)?.into(), i32::try_from(cp.level).map_err(anyhow::Error::from)?.into(), i64::try_from(cp.xp).map_err(anyhow::Error::from)?.into(), i64::try_from(ledger.sp).map_err(anyhow::Error::from)?.into()])).await.map_err(super::character_repository::checkpoint_db_error)?;
    Ok(())
}

pub(super) async fn create_main_slot(
    tx: &DatabaseTransaction,
    c: &Character,
) -> anyhow::Result<()> {
    tx.execute_raw(Statement::from_sql_and_values(DbBackend::Postgres,
        "INSERT INTO character_class_slots (character_id,slot,class_id,level,exp,sp) VALUES ($1,0,$2,$3,$4,$5)",
        [c.id.as_uuid().into(), i32::try_from(c.class_state.current_class_id.0)?.into(), i32::try_from(c.level)?.into(), i64::try_from(c.xp)?.into(), i64::try_from(c.class_state.sp)?.into()])).await?;
    persist_skills(tx, c.id, &c.class_state.learned_skills).await?;
    Ok(())
}
