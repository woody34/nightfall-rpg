//! Validated V5 bootstrap inputs. Preparing a bundle never opens an epoch or enables
//! production gameplay; the composition root installs the durable checkpoint lane.

use std::fmt::Write as _;

use crate::application::zone_bootstrap::{slot_specs, ResolvedRules, ZoneDefinition};
use crate::domain::zone::{Phase3Rules, ZoneSeed, ZoneState};
use crate::infrastructure::class_data::ResolvedClasses;
use crate::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};

/// Immutable inputs shared by a future V5 epoch, repository adapters and catalogue.
#[derive(Debug, Clone)]
pub struct PreparedPhase3Bootstrap {
    /// Existing physical rules and their unchanged legacy hash.
    pub physical: ResolvedRules,
    /// Complete professions/ranks/growth and their canonical hash.
    pub classes: ResolvedClasses,
    /// Skills, classes-phase3, NPC/weapon profiles, PvP/regen/formulas and peace geometry.
    pub phase3: Phase3Rules,
    /// Configured zone and its unchanged definition hash.
    pub zone: ZoneDefinition,
}

impl PreparedPhase3Bootstrap {
    /// Constructs an explicit V5 state without starting an actor, writing an epoch or
    /// bypassing the production switch. All provenance survives snapshot restoration.
    pub fn state(&self, seed: ZoneSeed, time_origin_ms: i64) -> anyhow::Result<ZoneState> {
        anyhow::ensure!(seed.zone == self.zone.zone, "Phase3 bootstrap zone does not match seed");
        let state = ZoneState::new_phase3(
            seed,
            self.zone.bounds,
            time_origin_ms,
            self.physical.rules.clone(),
            self.classes.registry.clone(),
            self.phase3.clone(),
        )?
        .with_safe_point(self.zone.safe_point)
        .with_spawn_slots(slot_specs(&self.physical.rules, &self.zone)?);
        let mut snapshot = state.snapshot();
        snapshot.meta.config_hash.clone_from(&self.zone.config_hash);
        snapshot
            .meta
            .rules_hash
            .clone_from(&self.physical.config_hash);
        snapshot
            .meta
            .classes_hash
            .clone_from(&self.classes.config_hash);
        Ok(ZoneState::from_snapshot(snapshot)?)
    }
}

/// Checks a skill bundle (V5, or the V6 extension) against the class catalogue, the zone's
/// peace geometry, the physical starter weapon and every NPC template of this installation.
pub(super) fn validate_phase3_rules(
    phase3: &Phase3Rules,
    physical: &ResolvedRules,
    classes: &ResolvedClasses,
    zone: &ZoneDefinition,
) -> anyhow::Result<()> {
    phase3
        .validate_catalogue(&classes.registry)
        .map_err(anyhow::Error::msg)?;
    phase3.geometry().peace_square(zone.bounds)?;
    phase3
        .profiles()
        .starter_weapon
        .validate_physical(physical.rules.starter_weapon())
        .map_err(anyhow::Error::msg)?;
    for template in &zone.npc_templates {
        let profile = phase3.profiles().npcs.get(&template.id.0).ok_or_else(|| {
            anyhow::anyhow!("Phase3 rules missing NPC template {}", template.id.0)
        })?;
        anyhow::ensure!(
            profile.physical
                == crate::domain::zone::skill_profiles::NpcPhysicalProfile::from_template(template),
            "Phase3 rules disagree with NPC template {}",
            template.id.0
        );
    }
    Ok(())
}

/// Validates every input before any production broker connection, recovery or epoch write.
/// Class-catalogue and NPC-template compatibility prevent combining different installations.
pub fn prepare_phase3_bootstrap(
    source: &Phase3DataSource,
    physical: ResolvedRules,
    classes: ResolvedClasses,
    zone: ZoneDefinition,
) -> anyhow::Result<PreparedPhase3Bootstrap> {
    let required = classes
        .registry
        .known_skills()
        .iter()
        .map(|s| crate::domain::zone::skill::SkillId(s.id))
        .collect::<Vec<_>>();
    let phase3 = load_phase3_rules(source, &required)?;
    validate_phase3_rules(&phase3, &physical, &classes, &zone)?;
    let mut hash = "sha256:".to_owned();
    for byte in phase3.hash() {
        write!(&mut hash, "{byte:02x}")?;
    }
    tracing::info!(phase3_rules_hash = %hash,
        physical_rules_hash = %physical.config_hash,
        classes_hash = %classes.config_hash,
        zone_config_hash = %zone.config_hash,
        "Phase3 bootstrap bundle validated");
    Ok(PreparedPhase3Bootstrap {
        physical,
        classes,
        phase3,
        zone,
    })
}
