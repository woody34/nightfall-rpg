//! Immutable skill catalogue backed by Z's independently validated registry.

use crate::application::ports::{SkillCatalogue, SkillCatalogueEntry, SkillCataloguePort};
use crate::application::AppError;
use crate::domain::class::{ClassRegistry, RuntimeSkillStatus};
use crate::domain::zone::skill::{SkillId, SkillRegistry};
use std::fmt::Write as _;
use std::sync::Arc;

/// Validates catalogue roots against the full registry once at startup. Referenced/system
/// definitions remain in the registry but do not become player catalogue entries.
pub struct RegistrySkillCatalogue(SkillCatalogue);

impl RegistrySkillCatalogue {
    /// Both registries must come from their strict loaders, using the same data revision.
    pub fn new(classes: &ClassRegistry, skills: &SkillRegistry) -> anyhow::Result<Self> {
        let entries = classes
            .known_skills()
            .iter()
            .map(|root| {
                let def = skills.get(SkillId(root.id)).ok_or_else(|| {
                    anyhow::anyhow!("catalogue skill {} missing from validated registry", root.id)
                })?;
                anyhow::ensure!(
                    root.key == def.key
                        && root.max_level == def.max_level
                        && root.runtime_status == def.runtime_status
                        && root.l2_ref == def.l2_ref,
                    "catalogue skill {} disagrees with validated registry",
                    root.id
                );
                Ok(SkillCatalogueEntry {
                    skill_id: def.id,
                    key: def.key.clone(),
                    display_name: def.l2_ref.clone(),
                    max_level: def.max_level,
                    effect_implemented: def.runtime_status == RuntimeSkillStatus::Executable,
                    deferred_reasons: def.blockers.clone(),
                    l2_ref: def.l2_ref.clone(),
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut data_version = String::new();
        for byte in skills.hash() {
            write!(data_version, "{byte:02x}")?;
        }
        Ok(Self(SkillCatalogue {
            data_version,
            skills: entries,
        }))
    }
}

impl SkillCataloguePort for RegistrySkillCatalogue {
    fn catalogue(&self) -> Result<SkillCatalogue, AppError> {
        Ok(self.0.clone())
    }
}

/// Authenticated naturally idempotent catalogue read. Authentication is enforced at the edge.
pub struct GetSkillCatalog {
    catalogue: Option<Arc<dyn SkillCataloguePort>>,
}
impl GetSkillCatalog {
    /// None keeps the rollout fail-closed.
    pub fn new(catalogue: Option<Arc<dyn SkillCataloguePort>>) -> Self {
        Self { catalogue }
    }
    /// Returns immutable catalogue metadata, including deferred blockers.
    pub fn execute(&self) -> Result<SkillCatalogue, AppError> {
        self.catalogue
            .as_ref()
            .ok_or_else(|| AppError::Unavailable("phase 3 catalogue is not yet available".into()))?
            .catalogue()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn validated_registry_catalogue_has_only_roots_and_honest_blockers() {
        let classes = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap();
        let skills = crate::infrastructure::skill_data::load_skills(
            &crate::infrastructure::skill_data::SkillSource::embedded(),
            &[],
        )
        .unwrap();
        let adapter = RegistrySkillCatalogue::new(&classes.registry, &skills).unwrap();
        let uc = GetSkillCatalog::new(Some(Arc::new(adapter)));
        let first = uc.execute().unwrap();
        assert_eq!(first, uc.execute().unwrap());
        assert_eq!(first.skills.len(), classes.registry.known_skills().len());
        assert_eq!(first.skills.iter().filter(|s| s.effect_implemented).count(), 199);
        assert_eq!(
            first
                .skills
                .iter()
                .filter(|s| !s.effect_implemented)
                .count(),
            179
        );
        assert_eq!(first.data_version.len(), 64);
        assert!(first.skills.windows(2).all(|pair| pair
            .first()
            .zip(pair.get(1))
            .is_some_and(|(a, b)| a.skill_id < b.skill_id)));
        for entry in &first.skills {
            assert_eq!(entry.effect_implemented, entry.deferred_reasons.is_empty());
        }
        assert!(first.skills.len() < skills.skills().len());
        assert!(matches!(GetSkillCatalog::new(None).execute(), Err(AppError::Unavailable(_))));
    }
}
