//! Story 2.2: the V7 version boundary (Snapshot 11 / Record 9 / `BinaryV7`, design §12.1)
//! fails closed until the Phase 5 block exists, and the recorded version always wins over
//! the `PHASE5_ENABLED` selection on replay.
#![allow(clippy::unwrap_used)]
use nightfall_api::{
    application::replay_log::{
        decode_snapshot, encode_snapshot, restore_snapshot, AppliedTickRecord,
        RECORD_SCHEMA_VERSION,
    },
    domain::zone::*,
    infrastructure::{class_data, item_data, rules_data},
    zone_runtime::ZoneRuntimeConfig,
};

fn v6_zone() -> ZoneState {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    ZoneState::new_phase4(
        ZoneSeed {
            zone: ZoneId(75),
            epoch: 5,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules_data::load_rules(&rules_data::RulesSource::embedded())
            .unwrap()
            .rules,
        class_data::load_classes(&class_data::ClassSource::embedded())
            .unwrap()
            .registry,
        (*bundle.skills).clone(),
        bundle.items,
        ItemInstanceRange::new(1, 1, 100).unwrap(),
    )
    .unwrap()
}

#[test]
fn v7_constants_are_allocated_once() {
    assert_eq!(SNAPSHOT_SCHEMA_VERSION, 11);
    assert_eq!(RECORD_SCHEMA_VERSION, 9);
    assert!(Phase3AdmissionPolicy::version_three().is_valid());
    assert_eq!(Phase3AdmissionPolicy::version_three().version, 3);
    assert_eq!(
        Phase3AdmissionPolicy::version_three().record_budget,
        Phase3AdmissionPolicy::version_two().record_budget
    );
    let mut zone = v6_zone();
    let applied = zone.run_tick(zone.draft(Vec::new())).unwrap();
    let mut record =
        AppliedTickRecord::decode(&AppliedTickRecord::from_applied(ZoneId(75), &applied).encode())
            .unwrap();
    assert_eq!(record.digest_version, StateDigestVersion::BinaryV6);
    // A V7 record is Record 9 and round-trips; its digest version is `BinaryV7`.
    record.digest_version = StateDigestVersion::BinaryV7;
    let bytes = record.try_encode().unwrap();
    let decoded = AppliedTickRecord::decode(&bytes).unwrap();
    assert_eq!(decoded.digest_version, StateDigestVersion::BinaryV7);
    assert_eq!(decoded.try_encode().unwrap(), bytes);
}

#[test]
fn a_v6_snapshot_relabelled_as_v7_fails_closed_everywhere() {
    let zone = v6_zone();
    let snapshot = zone.snapshot();
    assert!(!zone.phase5_enabled());
    assert!(zone.phase4_enabled());
    let mut relabelled = snapshot.clone();
    relabelled.meta.schema_version = 11;
    relabelled.meta.digest_version = StateDigestVersion::BinaryV7;
    relabelled.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy::version_three());
    // Without the Phase 5 block (story 3.1) no V7 snapshot validates, encodes or restores.
    assert!(relabelled.validate_version().is_err());
    assert!(encode_snapshot(&relabelled).is_err());
    assert!(restore_snapshot(relabelled.clone()).is_err());
    assert!(ZoneState::from_snapshot(relabelled).is_err());
    // Mixed labels are refused too: schema 11 with the V6 digest, and vice versa.
    let mut mixed = snapshot.clone();
    mixed.meta.schema_version = 11;
    assert!(mixed.validate_version().is_err());
    let mut mixed = snapshot.clone();
    mixed.meta.digest_version = StateDigestVersion::BinaryV7;
    assert!(mixed.validate_version().is_err());
    // The V7 envelope magic on a V6 body is a schema mismatch, not a silent upgrade.
    let bytes = encode_snapshot(&snapshot).unwrap();
    assert!(bytes.starts_with(b"NFSNAP10"));
    let mut forged = b"NFSNAP11".to_vec();
    forged.extend_from_slice(bytes.get(8..).unwrap());
    assert!(decode_snapshot(&forged).is_err());
    assert_eq!(decode_snapshot(&bytes).unwrap(), snapshot);
}

#[test]
fn legacy_snapshot_json_refuses_v7_names_recursively() {
    // Legacy (schema < 10) JSON envelopes run the Phase 4 and Phase 5 scans.
    let mut legacy = ZoneState::new(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(64, 64)).unwrap(),
        0,
    )
    .snapshot();
    legacy.meta.schema_version = 6;
    legacy.meta.digest_version = StateDigestVersion::BinaryV2;
    let bytes = encode_snapshot(&legacy).unwrap();
    assert!(decode_snapshot(&bytes).is_ok());
    let text = String::from_utf8(bytes).unwrap();
    for (needle, injection) in [
        ("\"aoi\":[]", "\"aoi\":[{\"phase5\":null}]"),
        ("\"aoi\":[]", "\"aoi\":[{\"location\":\"warehouse\"}]"),
        ("\"aoi\":[]", "\"aoi\":[{\"type\":\"economy\"}]"),
        ("\"aoi\":[]", "\"aoi\":[{\"kind\":\"ground_item\"}]"),
        ("\"aoi\":[]", "\"aoi\":[{\"op\":\"party_split\"}]"),
    ] {
        assert!(text.contains(needle));
        let forged = text.replacen(needle, injection, 1);
        let error = decode_snapshot(forged.as_bytes()).unwrap_err().to_string();
        assert!(error.contains("V7"), "{injection}: {error}");
    }
}

#[test]
fn the_recorded_version_wins_over_the_phase5_selection() {
    // Every PHASE5_ENABLED selection replays a recorded V6 epoch as V6: restoration takes
    // only the snapshot, never the runtime selection.
    let snapshot = v6_zone().snapshot();
    for selection in [None, Some("0"), Some("1")] {
        let config = ZoneRuntimeConfig {
            phase5_enabled: selection.map(str::to_owned),
            ..ZoneRuntimeConfig::default()
        };
        assert_eq!(config.phase5_requested().unwrap(), selection != Some("0"));
        let restored = restore_snapshot(snapshot.clone()).unwrap();
        assert!(!restored.phase5_enabled());
        assert_eq!(restored.snapshot().meta.schema_version, 10);
        assert_eq!(restored.snapshot().meta.digest_version, StateDigestVersion::BinaryV6);
        assert_eq!(
            restored.state_digest(),
            restore_snapshot(snapshot.clone()).unwrap().state_digest()
        );
    }
    assert!(ZoneRuntimeConfig {
        phase5_enabled: Some("yes".into()),
        ..ZoneRuntimeConfig::default()
    }
    .validate_production_selection()
    .is_err());
}
