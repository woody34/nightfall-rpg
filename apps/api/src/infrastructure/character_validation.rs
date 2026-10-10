//! Shared repository validation against the immutable startup catalogue.

use super::class_data::{load_classes, ClassDataError, ClassSource};
use crate::domain::class::ClassRegistry;
use std::sync::{Arc, OnceLock};

static REGISTRY: OnceLock<Result<Arc<ClassRegistry>, ClassDataError>> = OnceLock::new();

/// Tests/default repositories resolve embedded data once; production supplies the same registry
/// used by the zone. Failure remains an error, never an invented fallback catalogue.
pub(super) fn registry(
    override_registry: Option<&Arc<ClassRegistry>>,
) -> anyhow::Result<Arc<ClassRegistry>> {
    if let Some(registry) = override_registry {
        return Ok(registry.clone());
    }
    match REGISTRY
        .get_or_init(|| load_classes(&ClassSource::embedded()).map(|classes| classes.registry))
    {
        Ok(registry) => Ok(registry.clone()),
        Err(error) => Err(error.clone().into()),
    }
}
