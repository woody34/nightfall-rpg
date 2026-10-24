//! Immutable skill catalogue backed by Z's independently validated registry.

use crate::application::ports::{
    SkillCatalogue, SkillCatalogueEntry, SkillCataloguePort, SkillOperateType,
};
use crate::application::AppError;
use crate::domain::class::{ClassRegistry, RuntimeSkillStatus};
use crate::domain::zone::skill::{RankValue, SkillId, SkillRegistry};
use std::fmt::Write as _;
use std::sync::Arc;

/// Validates catalogue roots against the full registry once at startup. Referenced/system
/// definitions remain in the registry but do not become player catalogue entries.
pub struct RegistrySkillCatalogue(SkillCatalogue);

impl RegistrySkillCatalogue {
    /// Both registries must come from their strict loaders, using the same data revision.
    pub fn new(classes: &ClassRegistry, skills: &SkillRegistry) -> anyhow::Result<Self> {
        Self::build(classes, skills, false)
    }

    /// V6 uses the canonical epoch rule validator for published capability promotions.
    pub fn for_rules(
        classes: &ClassRegistry,
        rules: &crate::domain::zone::Phase3Rules,
    ) -> anyhow::Result<Self> {
        rules
            .validate_catalogue(classes)
            .map_err(anyhow::Error::msg)?;
        Self::build(classes, rules.skills(), rules.is_v6())
    }

