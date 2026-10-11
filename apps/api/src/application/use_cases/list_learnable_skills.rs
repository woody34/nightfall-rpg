//! Read next learning ranks from generation-fenced actor state, never DB progression.

use crate::application::ports::{LearnableSkillsState, Phase3Runtime};
use crate::application::{parse_id, AppError, CharacterRepository};
use crate::domain::{AccountId, CharacterId};
use std::sync::Arc;

/// Owned live-character learning query. Gameplay eligibility stays in the actor.
pub struct ListLearnableSkills {
    characters: Arc<dyn CharacterRepository>,
    runtime: Option<Arc<dyn Phase3Runtime>>,
}
impl ListLearnableSkills {
    /// None supports an offline-only server without inventing live eligibility.
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        runtime: Option<Arc<dyn Phase3Runtime>>,
    ) -> Self {
        Self {
            characters,
            runtime,
        }
    }
    /// Checks ownership, captures generation, then asks the actor to revalidate it.
    pub async fn execute(
        &self,
        caller: AccountId,
        character_id: &str,
    ) -> Result<LearnableSkillsState, AppError> {
        let id: CharacterId = parse_id("character_id", character_id)?;
        let character = self
            .characters
            .get(id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: character_id.to_owned(),
            })?;
        if character.account_id != caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            AppError::FailedPrecondition("character has no live zone session".into())
        })?;
        let generation = runtime.generation(caller, id).await?;
        runtime.learnable_skills(caller, id, generation).await
    }
}
