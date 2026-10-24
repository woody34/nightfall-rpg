//! Out-of-world inventory availability policy (D15). Live inventory uses `InventorySync`.
use crate::application::AppError;

/// Checks required ports before refusing admitted characters. This guard must precede
/// retained inventory reads, so a live character can never receive stale database rows.
pub fn require_out_of_world(
    zone_available: bool,
    catalogue_available: bool,
    admitted: bool,
) -> Result<(), AppError> {
    if !zone_available || !catalogue_available {
        return Err(AppError::Unavailable("item runtime or catalogue is unavailable".into()));
    }
    if admitted {
        return Err(AppError::FailedPrecondition(
            "admitted characters receive InventorySync over the world channel".into(),
        ));
    }
    Ok(())
}

use crate::application::ports::{ItemCataloguePort, ItemInventoryRepository, ItemInventoryRuntime};
use crate::application::CharacterRepository;
use crate::domain::{zone::Phase4Checkpoint, AccountId, CharacterId};
use std::sync::Arc;

/// Detached baseline: the zero tick/life is explicit and cannot replace a live WS baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct GetInventoryOutput {
    pub character_id: CharacterId,
    pub character_revision: u64,
    pub checkpoint: Phase4Checkpoint,
    pub ordinary_capacity: u32,
    pub quest_capacity: u32,
}

/// Owner-authenticated retained read, never speculative live database state.
pub struct GetInventory {
    characters: Arc<dyn CharacterRepository>,
    inventory: Option<Arc<dyn ItemInventoryRepository>>,
    runtime: Option<Arc<dyn ItemInventoryRuntime>>,
    catalogue: Option<Arc<dyn ItemCataloguePort>>,
}
impl GetInventory {
    /// Missing runtime/catalogue/repository ports keep reads fail closed.
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        inventory: Option<Arc<dyn ItemInventoryRepository>>,
        runtime: Option<Arc<dyn ItemInventoryRuntime>>,
        catalogue: Option<Arc<dyn ItemCataloguePort>>,
    ) -> Self {
        Self {
            characters,
            inventory,
            runtime,
            catalogue,
        }
    }
    /// Validate before ports; ownership before any retained state is read.
    pub async fn execute(
        &self,
        caller: AccountId,
        raw_id: &[u8],
    ) -> Result<GetInventoryOutput, AppError> {
        let uuid = uuid::Uuid::from_slice(raw_id).map_err(|_| {
            AppError::InvalidArgument("character_id must be exactly 16 UUID bytes".into())
        })?;
        let id = CharacterId::from_uuid(uuid);
        let missing = || AppError::NotFound {
            entity: "character",
            id: uuid.to_string(),
        };
        let character = self.characters.get(id).await?.ok_or_else(missing)?;
        if character.account_id != caller {
            return Err(AppError::PermissionDenied("character belongs to another account".into()));
        }
        require_out_of_world(self.runtime.is_some(), self.catalogue.is_some(), false)?;
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("inventory zone is unavailable".into()))?;
        let inventory = self
            .inventory
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("retained inventory is unavailable".into()))?;
        let catalogue = self
            .catalogue
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("item catalogue is unavailable".into()))?
            .catalogue()?;
        let retained = runtime
            .detached_inventory(id, inventory.as_ref())
            .await?
            .ok_or_else(missing)?;
        if retained.character_id != id || i64::try_from(retained.character_revision).is_err() {
            return Err(
                anyhow::anyhow!("retained inventory identity or revision is invalid").into()
            );
        }
        let checkpoint = retained.checkpoint.ok_or_else(|| {
            AppError::FailedPrecondition("inventory awaits first V6 materialization".into())
        })?;
        if checkpoint.starter_kit_receipt.is_none() {
            return Err(AppError::FailedPrecondition(
                "inventory awaits first V6 materialization".into(),
            ));
        }
        if checkpoint.rules_hash != catalogue.hash() {
            return Err(AppError::Unavailable(
                "retained inventory catalogue is unavailable".into(),
            ));
        }
        Ok(GetInventoryOutput {
            character_id: id,
            character_revision: retained.character_revision,
            ordinary_capacity: checkpoint.ordinary_capacity,
            quest_capacity: checkpoint.quest_capacity,
            checkpoint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d15_refuses_live_reads_and_missing_ports() {
        assert!(require_out_of_world(true, true, false).is_ok());
        assert!(matches!(
            require_out_of_world(true, true, true),
            Err(AppError::FailedPrecondition(_))
        ));
        for (zone, catalogue) in [(false, false), (true, false), (false, true)] {
            for admitted in [false, true] {
                assert!(matches!(
                    require_out_of_world(zone, catalogue, admitted),
                    Err(AppError::Unavailable(_))
                ));
            }
        }
    }
}