    fn build(
        classes: &ClassRegistry,
        skills: &SkillRegistry,
        phase4: bool,
    ) -> anyhow::Result<Self> {
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
                        && (root.runtime_status == def.runtime_status
                            || (phase4
                                && root.runtime_status == RuntimeSkillStatus::Deferred
                                && def.runtime_status == RuntimeSkillStatus::Executable))
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
                    operate_type: operate_type(&def.operate_type),
                    target_type: wire_strings(
                        def.id,
                        "target_type",
                        &def.target_type,
                        def.max_level,
                    )?,
                    affect_scope: wire_strings(
                        def.id,
                        "affect_scope",
                        &def.affect_scope,
                        def.max_level,
                    )?,
                    mp_consume_start: wire_costs(
                        def.id,
                        "mp_consume_start",
                        &def.mp_consume_start,
                    )?,
                    mp_consume_end: wire_costs(def.id, "mp_consume_end", &def.mp_consume_end)?,
                    hp_consume: wire_costs(def.id, "hp_consume", &def.hp_consume)?,
                    reuse_ms: wire_ranks(def.id, "reuse_ms", &def.reuse_ms, def.max_level)?,
                    hit_time_ms: wire_ranks(
                        def.id,
                        "hit_time_ms",
                        &def.hit_time_ms,
                        def.max_level,
                    )?,
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

// Pinned l2j-game @abfde049 model/skills/SkillOperateType.java:94-131.
// A3 is the isSelfContinuous branch; A4/channelled/directional types stay OTHER.
fn operate_type(source: &str) -> SkillOperateType {
    match source {
        "A1" => SkillOperateType::ActiveInstant,
        "A2" => SkillOperateType::ActiveInstantContinuous,
        "A3" => SkillOperateType::ActiveContinuous,
        "P" => SkillOperateType::Passive,
        "T" => SkillOperateType::Toggle,
        _ => SkillOperateType::Other,
    }
}

fn wire_strings(
    id: SkillId,
    field: &str,
    values: &RankValue<String>,
    max_level: u32,
) -> anyhow::Result<Vec<String>> {
    (1..=max_level)
        .map(|rank| {
            values.at(rank, max_level).cloned().ok_or_else(|| {
                anyhow::anyhow!("catalogue skill {} {field} missing rank {rank}", id.0)
            })
        })
        .collect()
}

fn wire_value(id: SkillId, field: &str, value: i64) -> anyhow::Result<u32> {
    u32::try_from(value)
        .map_err(|e| anyhow::anyhow!("catalogue skill {} {field} cannot fit uint32: {e}", id.0))
}

fn wire_costs(id: SkillId, field: &str, values: &[i64]) -> anyhow::Result<Vec<u32>> {
    values
        .iter()
        .map(|&value| wire_value(id, field, value))
        .collect()
}

fn wire_ranks(
    id: SkillId,
    field: &str,
    values: &RankValue<i64>,
    max_level: u32,
) -> anyhow::Result<Vec<u32>> {
    (1..=max_level)
        .map(|rank| {
            let value = values.at(rank, max_level).ok_or_else(|| {
                anyhow::anyhow!("catalogue skill {} {field} missing rank {rank}", id.0)
            })?;
            wire_value(id, field, *value)
        })
        .collect()
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
    fn rank_projection_checks_wire_bounds_and_preserves_rank_tables() -> anyhow::Result<()> {
        let id = SkillId(3);
        assert_eq!(wire_costs(id, "cost", &[0, i64::from(u32::MAX)])?, [0, u32::MAX]);
        assert!(wire_costs(id, "cost", &[-1]).is_err());
        assert!(wire_costs(id, "cost", &[i64::from(u32::MAX) + 1]).is_err());
        assert_eq!(wire_ranks(id, "time", &RankValue::Scalar(7), 3)?, [7, 7, 7]);
        assert_eq!(wire_ranks(id, "time", &RankValue::Levels(vec![7, 0, 9]), 3)?, [7, 0, 9]);
        assert!(wire_ranks(id, "time", &RankValue::Levels(vec![7]), 2).is_err());
        assert!(wire_ranks(id, "time", &RankValue::Scalar(-1), 1).is_err());
        Ok(())
    }

    #[test]
    fn classification_preserves_source_types_and_rank_strings() -> anyhow::Result<()> {
        for (source, expected) in [
            ("A1", SkillOperateType::ActiveInstant),
            ("A2", SkillOperateType::ActiveInstantContinuous),
            ("A3", SkillOperateType::ActiveContinuous),
            ("P", SkillOperateType::Passive),
            ("T", SkillOperateType::Toggle),
            ("A4", SkillOperateType::Other),
            ("CA1", SkillOperateType::Other),
            ("DA2", SkillOperateType::Other),
            ("future", SkillOperateType::Other),
        ] {
            assert_eq!(operate_type(source), expected);
        }
        let id = SkillId(3);
        assert_eq!(
            wire_strings(id, "scope", &RankValue::Scalar("unset".into()), 2)?,
            ["unset", "unset"]
        );
        assert_eq!(
            wire_strings(id, "target", &RankValue::Levels(vec!["SELF".into(), "ENEMY".into()]), 2)?,
            ["SELF", "ENEMY"]
        );
        assert!(wire_strings(id, "target", &RankValue::Levels(vec!["SELF".into()]), 2).is_err());
        Ok(())
    }

    #[test]
    fn phase4_catalogue_epoch_uses_canonical_promotions_and_refuses_metadata_drift() {
        let classes = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap();
        let bundle = crate::infrastructure::item_data::embedded_phase4_bundle().unwrap();
        assert!(RegistrySkillCatalogue::new(&classes.registry, bundle.skills.skills()).is_err());
        let adapter = RegistrySkillCatalogue::for_rules(&classes.registry, &bundle.skills).unwrap();
        let catalogue = adapter.catalogue().unwrap();
        for entry in &catalogue.skills {
            let actual = bundle.skills.skills().get(entry.skill_id).unwrap();
            assert_eq!(
                entry.effect_implemented,
                actual.runtime_status == RuntimeSkillStatus::Executable
            );
            assert_eq!(entry.deferred_reasons, actual.blockers);
        }
        let mut metadata = serde_json::to_value(classes.registry.as_ref()).unwrap();
        *metadata
            .get_mut("known_skills")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap()
            .get_mut("source_file")
            .unwrap() = "src/main/resources/data/stats/skills/wrong.xml".into();
        let drifted: ClassRegistry = serde_json::from_value(metadata).unwrap();
        assert!(RegistrySkillCatalogue::for_rules(&drifted, &bundle.skills).is_err());
    }

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
        assert_eq!(first.skills.iter().filter(|s| s.effect_implemented).count(), 180);
        assert_eq!(
            first
                .skills
                .iter()
                .filter(|s| !s.effect_implemented)
                .count(),
            198
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
