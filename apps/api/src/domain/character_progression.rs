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
#[serde(try_from = "ClassStateParts")]
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
    pub successful_transfer_receipts: Vec<SuccessfulTransferReceipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassStateParts {
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
    pub successful_transfer_receipts: Vec<SuccessfulTransferReceipt>,
}

impl TryFrom<ClassStateParts> for ClassState {
    type Error = ClassStateError;
    fn try_from(parts: ClassStateParts) -> Result<Self, Self::Error> {
        let state = Self {
            base_class_id: parts.base_class_id,
            current_class_id: parts.current_class_id,
            sp: parts.sp,
            learned_skills: parts.learned_skills,
            cp: parts.cp,
            token_tier_1_count: parts.token_tier_1_count,
            token_tier_2_count: parts.token_tier_2_count,
            milestone_claimed_mask: parts.milestone_claimed_mask,
            successful_transfer_receipts: parts.successful_transfer_receipts,
        };
        state.validate()?;
        Ok(state)
    }
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
        let mut prior: Option<&FrozenTransferResult> = None;
        for receipt in &self.successful_transfer_receipts {
            let result = &receipt.result;
            if result.identity.base_class_id != self.base_class_id {
                return Err(ClassStateError::Identity);
            }
            if prior.is_some_and(|previous| {
                previous.character_id != result.character_id
                    || previous.identity != result.identity
                    || previous.name != result.name
                    || previous.stats != result.stats
            }) {
                return Err(ClassStateError::Identity);
            }
            prior = Some(result);
        }
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

    /// Contextual admission/checkpoint/snapshot validation against immutable catalogue and
    /// containing character. Historical mutable resources are never compared with live state.
    pub fn validate_for(
        &self,
        registry: &super::class::ClassRegistry,
        identity: &CharacterIdentity,
        character_id: CharacterId,
    ) -> Result<(), ClassStateError> {
        self.validate()?;
        if identity.base_class_id != self.base_class_id
            || identity.appearance.hair_style != 0
            || identity.appearance.hair_color != 0
            || identity.appearance.face != 0
        {
            return Err(ClassStateError::Identity);
        }
        let base = registry
            .get(self.base_class_id)
            .ok_or(ClassStateError::Lineage)?;
        if base.tier != 0 || base.race != identity.race || base.base_class_id != self.base_class_id
        {
            return Err(ClassStateError::Lineage);
        }
        let current = registry
            .get(self.current_class_id)
            .ok_or(ClassStateError::Lineage)?;
        if current.tier > 2
            || current.race != identity.race
            || current.base_class_id != self.base_class_id
        {
            return Err(ClassStateError::Lineage);
        }
        self.validate_learned_metadata(registry, identity.race)?;
        let mut parent = self.base_class_id;
        for (ordinal, receipt) in self.successful_transfer_receipts.iter().enumerate() {
            let target = registry
                .get(receipt.target_class_id)
                .ok_or(ClassStateError::Lineage)?;
            if target.tier > 2
                || usize::from(target.tier) != ordinal.saturating_add(1)
                || target.parent != Some(parent)
                || target.base_class_id != self.base_class_id
                || target.race != identity.race
            {
                return Err(ClassStateError::ReceiptHistory);
            }
            let result = &receipt.result;
            if result.character_id != character_id
                || result.identity != *identity
                || result.stats != base.base_stats
            {
                return Err(ClassStateError::Identity);
            }
            if !(target.min_level..=85).contains(&result.level)
                || result.hp > result.max_hp
                || result.mp > result.max_mp
                || result.cp > result.max_cp
            {
                return Err(ClassStateError::ReceiptHistory);
            }
            parent = target.id;
        }
        if parent != self.current_class_id
            || usize::from(current.tier) != self.successful_transfer_receipts.len()
        {
            return Err(ClassStateError::ReceiptHistory);
        }
        Ok(())
    }

    fn validate_learned_metadata(
        &self,
        registry: &super::class::ClassRegistry,
        race: Race,
    ) -> Result<(), ClassStateError> {
        let mut allowed = std::collections::BTreeSet::new();
        for row in registry
            .skill_tree(self.current_class_id)
            .map_err(|_| ClassStateError::Lineage)?
        {
            if row.auto_get && row.required_level <= 85 {
                let known = registry
                    .known_skill(row.skill_id)
                    .ok_or(ClassStateError::LearnedSkills)?;
                if known.key != row.key || row.skill_level > known.max_level {
                    return Err(ClassStateError::LearnedSkills);
                }
                allowed.insert((row.key.as_str(), row.skill_level));
            }
        }
        for key in &registry
            .race(race)
            .ok_or(ClassStateError::Identity)?
            .passive_skill_keys
        {
            allowed.insert((key.as_str(), 1));
        }
        if self
            .learned_skills
            .iter()
            .any(|skill| !allowed.contains(&(skill.key.as_str(), skill.level)))
        {
            return Err(ClassStateError::LearnedSkills);
        }
        Ok(())
    }

