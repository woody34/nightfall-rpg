//! Selection and behavioral regression coverage for the simulation-only runtime fixture.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use uuid::Uuid;

use super::*;
use crate::application::zone_bootstrap::slot_specs;
use crate::domain::zone::{
    AppliedTick, EntityId, EntityKind, HateEntry, Intention, PlayerLoad, SessionGeneration,
    Vec2Fixed, ZoneCommand, ZoneEvent, ZoneInput, ZoneSeed, ZoneState,
};
use crate::interface::grpc::pb::{server_message::Payload, world_event::Event};
use crate::interface::zone_mapping::observer_output_to_pb;

fn config(values: &[(&str, &str)]) -> ZoneRuntimeConfig {
    ZoneRuntimeConfig::read_env(|name| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).into())
    })
}

#[test]
fn simulation_selection_requires_exact_dev_auth_and_rejects_bad_or_ambiguous_names() {
    for auth in [
        None,
        Some(""),
        Some("0"),
        Some("true"),
        Some("01"),
        Some("1 "),
    ] {
        let mut values = vec![("ZONE_SIM_FIXTURE", "phase1a-social-aggro")];
        if let Some(auth) = auth {
            values.push(("AUTH_DEV_TOKENS", auth));
        }
        let error = config(&values).load_zone().unwrap_err().to_string();
        assert!(error.contains("requires AUTH_DEV_TOKENS=1"), "{error}");
    }
    for name in ["", "unknown", "phase1a-social-aggro "] {
        assert!(config(&[("AUTH_DEV_TOKENS", "1"), ("ZONE_SIM_FIXTURE", name)])
            .load_zone()
            .unwrap_err()
            .to_string()
            .contains("unknown ZONE_SIM_FIXTURE"));
    }
    assert!(config(&[
        ("AUTH_DEV_TOKENS", "1"),
        ("ZONE_SIM_FIXTURE", "phase1a-social-aggro"),
        ("ZONE_FILE", "does-not-exist.toml"),
    ])
    .load_zone()
    .unwrap_err()
    .to_string()
    .contains("conflicts with ZONE_FILE"));
    let selected = config(&[
        ("AUTH_DEV_TOKENS", "1"),
        ("ZONE_SIM_FIXTURE", "phase1a-social-aggro"),
    ])
    .load_zone()
    .unwrap();
    assert_eq!(selected, social_aggro_fixture().unwrap());
    assert_ne!(
        selected.config_hash,
        zone_data::parse_zone(zone_data::TEST_ZONE_TOML)
            .unwrap()
            .config_hash
    );
}

#[test]
fn dev_auth_alone_and_absent_selection_preserve_the_default_zone() {
    let expected = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
    assert_eq!(config(&[]).load_zone().unwrap(), expected);
    assert_eq!(config(&[("AUTH_DEV_TOKENS", "1")]).load_zone().unwrap(), expected);
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/data/zones/test_zone.toml");
    assert_eq!(
        config(&[("ZONE_FILE", path.to_str().unwrap())])
            .load_zone()
            .unwrap(),
        expected
    );
}

#[tokio::test]
async fn forbidden_fixture_aborts_runtime_before_connecting_any_adapters() {
    let cfg = config(&[("ZONE_SIM_FIXTURE", "phase1a-social-aggro")]);
    let result = start(&cfg, Some("not-a-nats-url"), None, &crate::Dependencies::in_memory()).await;
    assert!(result
        .err()
        .unwrap()
        .to_string()
        .contains("requires AUTH_DEV_TOKENS=1"));
}

