//! Shared deterministic character identity, class ledger and frozen transfer responses.
//! No persistence or transport types belong here; the zone is the mutable ledger's owner.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::class::ClassId;
use super::subclass::{LearnedSkill, Sex};
use super::{AccountId, BaseStats, CharacterId, CharacterName, Race};

/// Only two class transfers are reachable in Phase 2. Successful receipts are retained forever.
pub const MAX_TRANSFER_RECEIPTS: usize = 2;

/// Appearance indices into the selectable race's assets (Phase 2 has only index zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CharacterAppearance {
    pub sex: Sex,
    pub hair_style: u32,
    pub hair_color: u32,
    pub face: u32,
}

impl Default for CharacterAppearance {
    fn default() -> Self {
        Self {
            sex: Sex::Male,
            hair_style: 0,
            hair_color: 0,
            face: 0,
        }
    }
}

/// Immutable identity, copied into admission commands and replay snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct CharacterIdentity {
    pub account_id: AccountId,
    pub race: Race,
    pub base_class_id: ClassId,
    pub appearance: CharacterAppearance,
}

/// Exact response at the successful transfer tick. Integer position is in milli-tiles.
/// Deliberately contains no ClassState/receipts, avoiding recursive snapshots and responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct FrozenTransferResult {
    pub character_id: CharacterId,
    pub identity: CharacterIdentity,
    pub name: CharacterName,
    pub current_class_id: ClassId,
    pub level: u32,
    pub xp: u64,
    pub sp: u64,
    pub stats: BaseStats,
    pub position_millitiles: [i32; 2],
    pub hp: u32,
    pub mp: u32,
    pub cp: u32,
    pub max_hp: u32,
    pub max_mp: u32,
    pub max_cp: u32,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
    pub granted_skill_keys: Vec<String>,
}

/// Only successful mutations enter this ledger. Target + character define the fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct SuccessfulTransferReceipt {
    pub key: Uuid,
    pub target_class_id: ClassId,
    pub result: FrozenTransferResult,
}

/// A bounded, forever-retained transfer history and the active main-class resource ledger.
/// XP/level/HP/MP remain in the existing progression state to avoid competing copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct ClassState {
    pub base_class_id: ClassId,
    pub current_class_id: ClassId,
    pub sp: u64,
    /// Learned metadata only; Phase 3 owns skill effects and active skill execution.
    #[serde(default)]
    pub learned_skills: Vec<LearnedSkill>,
    pub cp: u32,
    pub token_tier_1_count: u32,
    pub token_tier_2_count: u32,
    /// Bits 0 and 1 mark the level-20 and level-40 token grants, never reset by death.
    pub milestone_claimed_mask: u8,
    #[serde(deserialize_with = "deserialize_receipts")]
    pub successful_transfer_receipts: Vec<SuccessfulTransferReceipt>,
}

impl ClassState {
    /// Empty level-one ledger. Resource maxima are derived by the stat engine.
    #[must_use]
    pub const fn new(base_class_id: ClassId) -> Self {
        Self {
            base_class_id,
            current_class_id: base_class_id,
            sp: 0,
            learned_skills: Vec::new(),
            cp: 0,
            token_tier_1_count: 0,
            token_tier_2_count: 0,
            milestone_claimed_mask: 0,
            successful_transfer_receipts: Vec::new(),
        }
    }

    /// Rejects impossible persisted ledgers before any checkpoint write.
    pub fn validate(&self) -> Result<(), ClassStateError> {
        validate_receipts(&self.successful_transfer_receipts)?;
        let mut previous: Option<&str> = None;
        for skill in &self.learned_skills {
            if skill.key.is_empty()
                || skill.level == 0
                || previous.is_some_and(|key| key >= skill.key.as_str())
            {
                return Err(ClassStateError::LearnedSkills);
            }
            previous = Some(&skill.key);
        }
        if self.milestone_claimed_mask & !3 != 0 {
            return Err(ClassStateError::MilestoneMask);
        }
        Ok(())
    }

    /// Merges validated metadata by stable key, retaining highest levels and returning only
    /// newly learned/upgraded keys in deterministic order. Auto-get does not charge SP.
    pub fn merge_learned_skills(
        &mut self,
        skills: impl IntoIterator<Item = LearnedSkill>,
    ) -> Vec<String> {
        let mut levels: std::collections::BTreeMap<String, u32> = self
            .learned_skills
            .iter()
            .map(|s| (s.key.clone(), s.level))
            .collect();
        let mut granted = std::collections::BTreeSet::new();
        for skill in skills {
            let level = levels.entry(skill.key.clone()).or_default();
            if skill.level > *level {
                *level = skill.level;
                granted.insert(skill.key);
            }
        }
        self.learned_skills = levels
            .into_iter()
            .map(|(key, level)| LearnedSkill { key, level })
            .collect();
        granted.into_iter().collect()
    }

