//! Phase 3 epoch boundary. This sidecar is never synthesized while restoring legacy data.
//!
//! The initial boundary commit reserves canonical allocation counters and immutable policy.
//! Gameplay construction remains fenced until registry and entity validation are installed.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Recorded pre-draft admission policy (design v3 §3/F2). These fixed-width values
/// are epoch inputs. The actor additionally proves the exact encoded envelope against
/// the durable gate ceiling with `validate_record_budget` before accepting any input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(missing_docs)]
pub struct Phase3AdmissionPolicy {
    pub version: u32,
    pub record_budget: u64,
    pub max_command_bytes: u64,
    pub max_observers: u32,
}
impl Phase3AdmissionPolicy {
    /// Explicit version-one defaults shared with actor admission; never synthesized for
    /// legacy or missing V5 metadata. The log retains 4096 bytes below its 1-MiB ceiling.
    #[must_use]
    pub const fn version_one() -> Self {
        Self {
            version: 1,
            record_budget: 1_044_480,
            max_command_bytes: 262_144,
            max_observers: 1_000,
        }
    }

    /// Portable structural checks. Exact protobuf framing/recipient feasibility and the
    /// backend ceiling remain application-layer checks; this module has no codec dependency.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.version == 1
            && self.max_command_bytes > 0
            && self.max_command_bytes <= 262_144
            && self.record_budget > self.max_command_bytes
            && self.record_budget <= u32::MAX as u64
            && self.max_observers > 0
    }
}

/// Immutable Phase 3 policy, separately hashed from historical stat/class registries.
/// Construction validates every registry capability and policy table; snapshot decoding
/// performs the same checks without loading any external files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Phase3Rules {
    timer_policy_version: u32,
    skills: super::skill::SkillRegistry,
    pvp: super::skill_rules::PvpRules,
    regen: super::skill_rules::RegenRules,
    constants: super::skill_rules::Phase3Constants,
    profiles: super::skill_profiles::SkillProfiles,
    geometry: super::state_pvp::Phase3Geometry,
    npc: super::skill_npc::NpcSkillRules,
    hash: [u8; 32],
}

impl Phase3Rules {
    /// Assemble a validated immutable rules bundle. The timer policy is explicitly paused
    /// offline (version one); this constructor cannot promote legacy snapshots.
    #[allow(clippy::too_many_arguments)] // Every immutable source bundle is explicit and required.
    pub fn try_new(
        skills: super::skill::SkillRegistry,
        pvp: super::skill_rules::PvpRules,
        regen: super::skill_rules::RegenRules,
        constants: super::skill_rules::Phase3Constants,
        profiles: super::skill_profiles::SkillProfiles,
        geometry: super::state_pvp::Phase3Geometry,
        npc: super::skill_npc::NpcSkillRules,
    ) -> Result<Self, Vec<String>> {
        let errors: Vec<_> = [
            pvp.validate(),
            regen.validate(),
            constants.validate(),
            profiles.validate(),
            geometry.validate(),
            npc.validate(),
        ]
        .into_iter()
        .filter_map(Result::err)
        .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        let timer_policy_version = 1_u32;
        // Fixed execution constants are hashed too. Sources: character.properties:145,268
        // (EffectTickRatio / MaxMAtkSpeed), accepted offline retention and offer policies.
        let bytes = serde_json::to_vec(&(
            timer_policy_version,
            &skills,
            &pvp,
            &regen,
            666_u32,
            &constants,
            &profiles,
            &geometry,
            &npc,
            50_u64,
            100_u64,
            600_u64,
            super::skill::PRIVILEGED_SKILL_RANGES,
        ))
        .map_err(|error| vec![format!("Phase3 rules canonical encoding: {error}")])?;
        let mut hash = Sha256::new();
        hash.update(b"nightfall.phase3.rules.1\0");
        hash.update(bytes);
        Ok(Self {
            timer_policy_version,
            skills,
            pvp,
            regen,
            constants,
            profiles,
            geometry,
            npc,
            hash: hash.finalize().into(),
        })
    }

