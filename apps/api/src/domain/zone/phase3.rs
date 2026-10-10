//! Phase 3 epoch boundary. This sidecar is never synthesized while restoring legacy data.
//!
//! The initial boundary commit reserves canonical allocation counters and immutable policy.
//! Gameplay construction remains fenced until registry and entity validation are installed.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Canonical Phase 3 policy, separately hashed from the historical stat/class registries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase3Rules {
    /// Version of the paused, remaining-tick admission policy (design §5.2).
    pub timer_policy_version: u32,
}

impl Phase3Rules {
    /// Hash using an explicit binary layout; never hashes a process-local representation.
    pub fn hash(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"nightfall.phase3.rules.1\0");
        hash.update(self.timer_policy_version.to_le_bytes());
        hash.finalize().into()
    }
}

/// Canonical Phase 3 snapshot block. Legacy snapshots must omit the field entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase3Snapshot {
    /// Complete immutable rules for this epoch, never fetched by replay.
    pub rules: Phase3Rules,
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
}

impl Phase3Snapshot {
    /// New empty epoch. All identities start at one; zero is never a live identity.
    pub fn new(rules: Phase3Rules) -> Self {
        Self {
            rules_hash: rules.hash(),
            rules,
            next_entity_ordinal: 1,
            next_cast_id: 1,
            next_effect_id: 1,
            next_offer_id: 1,
            next_install_sequence: 1,
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
    }
}

/// Runtime sidecar; derived indexes will be rebuilt from its canonical snapshot state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Phase3Zone {
    pub(super) canonical: Phase3Snapshot,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
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
            Phase3Rules {
                timer_policy_version: 1,
            },
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
        legacy.meta.schema_version = 8;
        legacy.meta.digest_version = StateDigestVersion::BinaryV4;
        assert_ne!(ZoneState::from_snapshot(legacy).unwrap().state_digest(), zone.state_digest());
    }

    #[test]
    fn legacy_snapshots_reject_recursive_presence_even_null_or_empty() {
        let zone = state();
        let mut snapshot = zone.snapshot();
        snapshot.phase3 = None;
        for schema in 4..=8 {
            snapshot.meta.schema_version = schema;
            snapshot.meta.digest_version = StateDigestVersion::JsonV1;
            snapshot.classes = None;
            snapshot.meta.classes_rules_hash = None;
            for extension in [serde_json::Value::Null, serde_json::json!({})] {
                for path in [
                    vec!["phase3"],
                    vec!["meta", "phase3"],
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
    fn boundary_does_not_silently_run_legacy_gameplay() {
        let mut zone = state();
        let before = zone.snapshot();
        let draft = zone.draft(Vec::new());
        assert_eq!(zone.run_tick(draft), Err(TickError::Phase3NotReady));
        assert_eq!(zone.snapshot(), before);
    }
}
