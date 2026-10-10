//! Pure subclass eligibility and reserved persistence state; village-master flows are deferred.

use serde::{Deserialize, Serialize};

use super::class::{ClassId, ClassRegistry};
use super::Race;

/// Retail maximum number of additional class slots.
pub const MAX_SUBCLASSES: usize = 3;
/// A new subclass starts at the second-transfer level.
pub const SUBCLASS_START_LEVEL: u32 = 40;
/// Maximum certifications from one subclass.
pub const CERTIFICATIONS_PER_SUBCLASS: usize = 4;
/// Maximum certifications from all three subclasses.
pub const MAX_CERTIFICATIONS: usize = 12;

/// Stable sex selection, reserved for appearance and future gender-restricted classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sex {
    /// Male appearance.
    Male,
    /// Female appearance.
    Female,
}

/// Reserved skill state. Actual skill definitions and learning rules belong to Phase 3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct LearnedSkill {
    pub key: String,
    pub level: u32,
}

/// Independent progress for one main or subclass slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct ClassProgress {
    pub class_id: ClassId,
    pub level: u32,
    pub exp: u64,
    pub sp: u64,
    pub skills: Vec<LearnedSkill>,
}

/// A certification belongs to a subclass slot; skill effects are deferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct Certification {
    pub subclass_slot: u8,
    pub skill_key: String,
}

/// Inputs to eligibility. Character ownership and interaction range are application concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct EligibilityContext {
    pub race: Race,
    pub main_class: ClassId,
    pub main_level: u32,
    /// Either Fate's Whisper or Mimir's Elixir was completed.
    pub quest_completed: bool,
    pub noble: bool,
    /// Existing subclass professions, excluding the main slot.
    pub held_classes: Vec<ClassId>,
}

/// Why a second-tier profession cannot be selected as a subclass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubclassDenied {
    /// Main or candidate profession is absent, including reserved Kamael ids.
    #[error("unknown profession")]
    UnknownClass,
    /// Main class has not completed its second transfer.
    #[error("main class must have completed the second transfer")]
    MainTransfer,
    /// Main level has not reached 75.
    #[error("main class must be level 75")]
    MainLevel,
    /// Neither subclass quest nor noble status is present.
    #[error("subclass quest or noble status is required")]
    Quest,
    /// Three additional slots are already occupied.
    #[error("all subclass slots are occupied")]
    SlotsFull,
    /// Only eligible second-tier professions are selectable; Overlord/Warsmith are excluded.
    #[error("profession is not allowed as a subclass")]
    ForbiddenClass,
    /// Elf and Dark Elf lineages cannot cross.
    #[error("race restriction")]
    Race,
    /// Same profession or one of its equivalent professions is already held.
    #[error("same or equivalent profession already held")]
    Equivalent,
}

/// Applies classic subclass rules without mutating character state or learning skills.
pub fn eligible(
    registry: &ClassRegistry,
    main: &EligibilityContext,
    candidate: ClassId,
) -> Result<(), SubclassDenied> {
    let class = registry
        .get(candidate)
        .ok_or(SubclassDenied::UnknownClass)?;
    let main_class = registry
        .get(main.main_class)
        .ok_or(SubclassDenied::UnknownClass)?;
    if main_class.tier < 2 {
        return Err(SubclassDenied::MainTransfer);
    }
    if main.main_level < 75 {
        return Err(SubclassDenied::MainLevel);
    }
    if !main.quest_completed && !main.noble {
        return Err(SubclassDenied::Quest);
    }
    if main.held_classes.len() >= MAX_SUBCLASSES {
        return Err(SubclassDenied::SlotsFull);
    }
    if class.tier != 2 || !class.subclass_allowed {
        return Err(SubclassDenied::ForbiddenClass);
    }
    if (main.race == Race::Elf && class.race == Race::DarkElf)
        || (main.race == Race::DarkElf && class.race == Race::Elf)
    {
        return Err(SubclassDenied::Race);
    }
    for held in std::iter::once(&main.main_class).chain(&main.held_classes) {
        let held = second_tier(registry, *held).ok_or(SubclassDenied::UnknownClass)?;
        if held == candidate || class.subclass_equivalents.contains(&held) {
            return Err(SubclassDenied::Equivalent);
        }
    }
    Ok(())
}

fn second_tier(registry: &ClassRegistry, id: ClassId) -> Option<ClassId> {
    let class = registry.get(id)?;
    if class.tier == 2 {
        Some(id)
    } else if class.tier == 3 {
        class
            .parent
            .filter(|parent| registry.get(*parent).is_some_and(|c| c.tier == 2))
    } else {
        None
    }
}

/// Post-MVP subclass level cap, five below the authoritative main cap.
#[must_use]
pub const fn subclass_level_cap(main_level_cap: u32) -> u32 {
    main_level_cap.saturating_sub(5)
}