    /// Validates incoming free-learning metadata before atomically merging it. Attainable
    /// inherited levels remain valid after deleveling; foreign lineages and racial keys fail.
    pub fn merge_learned_skills_checked(
        &mut self,
        registry: &super::class::ClassRegistry,
        race: Race,
        skills: impl IntoIterator<Item = LearnedSkill>,
    ) -> Result<Vec<String>, ClassStateError> {
        let incoming: Vec<_> = skills.into_iter().collect();
        let mut probe = self.clone();
        probe.learned_skills.clone_from(&incoming);
        probe.validate_learned_metadata(registry, race)?;
        let mut candidate = self.clone();
        let granted = candidate.merge_learned_skills(incoming);
        candidate.validate()?;
        candidate.validate_learned_metadata(registry, race)?;
        *self = candidate;
        Ok(granted)
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
        if receipt.result.identity.base_class_id != self.base_class_id {
            return Err(ClassStateError::Identity);
        }
        if self
            .successful_transfer_receipts
            .first()
            .is_some_and(|known| {
                known.result.character_id != receipt.result.character_id
                    || known.result.identity != receipt.result.identity
                    || known.result.name != receipt.result.name
                    || known.result.stats != receipt.result.stats
            })
        {
            return Err(ClassStateError::Identity);
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
    /// Active profession is unknown, unsupported or belongs to another lineage/race.
    #[error("invalid class lineage")]
    Lineage,
    /// Ledger or frozen response contradicts immutable containing identity.
    #[error("class ledger identity mismatch")]
    Identity,
    /// Receipt sequence does not describe the current reachable transfer history.
    #[error("invalid class transfer receipt history")]
    ReceiptHistory,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn receipt(key: u128, target: u32) -> SuccessfulTransferReceipt {
        let mut character = super::super::Character::create(
            AccountId::from_uuid(Uuid::nil()),
            CharacterName::new("Hero").unwrap(),
            Race::Human,
        );
        character.id = CharacterId::from_uuid(Uuid::nil());
        SuccessfulTransferReceipt {
            key: Uuid::from_u128(key),
            target_class_id: ClassId(target),
            result: FrozenTransferResult {
                character_id: character.id,
                identity: character.identity(),
                name: character.name,
                current_class_id: ClassId(target),
                level: 20,
                xp: 100,
                sp: 0,
                stats: character.stats,
                position_millitiles: [126_000, 126_000],
                hp: 50,
                mp: 20,
                cp: 10,
                max_hp: 100,
                max_mp: 40,
                max_cp: 20,
                token_tier_1_count: 0,
                token_tier_2_count: 0,
                granted_skill_keys: Vec::new(),
            },
        }
    }

    #[test]
    fn success_history_is_bounded_and_keys_are_immutable() {
        let mut state = ClassState::new(ClassId(0));
        let first = receipt(1, 1);
        state.record_success(first.clone()).unwrap();
        state.record_success(first.clone()).unwrap();
        assert_eq!(state.successful_transfer_receipts.len(), 1);
        let mut changed = first.clone();
        changed.target_class_id = ClassId(4);
        assert_eq!(state.record_success(changed), Err(ClassStateError::ConflictingReceipt));
        state.record_success(receipt(2, 2)).unwrap();
        assert_eq!(state.record_success(receipt(3, 3)), Err(ClassStateError::ReceiptLimit));
        state.validate().unwrap();
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(serde_json::from_value::<ClassState>(json).unwrap(), state);
    }

    #[test]
    fn deserialization_refuses_unbounded_or_duplicate_history() {
        let mut state = ClassState::new(ClassId(0));
        state.successful_transfer_receipts = vec![receipt(1, 1), receipt(2, 2), receipt(3, 3)];
        assert!(
            serde_json::from_value::<ClassState>(serde_json::to_value(&state).unwrap()).is_err()
        );
        let one = receipt(1, 1);
        state.successful_transfer_receipts = vec![one.clone(), one];
        assert!(
            serde_json::from_value::<ClassState>(serde_json::to_value(&state).unwrap()).is_err()
        );
    }

    #[test]
    fn learned_metadata_merge_is_ordered_upgrades_only_and_does_not_charge_sp() {
        let mut state = ClassState::new(ClassId(0));
        state.sp = 500;
        let skill = |key: &str, level| LearnedSkill {
            key: key.into(),
            level,
        };
        assert_eq!(
            state.merge_learned_skills([skill("test.b", 1), skill("test.a", 2)]),
            vec!["test.a", "test.b"]
        );
        assert_eq!(
            state.merge_learned_skills([skill("test.a", 1), skill("test.b", 3)]),
            vec!["test.b"]
        );
        assert_eq!(state.learned_skills, vec![skill("test.a", 2), skill("test.b", 3)]);
        assert_eq!(state.sp, 500);
        state.validate().unwrap();
    }
    #[test]
    fn learned_metadata_rejects_forgery_but_retains_attainable_deleveled_skills() {
        let registry = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap()
        .registry;
        let result = receipt(1, 1).result;
        let mut state = ClassState::new(ClassId(0));
        let allowed = auto_get_metadata(&registry, ClassId(0), 85).unwrap();
        state
            .merge_learned_skills_checked(&registry, Race::Human, allowed.clone())
            .unwrap();
        state
            .validate_for(&registry, &result.identity, result.character_id)
            .unwrap();
        let foreign = auto_get_metadata(&registry, ClassId(10), 85)
            .unwrap()
            .into_iter()
            .find(|s| !allowed.iter().any(|a| a == s))
            .unwrap();
        let wrong_race = registry.race(Race::Elf).unwrap().passive_skill_keys[0].clone();
        let known = &registry.known_skills()[0];
        for skill in [
            LearnedSkill {
                key: "garbage".into(),
                level: 1,
            },
            LearnedSkill {
                key: "l2.skill.999999".into(),
                level: 1,
            },
            LearnedSkill {
                key: known.key.clone(),
                level: known.max_level + 1,
            },
            LearnedSkill {
                key: wrong_race,
                level: 1,
            },
            foreign,
        ] {
            let previous = state.clone();
            assert_eq!(
                state.merge_learned_skills_checked(&registry, Race::Human, [skill.clone()]),
                Err(ClassStateError::LearnedSkills)
            );
            assert_eq!(state, previous);
            let mut forged = previous;
            forged.merge_learned_skills([skill]);
            let decoded: ClassState =
                serde_json::from_value(serde_json::to_value(forged).unwrap()).unwrap();
            assert_eq!(
                decoded.validate_for(&registry, &result.identity, result.character_id),
                Err(ClassStateError::LearnedSkills)
            );
        }
    }

    #[test]
    fn entire_local_ledger_is_validated_during_deserialization() {
        for mask in 0..=255 {
            let mut state = ClassState::new(ClassId(0));
            state.milestone_claimed_mask = mask;
            let result =
                serde_json::from_value::<ClassState>(serde_json::to_value(&state).unwrap());
            assert_eq!(result.is_ok(), mask <= 3);
        }
        let mut state = ClassState::new(ClassId(0));
        let mut success = receipt(1, 1);
        success.result.identity.base_class_id = ClassId(10);
        state.successful_transfer_receipts.push(success);
        assert_eq!(state.validate(), Err(ClassStateError::Identity));
        assert!(
            serde_json::from_value::<ClassState>(serde_json::to_value(&state).unwrap()).is_err()
        );
    }

    #[test]
    fn contextual_validation_refuses_corrupt_lineages_identities_and_history() {
        let registry = crate::infrastructure::class_data::load_classes(
            &crate::infrastructure::class_data::ClassSource::embedded(),
        )
        .unwrap()
        .registry;
        let success = receipt(1, 1);
        let identity = success.result.identity.clone();
        let id = success.result.character_id;
        let mut state = ClassState::new(ClassId(0));
        state.validate_for(&registry, &identity, id).unwrap();
        for current in [31, 123, 88, 999] {
            state.current_class_id = ClassId(current);
            assert!(state.validate_for(&registry, &identity, id).is_err());
        }
        state.current_class_id = ClassId(1);
        state.record_success(success.clone()).unwrap();
        state.validate_for(&registry, &identity, id).unwrap();
        for field in 0..7 {
            let mut corrupt = state.clone();
            let result = &mut corrupt.successful_transfer_receipts[0].result;
            match field {
                0 => result.character_id = CharacterId::from_uuid(Uuid::from_u128(7)),
                1 => result.identity.account_id = AccountId::from_uuid(Uuid::from_u128(7)),
                2 => result.identity.race = Race::Elf,
                3 => result.identity.appearance.sex = Sex::Female,
                4 => result.identity.base_class_id = ClassId(18),
                5 => {
                    result.current_class_id = ClassId(88);
                    corrupt.successful_transfer_receipts[0].target_class_id = ClassId(88);
                },
                _ => {
                    result.current_class_id = ClassId(999);
                    corrupt.successful_transfer_receipts[0].target_class_id = ClassId(999);
                },
            }
            assert!(corrupt.validate_for(&registry, &identity, id).is_err(), "field {field}");
        }
    }
}
