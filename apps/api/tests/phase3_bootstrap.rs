//! Production startup validates the complete immutable bundle before broker work.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]

use api::application::replay_log::{decode_snapshot, encode_snapshot};
use api::domain::zone::{StateDigestVersion, ZoneSeed};
use api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::{load_rules, RulesSource},
    skill_data::Phase3DataSource,
    zone_data::{parse_zone, TEST_ZONE_TOML},
};
use api::zone_runtime::{prepare_phase3_bootstrap, PreparedPhase3Bootstrap, ZoneRuntimeConfig};
use nightfall_api as api;

fn prepared(source: &Phase3DataSource) -> anyhow::Result<PreparedPhase3Bootstrap> {
    prepare_phase3_bootstrap(
        source,
        load_rules(&RulesSource::embedded())?,
        load_classes(&ClassSource::embedded())?,
        parse_zone(TEST_ZONE_TOML)?,
    )
}

#[test]
fn complete_embedded_and_explicit_bundles_build_equivalent_v5_states_with_provenance() {
    let data_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/data");
    let source = Phase3DataSource::from_dir(&data_root).unwrap();
    let embedded = prepared(&Phase3DataSource::embedded()).unwrap();
    let explicit = prepare_phase3_bootstrap(
        &source,
        load_rules(&RulesSource::from_dir(&data_root).unwrap()).unwrap(),
        load_classes(&ClassSource::from_dir(&data_root).unwrap()).unwrap(),
        parse_zone(TEST_ZONE_TOML).unwrap(),
    )
    .unwrap();
    assert_eq!(explicit.phase3.hash(), embedded.phase3.hash());
    assert_eq!(explicit.physical.config_hash, embedded.physical.config_hash);
    assert_eq!(explicit.classes.config_hash, embedded.classes.config_hash);
    let seed = ZoneSeed {
        zone: embedded.zone.zone,
        epoch: 44,
    };
    let mut state = embedded.state(seed, 700).unwrap();
    assert_eq!(state.snapshot(), explicit.state(seed, 700).unwrap().snapshot());
    let snapshot = state.snapshot();
    assert_eq!(snapshot.meta.schema_version, 9);
    assert_eq!(snapshot.meta.digest_version, StateDigestVersion::BinaryV5);
    assert_eq!(snapshot.meta.classes_hash, embedded.classes.config_hash);
    assert_eq!(snapshot.meta.rules_hash, embedded.physical.config_hash);
    assert_eq!(snapshot.meta.config_hash, embedded.zone.config_hash);
    assert_eq!(snapshot.phase3.as_ref().unwrap().rules.hash(), embedded.phase3.hash());
    assert_eq!(decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap(), snapshot);
    let applied = state.run_tick(state.draft(vec![])).unwrap();
    assert_eq!(applied.digest_version, StateDigestVersion::BinaryV5);
    assert!(applied.dispositions.is_empty());
    assert_eq!(state.entity_count(), 3);
}

#[test]
fn invalid_or_incomplete_policy_profiles_skills_and_geometry_are_refused() {
    let valid = Phase3DataSource::embedded();
    let mut bad = valid.clone();
    bad.profiles
        .classes
        .remove("classes-phase3/human_fighter.toml");
    assert!(prepared(&bad).is_err());
    bad = valid.clone();
    bad.skills.0.clear();
    assert!(prepared(&bad).is_err());
    bad = valid.clone();
    bad.pvp.clear();
    assert!(prepared(&bad).is_err());
    bad = valid.clone();
    bad.regen.clear();
    assert!(prepared(&bad).is_err());
    bad = valid.clone();
    bad.formulas.clear();
    assert!(prepared(&bad).is_err());
    bad = valid;
    bad.geometry.clear();
    assert!(prepared(&bad).is_err());
    assert!(Phase3DataSource::from_dir(std::path::Path::new(
        "/tmp/nightfall-phase3-api-missing-bundle"
    ))
    .is_err());
}

#[test]
fn configured_zone_cannot_disagree_with_hashed_npc_profiles() {
    let mut zone = parse_zone(TEST_ZONE_TOML).unwrap();
    zone.npc_templates[0].max_hp = 1;
    let failure = prepare_phase3_bootstrap(
        &Phase3DataSource::embedded(),
        load_rules(&RulesSource::embedded()).unwrap(),
        load_classes(&ClassSource::embedded()).unwrap(),
        zone,
    )
    .unwrap_err();
    assert!(failure.to_string().contains("disagree with NPC template"));
}

#[tokio::test]
async fn bad_switches_refuse_before_broker_or_epoch_work() {
    let dependencies = api::Dependencies::in_memory();
    for value in ["", "true", "01", "1 ", "yes", "2"] {
        let cfg = ZoneRuntimeConfig {
            phase3_enabled: Some(value.into()),
            ..Default::default()
        };
        let failure = api::zone_runtime::start(&cfg, Some("not-a-nats-url"), None, &dependencies)
            .await
            .err()
            .unwrap();
        assert!(failure
            .to_string()
            .contains("PHASE3_ENABLED must be unset, 0 or 1"));
    }
}

#[tokio::test]
async fn off_switch_preserves_legacy_bootstrap_and_snapshot_policy() {
    for selection in [Some("0".into())] {
        let cfg = ZoneRuntimeConfig {
            phase3_enabled: selection,
            ..Default::default()
        };
        let running = api::zone_runtime::start(&cfg, None, None, &api::Dependencies::in_memory())
            .await
            .unwrap();
        let snapshot = running.handle().snapshot().await.unwrap();
        assert!(!running.handle().phase3_enabled());
        assert_eq!(snapshot.meta.digest_version, StateDigestVersion::BinaryV4);
        assert!(snapshot.phase3.is_none());
        assert!(snapshot.meta.phase3_admission_policy.is_none());
        running
            .shutdown(api::application::replay_log::WatermarkReason::Shutdown)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn normal_default_and_explicit_enable_write_v5_baselines() {
    for selection in [None, Some("1".into())] {
        let cfg = ZoneRuntimeConfig {
            phase3_enabled: selection,
            ..Default::default()
        };
        let running = api::zone_runtime::start(&cfg, None, None, &api::Dependencies::in_memory())
            .await
            .unwrap();
        let snapshot = running.handle().snapshot().await.unwrap();
        assert!(running.handle().phase3_enabled());
        assert_eq!(snapshot.meta.schema_version, 9);
        assert_eq!(snapshot.meta.digest_version, StateDigestVersion::BinaryV5);
        assert_eq!(snapshot.phase3.unwrap().rules.runtime_revision(), 2);
        assert!(snapshot.meta.phase3_admission_policy.is_some());
        running
            .shutdown(api::application::replay_log::WatermarkReason::Shutdown)
            .await
            .unwrap();
    }
}
