//! V6 boundary acceptance with complete generated, compiled rule bundles.
#![allow(clippy::unwrap_used)]
use nightfall_api::{
    application::replay_log::{
        decode_snapshot, encode_snapshot, restore_snapshot, AppliedTickRecord,
    },
    domain::zone::*,
    infrastructure::{class_data, item_data, rules_data},
};

fn zone() -> ZoneState {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    ZoneState::new_phase4(
        ZoneSeed {
            zone: ZoneId(74),
            epoch: 4,
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
fn v6_hash_only_snapshot_record_and_cold_continuation() {
    let mut live = zone();
    let snapshot = live.snapshot();
    assert_eq!(snapshot.meta.schema_version, 10);
    assert!(live.phase4_enabled());
    assert!(ZoneState::from_snapshot(snapshot.clone()).is_err());
    let bytes = encode_snapshot(&snapshot).unwrap();
    assert!(bytes.starts_with(b"NFSNAP10"));
    // Inspect the payload directly: compression size alone cannot prove that static
    // rule tables were omitted. encode_snapshot enforces the existing broker budget.
    {
        use prost::Message as _;
        use std::io::Read as _;
        #[derive(prost::Message)]
        struct Header {
            #[prost(uint32, tag = "1")]
            format: u32,
            #[prost(bytes = "vec", tag = "2")]
            base: Vec<u8>,
            #[prost(bytes = "vec", optional, tag = "4")]
            rules: Option<Vec<u8>>,
        }
        let mut expanded = Vec::new();
        flate2::read::ZlibDecoder::new(bytes.get(12..).unwrap())
            .read_to_end(&mut expanded)
            .unwrap();
        let header = Header::decode(expanded.as_slice()).unwrap();
        assert_eq!(header.format, 2);
        assert!(header.rules.is_none());
        let base: serde_json::Value = serde_json::from_slice(&header.base).unwrap();
        assert!(base.get("phase3").is_none());
        assert!(base.get("phase4").unwrap().get("rules").is_none());
    }
    let decoded = decode_snapshot(&bytes).unwrap();
    assert_eq!(snapshot, decoded);
    assert_eq!(encode_snapshot(&decoded).unwrap(), bytes);
    let mut cold = restore_snapshot(decoded).unwrap();
    assert_eq!(live.state_digest(), cold.state_digest());
    for _ in 0..3 {
        let a = live.run_tick(live.draft(Vec::new())).unwrap();
        let b = cold.run_tick(cold.draft(Vec::new())).unwrap();
        let record = AppliedTickRecord::from_applied(ZoneId(74), &a);
        assert_eq!(record.digest_version, StateDigestVersion::BinaryV6);
        assert_eq!(record.encode(), AppliedTickRecord::from_applied(ZoneId(74), &b).encode());
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    }
    assert!(decode_snapshot(&serde_json::to_vec(&snapshot).unwrap()).is_err());
}

#[test]
fn v6_missing_sidecars_policy_and_hash_substitution_are_refused() {
    let snapshot = zone().snapshot();
    for field in 0..7 {
        let mut bad = snapshot.clone();
        match field {
            0 => bad.phase4 = None,
            1 => bad.phase3 = None,
            2 => bad.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy::version_one()),
            3 => bad.phase4.as_mut().unwrap().rules_hash[0] ^= 1,
            4 => bad.phase3.as_mut().unwrap().rules_hash[0] ^= 1,
            5 => bad.meta.schema_version = 9,
            _ => bad.meta.digest_version = StateDigestVersion::BinaryV5,
        }
        assert!(restore_snapshot(bad).is_err(), "field {field}");
    }
    let mut changed = snapshot.clone();
    changed.phase4.as_mut().unwrap().allocator = ItemInstanceRange::new(1, 2, 100).unwrap();
    assert_ne!(
        restore_snapshot(snapshot).unwrap().state_digest(),
        restore_snapshot(changed).unwrap().state_digest()
    );
}
