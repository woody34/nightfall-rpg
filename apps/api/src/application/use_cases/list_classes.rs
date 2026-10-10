//! Read the immutable, validated class catalogue. Authentication is enforced by the interface.

use std::sync::Arc;

use crate::domain::class::ClassRegistry;

/// Static catalogue read response, mapped to transport types at the interface.
#[derive(Debug, Clone)]
pub struct ClassCatalogue {
    /// Exact validated registry shared with creation and the zone.
    pub registry: Arc<ClassRegistry>,
    /// Canonical hash including growth and metadata.
    pub data_version: String,
}

/// A naturally idempotent catalogue read.
pub struct ListClasses {
    catalogue: ClassCatalogue,
}

impl ListClasses {
    /// Shares the startup catalogue without consulting files on requests.
    #[must_use]
    pub fn new(registry: Arc<ClassRegistry>, data_version: String) -> Self {
        Self {
            catalogue: ClassCatalogue {
                registry,
                data_version,
            },
        }
    }

    /// Returns the catalogue used by this server epoch.
    #[must_use]
    pub fn execute(&self) -> ClassCatalogue {
        self.catalogue.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_returns_the_validated_startup_data_without_writes() {
        let classes = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap();
        let expected = classes.registry.clone();
        let hash = classes.config_hash.clone();
        let response = ListClasses::new(classes.registry, classes.config_hash).execute();
        assert!(Arc::ptr_eq(&response.registry, &expected));
        assert_eq!(response.data_version, hash);
        assert_eq!(response.registry.classes().len(), 89);
        assert_eq!(response.registry.races().len(), 5);
    }
}