#[tokio::test]
async fn selected_fixture_bootstraps_real_ai_slots_and_snapshot_provenance() {
    let cfg = config(&[
        ("AUTH_DEV_TOKENS", "1"),
        ("ZONE_SIM_FIXTURE", "phase1a-social-aggro"),
    ]);
    let running = start(&cfg, None, None, &crate::Dependencies::in_memory())
        .await
        .unwrap();
    // A boundary can precede tick zero; wait for the spawn phase to be admitted first.
    let mut stats = running.handle().stats();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stats.wait_for(|stats| stats.entities == 3),
    )
    .await
    .unwrap()
    .unwrap();
    let snapshot = running.handle().snapshot().await.unwrap();
    assert_eq!(snapshot.seed.zone, crate::domain::zone::ZoneId(1));
    assert_eq!(snapshot.meta.config_hash, cfg.load_zone().unwrap().config_hash);
    assert_eq!(snapshot.entities.len(), 3);
    assert_eq!(snapshot.spawn_slots.len(), 3);
    for entity in &snapshot.entities {
        assert_eq!(entity.kind, EntityKind::Npc);
        assert_eq!(entity.speed.milli_tiles_per_tick(), 0);
        assert_eq!(entity.pos, entity.ai.as_ref().unwrap().home);
        assert_eq!(entity.combat.as_ref().unwrap().hp, 1_000_000);
    }
    for spec in &snapshot.spawn_slots {
        assert!(!spec.brain.aggressive);
        assert!(spec
            .home
            .within(Vec2Fixed::from_tiles(100, 100), spec.brain.clan_help_range));
        assert!(spec
            .home
            .within(Vec2Fixed::from_tiles(99, 100), spec.combat.attack_range));
        if spec.id != "caller" {
            assert!(
                spec.home.distance_sq(Vec2Fixed::from_tiles(104, 102))
                    < spec.home.distance_sq(Vec2Fixed::from_tiles(99, 100))
            );
        }
    }
    running
        .shutdown(crate::application::replay_log::WatermarkReason::Shutdown)
        .await
        .unwrap();
}

#[cfg(unix)]
#[test]
fn non_unicode_fixture_selection_is_refused_instead_of_using_default_content() {
    use std::os::unix::ffi::OsStringExt as _;
    let cfg = ZoneRuntimeConfig::read_env(|name| match name {
        "ZONE_SIM_FIXTURE" => Some(std::ffi::OsString::from_vec(vec![0xff])),
        "AUTH_DEV_TOKENS" => Some("1".into()),
        _ => None,
    });
    assert!(cfg
        .load_zone()
        .unwrap_err()
        .to_string()
        .contains("unknown ZONE_SIM_FIXTURE"));
}

fn player(n: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(n))
}

fn spawn(n: u128, x: i32, y: i32) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: player(n),
        name: format!("player{n}"),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(PlayerLoad::fresh("human_fighter"))),
    })
}

fn run(state: &mut ZoneState, inputs: Vec<ZoneInput>) -> AppliedTick {
    let tick = state.run_tick(state.draft(inputs)).unwrap();
    assert!(tick.dispositions.is_empty(), "unexpected rejection: {:?}", tick.dispositions);
    tick
}

/// Count the same real wire `AttackResult`s used by Unreal's `attacked_by`/`npcs_fighting`.
/// Restrict to the fixture NPC ids, so A's own swings cannot inflate the oracle.
fn wire_attackers(
    ticks: &[AppliedTick],
    observer: EntityId,
    target: EntityId,
    npcs: &BTreeSet<EntityId>,
) -> BTreeSet<String> {
    ticks
        .iter()
        .flat_map(|t| {
            t.outputs
                .get(&observer)
                .into_iter()
                .flatten()
                .flat_map(|o| observer_output_to_pb(o, t.server_time_ms))
        })
        .filter_map(|message| match message.payload {
            Some(Payload::Event(event)) => event.event,
            _ => None,
        })
        .filter_map(|event| {
            let Event::AttackResult(result) = event else {
                return None;
            };
            (result.target == target.to_string()
                && npcs.iter().any(|id| id.to_string() == result.attacker))
            .then_some(result.attacker)
        })
        .collect()
}

