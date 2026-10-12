//! Explicit V6 bootstrap records the hash-bound noncombat blacksmith and allocator.
#![allow(missing_docs, clippy::unwrap_used)]
use nightfall_api::application::ports::ItemAdmissionRepository;
use nightfall_api::application::replay_log::{restore_snapshot, EventLog, WatermarkReason};
use nightfall_api::application::zone_actor::manual_ticks;
use nightfall_api::application::zone_bootstrap::ZoneBootstrap;
use nightfall_api::domain::zone::{StateDigestVersion, Vec2Fixed};
use nightfall_api::infrastructure::eventlog::InMemoryEventLog;
use nightfall_api::infrastructure::memory::{InMemoryCharacterRepository, ManualClock};
use nightfall_api::infrastructure::telemetry::Metrics;
use nightfall_api::infrastructure::{class_data, item_data, rules_data, zone_data};
use std::sync::Arc;

#[tokio::test]
async fn explicit_v6_bootstrap_reserves_range_and_records_source_blacksmith() {
    let bundle = item_data::embedded_phase4_bundle().unwrap();
    let rules = rules_data::load_rules(&rules_data::RulesSource::embedded()).unwrap();
    let classes = class_data::load_classes(&class_data::ClassSource::embedded()).unwrap();
    let definition = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
    let reservations = Arc::new(InMemoryCharacterRepository::default());
    let log = Arc::new(InMemoryEventLog::default());
    let bootstrap = ZoneBootstrap::new(
        log.clone(),
        None,
        Arc::new(ManualClock::default()),
        Arc::new(Metrics::detached()),
    )
    .with_rules(rules)
    .with_classes(classes.registry, classes.config_hash)
    .with_phase3_rules((*bundle.skills).clone())
    .with_phase4_rules(bundle.items.clone(), reservations.clone());
    let (ticks, driver) = manual_ticks();
    let running = bootstrap.start(&definition, ticks).await.unwrap();
    let recorded = log
        .read_snapshot(running.zone(), running.epoch())
        .await
        .unwrap()
        .unwrap()
        .snapshot;
    assert_eq!(recorded.meta.digest_version, StateDigestVersion::BinaryV6);
    let range = reservations
        .reserve_item_range(running.zone(), running.epoch())
        .await
        .unwrap();
    assert_eq!(
        recorded.phase4.as_ref().unwrap().allocator.bounds(),
        (range.range_start, range.next_reserved, range.range_end_exclusive)
    );
    let mut received = running.handle().subscribe();
    driver.step().await.unwrap();
    let applied = received.recv().await.unwrap();
    let mut cold = restore_snapshot(recorded).unwrap();
    assert_eq!(
        *applied,
        cold.run_tick(nightfall_api::domain::zone::AppliedTickDraft {
            epoch: applied.epoch,
            tick: applied.tick,
            commands: applied.commands.clone(),
        })
        .unwrap()
    );
    let snapshot = running.handle().snapshot().await.unwrap();
    let rule = &bundle.items.parts().capacities.blacksmith;
    let smiths: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|npc| npc.name == rule.template)
        .collect();
    assert_eq!(smiths.len(), 1);
    assert_eq!(smiths.first().unwrap().pos, Vec2Fixed::from_tiles(rule.x, rule.y));
    assert!(smiths.first().unwrap().combat.is_none());
    assert!(applied.dispositions.is_empty());
    let epoch = running.epoch();
    running.shutdown(WatermarkReason::Shutdown).await.unwrap();
    let mut mismatched = definition.clone();
    mismatched
        .npcs
        .push(nightfall_api::application::zone_bootstrap::NpcSpawn {
            name: rule.template.clone(),
            pos: Vec2Fixed::from_tiles(0, 0),
            speed: nightfall_api::domain::zone::Speed::from_milli_tiles_per_tick(0),
        });
    let (ticks, _driver) = manual_ticks();
    assert!(bootstrap.start(&mismatched, ticks).await.is_err());
    assert_eq!(log.latest_epoch(definition.zone).await.unwrap(), Some(epoch));
}
