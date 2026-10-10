//! Ownership-checked read of actor-owned transfer eligibility.

use std::sync::Arc;

use crate::application::ports::{ClassTransferRuntime, TransferOptionsState};
use crate::application::{parse_id, AppError, CharacterRepository};
use crate::domain::AccountId;

/// Reads the current live eligibility, including range, life and transfer-token requirements.
pub struct TransferOptions {
    characters: Arc<dyn CharacterRepository>,
    runtime: Option<Arc<dyn ClassTransferRuntime>>,
}

impl TransferOptions {
    /// Binds persistence ownership reads and actor routing.
    #[must_use]
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        runtime: Option<Arc<dyn ClassTransferRuntime>>,
    ) -> Self {
        Self {
            characters,
            runtime,
        }
    }

    /// Checks ownership before touching a live session; the actor decides mutable eligibility.
    pub async fn execute(
        &self,
        caller: AccountId,
        character_id: &str,
    ) -> Result<TransferOptionsState, AppError> {
        let id = parse_id("character_id", character_id)?;
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
        runtime.transfer_options(caller, id).await
    }
}