    /// Class-tree references must resolve to the same rank/capability metadata in this epoch.
    /// This prevents combining a current skill registry with an older class catalogue.
    pub fn validate_catalogue(
        &self,
        classes: &crate::domain::class::ClassRegistry,
    ) -> Result<(), String> {
        self.profiles.validate_catalogue(classes)?;
        for known in classes.known_skills() {
            let skill = self
                .skills
                .get(super::skill::SkillId(known.id))
                .ok_or_else(|| format!("Phase3 rules missing catalogue skill {}", known.id))?;
            if skill.key != known.key
                || skill.max_level != known.max_level
                || skill.l2_ref != known.l2_ref
                || skill.runtime_status != known.runtime_status
                || skill.provenance.file != known.source_file
            {
                return Err(format!("Phase3 rules disagree with catalogue skill {}", known.id));
            }
        }
        if self.skills.get(super::skill::SkillId(5076)).is_none() {
            return Err("Phase3 rules missing system skill 5076".into());
        }
        Ok(())
    }

    /// Stable hash of the full rules bundle, including provenance and engine policy.
    pub const fn hash(&self) -> [u8; 32] {
        self.hash
    }
    /// Paused-timer persistence policy version.
    pub const fn timer_policy_version(&self) -> u32 {
        self.timer_policy_version
    }
    /// Capability-validated skill registry; source-only samples are absent.
    pub const fn skills(&self) -> &super::skill::SkillRegistry {
        &self.skills
    }
    /// Source flag/karma policy, with exact per-level divisors.
    pub const fn pvp(&self) -> &super::skill_rules::PvpRules {
        &self.pvp
    }
    /// Source-pinned C5 clamps/caps, included in the immutable epoch hash.
    pub const fn constants(&self) -> &super::skill_rules::Phase3Constants {
        &self.constants
    }
    /// Source class and NPC magic/regeneration inputs.
    pub const fn profiles(&self) -> &super::skill_profiles::SkillProfiles {
        &self.profiles
    }
    /// Source regeneration periods and multipliers.
    pub const fn regen(&self) -> &super::skill_rules::RegenRules {
        &self.regen
    }
    /// Pinned NPC penalties and hit-condition data, independently validated and hashed.
    pub const fn npc(&self) -> &super::skill_npc::NpcSkillRules {
        &self.npc
    }
    /// Explicit architect-configured peace geometry, included in the V5 policy hash.
    pub const fn geometry(&self) -> &super::state_pvp::Phase3Geometry {
        &self.geometry
    }
}

impl<'de> Deserialize<'de> for Phase3Rules {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Snapshot {
            timer_policy_version: u32,
            skills: super::skill::SkillRegistry,
            pvp: super::skill_rules::PvpRules,
            regen: super::skill_rules::RegenRules,
            constants: super::skill_rules::Phase3Constants,
            profiles: super::skill_profiles::SkillProfiles,
            geometry: super::state_pvp::Phase3Geometry,
            npc: super::skill_npc::NpcSkillRules,
            hash: [u8; 32],
        }
        let snapshot = Snapshot::deserialize(deserializer)?;
        let rules = Self::try_new(
            snapshot.skills,
            snapshot.pvp,
            snapshot.regen,
            snapshot.constants,
            snapshot.profiles,
            snapshot.geometry,
            snapshot.npc,
        )
        .map_err(|errors| serde::de::Error::custom(errors.join("; ")))?;
        if snapshot.timer_policy_version != rules.timer_policy_version
            || snapshot.hash != rules.hash
        {
            return Err(serde::de::Error::custom("Phase3 rules hash/policy mismatch"));
        }
        Ok(rules)
    }
}

/// Canonical Phase 3 snapshot block. Legacy snapshots must omit the field entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase3Snapshot {
    /// Complete immutable rules for this epoch, never fetched by replay.
    pub rules: std::sync::Arc<Phase3Rules>,
    /// Verified immutable rules fingerprint.
    pub rules_hash: [u8; 32],
    /// Next spawn allocation order, independent of UUID/RNG order.
    pub next_entity_ordinal: u64,
    /// Next cast identity.
    pub next_cast_id: u64,
    /// Next effect identity.
    pub next_effect_id: u64,
    /// Next resurrection offer identity.
    pub next_offer_id: u64,
    /// Shared passive/effect modifier installation sequence.
    pub next_install_sequence: u64,
    /// Complete canonical combatant sidecars, ordered by entity identity.
    pub entities: BTreeMap<super::EntityId, super::Phase3Entity>,
}