#[test]
fn only_clan_call_satisfies_the_social_scenario_wire_oracle_across_seeds() {
    let def = social_aggro_fixture().unwrap();
    let rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules;
    for epoch in [1, 7, 91] {
        // Negative controls disable the call alone, then also enable ordinary proximity aggro.
        for (clan_call, ordinary_aggro) in [(true, false), (false, false), (false, true)] {
            let mut specs = slot_specs(&rules, &def).unwrap();
            for spec in &mut specs {
                if !clan_call {
                    spec.brain.clan_help_range = crate::domain::zone::Fixed::ZERO;
                }
                spec.brain.aggressive = ordinary_aggro;
            }
            let mut state = ZoneState::new(
                ZoneSeed {
                    zone: def.zone,
                    epoch,
                },
                def.bounds,
                0,
            )
            .with_rules(rules.clone())
            .with_spawn_slots(specs);
            run(&mut state, vec![spawn(1, 99, 100), spawn(2, 104, 102)]);
            let npcs: BTreeSet<_> = state
                .entities()
                .filter(|e| e.kind == EntityKind::Npc)
                .map(|e| e.id)
                .collect();
            assert_eq!(npcs.len(), 3);
            let caller = state
                .entities()
                .find(|e| e.pos == Vec2Fixed::from_tiles(100, 100))
                .unwrap()
                .id;
            if !ordinary_aggro {
                let rng = state.snapshot().rng;
                // Longer than the entire Unreal scenario budget: no wander draws, movement,
                // spontaneous hate or attacks even with both players inside detection range.
                for _ in 0..1800 {
                    let tick = run(&mut state, Vec::new());
                    assert!(!tick.events.iter().any(|e| matches!(
                        e,
                        ZoneEvent::AttackResult { .. }
                            | ZoneEvent::HateChanged { .. }
                            | ZoneEvent::EntityMove { .. }
                    )));
                }
                assert_eq!(state.snapshot().rng, rng);
                assert!(state.snapshot().hate.is_empty());
            }
            let mut ticks = vec![run(
                &mut state,
                vec![
                    ZoneInput::session(
                        player(1),
                        SessionGeneration(1),
                        1,
                        ZoneCommand::SetTarget {
                            entity: player(1),
                            target: Some(caller),
                        },
                    ),
                    ZoneInput::session(
                        player(1),
                        SessionGeneration(1),
                        2,
                        ZoneCommand::Attack { entity: player(1) },
                    ),
                ],
            )];
            for _ in 0..150 {
                ticks.push(run(&mut state, Vec::new()));
            }
            let a_seen_by_a = wire_attackers(&ticks, player(1), player(1), &npcs);
            let a_seen_by_b = wire_attackers(&ticks, player(2), player(1), &npcs);
            let b_seen_by_b = wire_attackers(&ticks, player(2), player(2), &npcs);
            assert_eq!(a_seen_by_a, a_seen_by_b, "B must see A's real attack stream");
            let scenario_passes = a_seen_by_a.len() == 3 && b_seen_by_b.is_empty();
            assert_eq!(scenario_passes, clan_call, "epoch={epoch}, ordinary={ordinary_aggro}");
            assert_eq!(a_seen_by_a.len(), if clan_call { 3 } else { 1 });
            assert_eq!(b_seen_by_b.len(), if ordinary_aggro { 2 } else { 0 });
            for npc in &npcs {
                let entity = state.entity(*npc).unwrap();
                assert_eq!(entity.pos, entity.ai.as_ref().unwrap().home);
                assert_eq!(
                    entity.ai.as_ref().unwrap().intention,
                    if clan_call || ordinary_aggro || *npc == caller {
                        Intention::Attack
                    } else {
                        Intention::Active
                    }
                );
                if clan_call && *npc != caller {
                    assert_eq!(
                        state.hate_ledger(*npc).unwrap().get(player(1)),
                        Some(HateEntry { hate: 1, damage: 0 })
                    );
                    assert!(state.hate_ledger(*npc).unwrap().get(player(2)).is_none());
                    let calls: Vec<_> = ticks.iter().flat_map(|t| &t.events).filter(|e|
                        matches!(e, ZoneEvent::HateChanged { npc: id, target, hate: 1, damage: 0, .. }
                            if id == npc && *target == player(1))).collect();
                    assert_eq!(calls.len(), 1, "helper receives exactly one clan call");
                }
            }
            assert!(!state.entity(player(1)).unwrap().targeting.dead);
            assert!(!state.entity(player(2)).unwrap().targeting.dead);
        }
    }
}

#[tokio::test]
async fn persistent_database_without_nats_is_refused_before_adapter_or_epoch_work() {
    // A disconnected DB is intentional: the configuration guard must run before any DB
    // lookup, baseline insert, log construction, actor spawn or admission can occur.
    for nats in [None, Some(""), Some("   ")] {
        let error = start(
            &ZoneRuntimeConfig::default(),
            nats,
            Some(DatabaseConnection::default()),
            &crate::Dependencies::in_memory(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "persistent character runtime requires NATS_URL for durable replay before startup"
        );
    }
}
