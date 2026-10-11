//! Paid learning validates input, replays the durable receipt, then routes to the actor.

use crate::application::ports::{
    learn_skill_fingerprint, FrozenLearnResult, LearnReceiptLookup, LearnReceiptRepository,
    Phase3Runtime, SkillCataloguePort,
};
use crate::application::{parse_id, AppError, CharacterRepository, IdempotencyKey};
use crate::domain::zone::skill::{SkillId, SkillRef};
use crate::domain::{AccountId, CharacterId};
use std::sync::Arc;

/// Authenticated exact-rank mutation request; caller comes only from verified extensions.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct LearnSkillInput {
    pub caller: AccountId,
    pub character_id: String,
    pub skill_id: u32,
    pub skill_level: u32,
    pub idempotency_key: String,
}

/// Reads receipts and ownership; the runtime alone mutates live state and commits successes.
pub struct LearnSkill {
    characters: Arc<dyn CharacterRepository>,
    receipts: Arc<dyn LearnReceiptRepository>,
    catalogue: Arc<dyn SkillCataloguePort>,
    runtime: Option<Arc<dyn Phase3Runtime>>,
}

impl LearnSkill {
    /// Receipt persistence is independent of transfer history and always required.
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        receipts: Arc<dyn LearnReceiptRepository>,
        catalogue: Arc<dyn SkillCataloguePort>,
        runtime: Option<Arc<dyn Phase3Runtime>>,
    ) -> Self {
        Self {
            characters,
            receipts,
            catalogue,
            runtime,
        }
    }

    /// Frozen receipt precedes ownership/progression reads, registry, session and gameplay
    /// guards. A conflict is account/operation-wide even for another requested character.
    pub async fn execute(&self, input: LearnSkillInput) -> Result<FrozenLearnResult, AppError> {
        let character: CharacterId = parse_id("character_id", &input.character_id)?;
        let key = IdempotencyKey::parse(&input.idempotency_key)
            .map_err(|e| AppError::InvalidArgument(e.to_string()))?;
        if input.skill_id == 0 || input.skill_level == 0 {
            return Err(AppError::InvalidArgument(
                "skill_id and skill_level must be positive".into(),
            ));
        }
        let skill = SkillRef {
            id: SkillId(input.skill_id),
            rank: input.skill_level,
        };
        let fingerprint = learn_skill_fingerprint(character, skill);
        match self
            .receipts
            .learn_receipt_lookup(input.caller, &key, &fingerprint)
            .await?
        {
            LearnReceiptLookup::Known(result) => return Ok(result),
            LearnReceiptLookup::Conflict => return Err(AppError::IdempotencyConflict),
            LearnReceiptLookup::Unknown => {},
        }
        let owner = self
            .characters
            .get(character)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: input.character_id,
            })?;
        if owner.account_id != input.caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        let catalogue = self.catalogue.catalogue()?;
        let entry = catalogue
            .skills
            .iter()
            .find(|entry| entry.skill_id == skill.id)
            .ok_or_else(|| AppError::InvalidArgument("unknown skill_id".into()))?;
        if skill.rank > entry.max_level {
            return Err(AppError::InvalidArgument("unknown skill_level".into()));
        }
        if !entry.effect_implemented {
            return Err(AppError::FailedPrecondition(format!(
                "skill runtime is deferred: {}",
                entry.deferred_reasons.join(", ")
            )));
        }
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            AppError::FailedPrecondition("character has no live zone session".into())
        })?;
        let generation = runtime.generation(input.caller, character).await?;
        let reply = runtime
            .enqueue_learn_skill(input.caller, character, generation, key, skill)
            .await?;
        reply
            .await
            .map_err(|_| AppError::Unavailable("zone stopped before learning reply".into()))?
    }
}