impl Phase3Snapshot {
    /// New empty epoch. All identities start at one; zero is never a live identity.
    pub fn new(rules: Phase3Rules) -> Self {
        Self {
            rules_hash: rules.hash(),
            rules: std::sync::Arc::new(rules),
            next_entity_ordinal: 1,
            next_cast_id: 1,
            next_effect_id: 1,
            next_offer_id: 1,
            next_install_sequence: 1,
            entities: BTreeMap::new(),
        }
    }

    pub(super) fn validate(&self) -> bool {
        self.rules.timer_policy_version == 1
            && self.rules_hash == self.rules.hash()
            && [
                self.next_entity_ordinal,
                self.next_cast_id,
                self.next_effect_id,
                self.next_offer_id,
                self.next_install_sequence,
            ]
            .iter()
            .all(|n| *n > 0)
            && self.validate_allocations().is_ok()
    }

    /// Prove global uniqueness and that allocation never reuses any live identity. Entity
    /// life/source validation is performed separately against the corresponding combatant.
    pub fn validate_allocations(&self) -> Result<(), super::effects::EffectError> {
        use super::effects::EffectError;
        let mut ordinals = BTreeSet::new();
        let mut casts = BTreeSet::new();
        let mut effects = BTreeSet::new();
        let mut offers = BTreeSet::new();
        let mut installs = BTreeSet::new();
        let insert = |value, next, seen: &mut BTreeSet<u64>| {
            if value == 0 || value >= next || !seen.insert(value) {
                Err(EffectError::Invalid("duplicate/out-of-counter live allocation"))
            } else {
                Ok(())
            }
        };
        for entity in self.entities.values() {
            insert(entity.ordinal.0, self.next_entity_ordinal, &mut ordinals)?;
            if let super::CastState::Casting(cast) = &entity.cast {
                insert(cast.id.0, self.next_cast_id, &mut casts)?;
            }
            for effect in entity.effects.entries().keys() {
                insert(effect.0, self.next_effect_id, &mut effects)?;
            }
            if let Some(offer) = entity
                .player
                .as_ref()
                .and_then(|p| p.resurrect_offer.as_ref())
            {
                insert(offer.id.0, self.next_offer_id, &mut offers)?;
            }
            for sequence in entity.modifier_install_order.keys() {
                insert(*sequence, self.next_install_sequence, &mut installs)?;
            }
        }
        Ok(())
    }

    /// Validate every sidecar against independently restored combat facts. The caller must
    /// supply exactly all combatants, and subsequently rebuild/compare effective sheets.
    pub fn validate_live(
        &self,
        combatants: &BTreeMap<super::EntityId, super::Phase3EntityContext<'_>>,
    ) -> Result<(), super::effects::EffectError> {
        if !self.validate() || combatants.len() != self.entities.len() {
            return Err(super::effects::EffectError::Invalid("Phase3 sidecar cardinality/policy"));
        }
        for (id, context) in combatants {
            self.entities
                .get(id)
                .ok_or(super::effects::EffectError::Invalid("missing combatant sidecar"))?
                .validate_live(&self.rules, *context)?;
        }
        Ok(())
    }
}

/// Runtime sidecar; derived indexes will be rebuilt from its canonical snapshot state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Phase3Zone {
    pub(super) canonical: Phase3Snapshot,
    pub(super) schedule: super::state_effects::Phase3Schedule,
    /// Derived effective sheets; rebuilt on modifier/class/level changes, never persisted.
    pub(super) effective:
        BTreeMap<super::EntityId, std::sync::Arc<super::effective_stats::EffectiveStats>>,
    pub(super) schedule_dirty: BTreeSet<super::EntityId>,
    pub(super) vitals_dirty: std::collections::BTreeSet<super::EntityId>,
    pub(super) ai_requests: Vec<AiRequest>,
    pub(super) forced_this_tick: BTreeSet<super::EntityId>,
    pub(super) landed_hits: BTreeSet<(super::EntityId, u32)>,
}