    /// Inserts a success exactly once, refusing conflicting keys and history overflow.
    pub fn record_success(
        &mut self,
        receipt: SuccessfulTransferReceipt,
    ) -> Result<(), ClassStateError> {
        if let Some(known) = self.receipt(receipt.key) {
            return if known == &receipt {
                Ok(())
            } else {
                Err(ClassStateError::ConflictingReceipt)
            };
        }
        if self.successful_transfer_receipts.len() >= MAX_TRANSFER_RECEIPTS {
            return Err(ClassStateError::ReceiptLimit);
        }
        if receipt.result.current_class_id != receipt.target_class_id {
            return Err(ClassStateError::ReceiptTarget);
        }
        self.successful_transfer_receipts.push(receipt);
        Ok(())
    }

    /// Lookup precedes live eligibility, so retry returns its original frozen result.
    #[must_use]
    pub fn receipt(&self, key: Uuid) -> Option<&SuccessfulTransferReceipt> {
        self.successful_transfer_receipts
            .iter()
            .find(|r| r.key == key)
    }
}

/// Invalid shared class ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClassStateError {
    /// More successes than reachable transfer tiers.
    #[error("at most two successful transfer receipts are allowed")]
    ReceiptLimit,
    /// A key occurs twice or conflicts with its existing success.
    #[error("conflicting transfer receipt")]
    ConflictingReceipt,
    /// The frozen response disagrees with the requested target.
    #[error("transfer receipt target does not match result")]
    ReceiptTarget,
    /// Learned metadata is not strictly ordered, has duplicate/empty keys or level zero.
    #[error("invalid learned skill metadata")]
    LearnedSkills,
    /// An unknown token milestone bit is present.
    #[error("unknown token milestone bit")]
    MilestoneMask,
}

fn validate_receipts(receipts: &[SuccessfulTransferReceipt]) -> Result<(), ClassStateError> {
    if receipts.len() > MAX_TRANSFER_RECEIPTS {
        return Err(ClassStateError::ReceiptLimit);
    }
    let mut keys = std::collections::BTreeSet::new();
    for receipt in receipts {
        if !keys.insert(receipt.key) {
            return Err(ClassStateError::ConflictingReceipt);
        }
        if receipt.result.current_class_id != receipt.target_class_id {
            return Err(ClassStateError::ReceiptTarget);
        }
    }
    Ok(())
}

fn deserialize_receipts<'de, D>(deserializer: D) -> Result<Vec<SuccessfulTransferReceipt>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let receipts = Vec::deserialize(deserializer)?;
    validate_receipts(&receipts).map_err(serde::de::Error::custom)?;
    Ok(receipts)
}

/// Legacy Phase 1 combat profile for a lineage's base profession; unknown ids fail closed.
#[must_use]
pub const fn base_class_profile(id: ClassId) -> Option<&'static str> {
    match id.0 {
        0 => Some("human_fighter"),
        10 => Some("human_mystic"),
        18 => Some("elven_fighter"),
        25 => Some("elven_mystic"),
        31 => Some("dark_fighter"),
        38 => Some("dark_mystic"),
        44 => Some("orc_fighter"),
        49 => Some("orc_mystic"),
        53 => Some("dwarven_fighter"),
        _ => None,
    }
}

/// Validated inherited free learning metadata up to a level; skill effects remain Phase 3.
pub fn auto_get_metadata(
    registry: &super::class::ClassRegistry,
    id: ClassId,
    level: u32,
) -> Result<Vec<LearnedSkill>, super::class::RegistryError> {
    use super::class::RegistryError;
    if !(1..=85).contains(&level) {
        return Err(RegistryError::LevelOutOfRange(level));
    }
    if registry.get(id).is_none() {
        return Err(RegistryError::UnknownClass(id));
    }
    let mut skills = std::collections::BTreeMap::<String, u32>::new();
    for class_id in std::iter::once(id).chain(registry.ancestors(id)) {
        let class = registry
            .get(class_id)
            .ok_or(RegistryError::UnknownClass(class_id))?;
        for skill in &class.skill_tree {
            if skill.auto_get && skill.required_level <= level {
                let known = skills.entry(skill.key.clone()).or_default();
                *known = (*known).max(skill.skill_level);
            }
        }
    }
    Ok(skills
        .into_iter()
        .map(|(key, level)| LearnedSkill { key, level })
        .collect())
}
