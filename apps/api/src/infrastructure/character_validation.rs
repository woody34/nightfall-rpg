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

/// D116: under V6 skill rules (runtime revision 3) a retained V5 skill checkpoint gets an
/// install source for every learned passive that V6 promotes to Executable and that has none,
/// appended once in skill-id order after the retained sources (as a passive learned
/// mid-session is installed). Applied at the persistence boundary before validation; the
/// admission load carries the upgraded checkpoint into the recorded `SpawnPlayer`. A no-op for
/// V5 rules and for checkpoints that are already complete.
pub(super) fn upgrade_v6_skill_checkpoint(
    phase3: Option<crate::domain::zone::Phase3Checkpoint>,
    rules: &crate::domain::zone::Phase3Rules,
    learned: &[crate::domain::subclass::LearnedSkill],
) -> Option<crate::domain::zone::Phase3Checkpoint> {
    use crate::domain::class::RuntimeSkillStatus;
    use crate::domain::zone::effects::RetainedModifierSource;
    let mut phase3 = phase3?;
    if rules.runtime_revision() != 3 {
        return Some(phase3);
    }
    let installed: std::collections::BTreeSet<_> = phase3
        .modifier_install_order
        .iter()
        .filter_map(|source| match source {
            RetainedModifierSource::Passive { skill } => Some(skill.id),
            RetainedModifierSource::Effect { .. } | RetainedModifierSource::DeathPenalty => None,
        })
        .collect();
    for skill in rules.skills().skills().values().filter(|skill| {
        skill.operate_type == "P"
            && skill.runtime_status == RuntimeSkillStatus::Executable
            && !installed.contains(&skill.id)
    }) {
        if let Some(entry) = learned.iter().find(|entry| entry.key == skill.key) {
            phase3
                .modifier_install_order
                .push(RetainedModifierSource::Passive {
                    skill: crate::domain::zone::skill::SkillRef {
                        id: skill.id,
                        rank: entry.level,
                    },
                });
        }
    }
    Some(phase3)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::domain::subclass::LearnedSkill;
    use crate::domain::zone::effects::RetainedModifierSource;
    use crate::domain::zone::skill::{SkillId, SkillRef};
    use crate::domain::zone::Phase3Checkpoint;

    fn passive(id: u32, rank: u32) -> RetainedModifierSource {
        RetainedModifierSource::Passive {
            skill: SkillRef {
                id: SkillId(id),
                rank,
            },
        }
    }

    #[test]
    fn d116_appends_v6_promoted_learned_passives_once_in_skill_order() {
        let v5 = crate::infrastructure::skill_data::load_phase3_rules(
            &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
            &[],
        )
        .unwrap();
        let v6 = crate::infrastructure::item_data::embedded_phase4_bundle()
            .unwrap()
            .skills;
        let learned: Vec<_> = [
            ("l2.skill.214", 1),
            ("l2.skill.163", 1),
            ("l2.skill.118", 1),
        ]
        .into_iter()
        .map(|(key, level)| LearnedSkill {
            key: key.into(),
            level,
        })
        .collect();
        let mut retained = Phase3Checkpoint::new();
        retained.modifier_install_order = vec![RetainedModifierSource::DeathPenalty];
        // V5 rules: unchanged (none of these passives executes in V5).
        assert_eq!(
            upgrade_v6_skill_checkpoint(Some(retained.clone()), &v5, &learned),
            Some(retained.clone())
        );
        let upgraded = upgrade_v6_skill_checkpoint(Some(retained.clone()), &v6, &learned).unwrap();
        assert_eq!(
            upgraded.modifier_install_order,
            vec![
                RetainedModifierSource::DeathPenalty,
                passive(118, 1),
                passive(163, 1),
                passive(214, 1)
            ]
        );
        // Complete already: a no-op (append once).
        assert_eq!(
            upgrade_v6_skill_checkpoint(Some(upgraded.clone()), &v6, &learned),
            Some(upgraded)
        );
        assert_eq!(upgrade_v6_skill_checkpoint(None, &v6, &learned), None);
    }
}