/// Tick-local emission order, never persisted. The selected life is captured when hate
/// changes, not recomputed from a later ledger while phase nine drains the requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AiRequest {
    pub(super) npc: super::effects::TargetBinding,
    pub(super) target: Option<super::effects::TargetBinding>,
    pub(super) force: bool,
}

impl Phase3Zone {
    pub(super) fn from_canonical(
        canonical: Phase3Snapshot,
    ) -> Result<Self, super::effects::EffectError> {
        let schedule = super::state_effects::Phase3Schedule::rebuild(&canonical.entities)?;
        Ok(Self {
            canonical,
            schedule,
            effective: BTreeMap::new(),
            schedule_dirty: BTreeSet::new(),
            vitals_dirty: std::collections::BTreeSet::new(),
            ai_requests: Vec::new(),
            forced_this_tick: BTreeSet::new(),
            landed_hits: BTreeSet::new(),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use crate::application::replay_log::{decode_snapshot, encode_snapshot};
    use crate::domain::zone::*;
    use crate::infrastructure::{
        class_data::{load_classes, ClassSource},
        rules_data::{load_rules, RulesSource},
    };

    fn state() -> ZoneState {
        ZoneState::new_phase3(
            ZoneSeed {
                zone: ZoneId(1),
                epoch: 1,
            },
            ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
            0,
            load_rules(&RulesSource::embedded()).unwrap().rules,
            load_classes(&ClassSource::embedded()).unwrap().registry,
            crate::infrastructure::skill_data::load_phase3_rules(
                &crate::infrastructure::skill_data::Phase3DataSource::embedded(),
                &[],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn phase3_version_matrix_and_envelopes() {
        let valid = state().snapshot();
        for schema in 4..=9 {
            for digest in [
                StateDigestVersion::JsonV1,
                StateDigestVersion::BinaryV2,
                StateDigestVersion::BinaryV3,
                StateDigestVersion::BinaryV4,
                StateDigestVersion::BinaryV5,
            ] {
                for phase3 in [false, true] {
                    let mut candidate = valid.clone();
                    candidate.meta.schema_version = schema;
                    candidate.meta.digest_version = digest;
                    if !phase3 {
                        candidate.phase3 = None;
                        candidate.meta.phase3_admission_policy = None;
                    }
                    let accepted = matches!(
                        (schema, digest, phase3),
                        (7, StateDigestVersion::BinaryV3, false)
                            | (8, StateDigestVersion::BinaryV4, false)
                            | (9, StateDigestVersion::BinaryV5, true)
                    );
                    assert_eq!(ZoneState::from_snapshot(candidate.clone()).is_ok(), accepted);
                    assert_eq!(encode_snapshot(&candidate).is_ok(), accepted);
                }
            }
        }
        let bytes = encode_snapshot(&valid).unwrap();
        assert_eq!(&bytes[..8], b"NFSNAP9\0");
        assert_eq!(decode_snapshot(&bytes).unwrap(), valid);
        for magic in *b"78" {
            let mut wrong = bytes.clone();
            wrong[6] = magic;
            assert!(decode_snapshot(&wrong).is_err());
        }
        let mut missing = valid.clone();
        missing.rules = None;
        assert!(ZoneState::from_snapshot(missing).is_err());
        let mut corrupt = valid;
        corrupt.phase3.as_mut().unwrap().rules_hash[0] ^= 1;
        assert!(ZoneState::from_snapshot(corrupt).is_err());
    }

    #[test]
    fn each_canonical_counter_changes_v5_digest() {
        let zone = state();
        let snapshot = zone.snapshot();
        for i in 0..5 {
            let mut changed = snapshot.clone();
            let p = changed.phase3.as_mut().unwrap();
            let field = match i {
                0 => &mut p.next_entity_ordinal,
                1 => &mut p.next_cast_id,
                2 => &mut p.next_effect_id,
                3 => &mut p.next_offer_id,
                _ => &mut p.next_install_sequence,
            };
            *field = 2;
            assert_ne!(
                ZoneState::from_snapshot(changed).unwrap().state_digest(),
                zone.state_digest()
            );
        }
        let mut legacy = snapshot;
        legacy.phase3 = None;
        legacy.meta.phase3_admission_policy = None;
        legacy.meta.schema_version = 8;
        legacy.meta.digest_version = StateDigestVersion::BinaryV4;
        assert_ne!(ZoneState::from_snapshot(legacy).unwrap().state_digest(), zone.state_digest());
    }

    #[test]
    fn legacy_snapshots_reject_recursive_presence_even_null_or_empty() {
        let zone = state();
        let mut snapshot = zone.snapshot();
        snapshot.phase3 = None;
        snapshot.meta.phase3_admission_policy = None;
        for schema in 4..=8 {
            snapshot.meta.schema_version = schema;
            snapshot.meta.digest_version = StateDigestVersion::JsonV1;
            snapshot.classes = None;
            snapshot.meta.classes_rules_hash = None;
            for extension in [serde_json::Value::Null, serde_json::json!({})] {
                for path in [
                    vec!["phase3"],
                    vec!["meta", "phase3"],
                    vec!["meta", "phase3_admission_policy"],
                    vec!["checkpoints", "latest", "phase3"],
                    vec!["spawn_slots", "profile", "phase3"],
                    vec!["entities", "combat", "progression", "phase3"],
                ] {
                    let mut json = serde_json::to_value(&snapshot).unwrap();
                    let mut node = &mut json;
                    for key in &path[..path.len() - 1] {
                        if !node[key].is_object() {
                            node[key] = serde_json::json!({});
                        }
                        node = &mut node[key];
                    }
                    node[path[path.len() - 1]] = extension.clone();
                    let error = decode_snapshot(&serde_json::to_vec(&json).unwrap()).unwrap_err();
                    assert!(error.0.contains("Phase 3 field"), "{error}");
                }
            }
        }
    }

    #[test]
    fn boundary_records_unintegrated_policy_without_gameplay_mutation() {
        let mut zone = state();
        let before = zone.snapshot();
        let draft = zone.draft(vec![ZoneInput::system(ZoneCommand::Attack {
            entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
        })]);
        let applied = zone.run_tick(draft).unwrap();
        assert_eq!(applied.dispositions.len(), 1);
        assert_eq!(applied.dispositions[0].reason, RejectReason::NotYetImplemented);
        let after = zone.snapshot();
        assert_eq!(after.entities, before.entities);
        assert_eq!(after.phase3, before.phase3);
        assert_eq!(after.rng, before.rng);
        assert_eq!(after.tick, before.tick.next());
    }
    #[test]
    fn admission_policy_is_required_validated_and_digest_bound() {
        let state = state();
        let snapshot = state.snapshot();
        assert_eq!(state.phase3_admission_policy(), Some(&Phase3AdmissionPolicy::version_one()));
        let mut absent = snapshot.clone();
        absent.meta.phase3_admission_policy = None;
        assert!(ZoneState::from_snapshot(absent.clone()).is_err());
        assert!(encode_snapshot(&absent).is_err());
        for field in 0..4 {
            let mut bad = snapshot.clone();
            let policy = bad.meta.phase3_admission_policy.as_mut().unwrap();
            match field {
                0 => policy.version = 2,
                1 => policy.record_budget = policy.max_command_bytes,
                2 => policy.max_command_bytes = 262_145,
                _ => policy.max_observers = 0,
            }
            assert!(ZoneState::from_snapshot(bad).is_err());
        }
        for field in 0..3 {
            let mut changed = snapshot.clone();
            let policy = changed.meta.phase3_admission_policy.as_mut().unwrap();
            match field {
                0 => policy.record_budget -= 1,
                1 => policy.max_command_bytes -= 1,
                _ => policy.max_observers -= 1,
            }
            assert_ne!(
                ZoneState::from_snapshot(changed).unwrap().state_digest(),
                state.state_digest()
            );
        }
        let mut legacy = snapshot;
        legacy.phase3 = None;
        legacy.meta.schema_version = 8;
        legacy.meta.digest_version = StateDigestVersion::BinaryV4;
        assert!(
            ZoneState::from_snapshot(legacy).is_err(),
            "legacy cannot carry V5 admission inputs"
        );
    }
}
