//! Immutable item catalogue read. Authentication belongs to the transport boundary.
use crate::application::{ports::ItemCataloguePort, AppError};
use crate::domain::zone::{Phase3Rules, Phase4Rules};
use std::sync::Arc;

/// Epoch-validated immutable catalogue adapter, constructed before services are bound.
pub struct RegistryItemCatalogue(Arc<Phase4Rules>);
impl RegistryItemCatalogue {
    /// The actual V6 skill/profile content must agree with every admitted item skill.
    pub fn new(rules: Arc<Phase4Rules>, skills: &Phase3Rules) -> Result<Self, AppError> {
        rules.validate_phase3(skills).map_err(|error| {
            AppError::Unavailable(format!("item catalogue epoch is unsupported: {error}"))
        })?;
        Ok(Self(rules))
    }
}
impl ItemCataloguePort for RegistryItemCatalogue {
    fn catalogue(&self) -> Result<Arc<Phase4Rules>, AppError> {
        Ok(self.0.clone())
    }
}

/// Naturally idempotent catalogue read. None deliberately keeps rollout fail closed.
pub struct GetItemCatalog {
    catalogue: Option<Arc<dyn ItemCataloguePort>>,
}
impl GetItemCatalog {
    /// Inject a prevalidated registry; no read can manufacture unavailable metadata.
    pub fn new(catalogue: Option<Arc<dyn ItemCataloguePort>>) -> Self {
        Self { catalogue }
    }
    /// The known-hash hint never turns a complete catalogue into an ambiguous empty list.
    pub fn execute(&self) -> Result<Arc<Phase4Rules>, AppError> {
        self.catalogue
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("item catalogue is not available".into()))?
            .catalogue()
    }
}
