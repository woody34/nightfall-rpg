#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm,
    clippy::float_cmp
)]

use super::*;
use crate::application::zone_actor::{manual_ticks, GateError, TickGate, TickOutcome, ZoneActor};
use crate::domain::zone::{
    NpcCombat, PlayerLoad, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneBounds, ZoneCommand,
    ZoneId, ZoneInput, ZoneSeed, ZoneState,
};
use std::sync::atomic::{AtomicBool, Ordering};

fn fresh() -> ZoneState {
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
    )
}

fn value(m: &Metrics, name: &str, label: &str) -> f64 {
    m.render()
        .unwrap()
        .lines()
        .find(|l| l.starts_with(name) && l.contains(label))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
#[allow(clippy::too_many_lines)] // One fixture exercises every combat fact and fan-out.
fn every_combat_fact_counts_once_and_outputs_are_not_counted() {
    let m = Metrics::detached();
    let mut state = fresh();
    let player = EntityId::from_uuid(uuid::Uuid::from_u128(1));
    let draft = state.draft(vec![
        ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: player,
            name: "private player name".into(),
            pos: Vec2Fixed::from_tiles(1, 1),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: None,
        }),
        ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "private npc name".into(),
            pos: Vec2Fixed::from_tiles(1, 1),
            speed: Speed::DEFAULT,
            combat: None,
        }),
    ]);
    let mut batch = state.run_tick(draft).unwrap();
    let npc = batch
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::EntitySpawn {
                entity,
                kind: EntityKind::Npc,
                ..
            } => Some(*entity),
            _ => None,
        })
        .unwrap();
    let mut consumer = m.consumer(&fresh().snapshot());
    consumer.admitted(&batch); // Seed from zone spawns, not session AOI output.
    batch.events.clear();
    for outcome in [AttackOutcome::Miss, AttackOutcome::Hit, AttackOutcome::Crit] {
        batch.events.push(ZoneEvent::AttackResult {
            attacker: player,
            target: npc,
            tick: Tick(1),
            outcome,
            damage: 1,
            target_hp_after: 1,
            target_incarnation: 0,
        });
    }
    for entity in [player, npc] {
        batch.events.push(ZoneEvent::EntityDied {
            entity,
            tick: Tick(1),
            killer: None,
            incarnation: 0,
        });
        batch.events.push(ZoneEvent::EntityRespawned {
            entity,
            tick: Tick(1),
            position: Vec2Fixed::from_tiles(1, 1),
            hp: 10,
            incarnation: 2,
        });
    }
    batch.events.extend([
        ZoneEvent::XpGained {
            entity: player,
            tick: Tick(1),
            amount: 37,
            total: 100,
        },
        ZoneEvent::LevelUp {
            entity: player,
            tick: Tick(1),
            level: 2,
        },
        ZoneEvent::StatsChanged {
            entity: player,
            tick: Tick(1),
            hp: 10,
            max_hp: 10,
            mp: 5,
            max_mp: 5,
            level: 2,
            xp: 100,
        },
        ZoneEvent::TargetChanged {
            entity: player,
            tick: Tick(1),
            target: Some(npc),
        },
    ]);
    let outputs = batch
        .events
        .iter()
        .cloned()
        .map(crate::domain::zone::ObserverOutput::Event)
        .collect::<Vec<_>>();
    batch.outputs.insert(player, outputs.clone());
    batch.outputs.insert(npc, outputs); // Fan-out must never multiply facts.
    consumer.admitted(&batch);
    for outcome in ["miss", "hit", "crit"] {
        assert_eq!(
            value(&m, "nightfall_combat_attacks_total{", &format!("outcome=\"{outcome}\"")),
            1.0
        );
    }
    for kind in ["npc", "player"] {
        for metric in ["deaths", "respawns"] {
            assert_eq!(
                value(
                    &m,
                    &format!("nightfall_combat_{metric}_total{{"),
                    &format!("kind=\"{kind}\"")
                ),
                1.0
            );
        }
    }
    assert_eq!(value(&m, "nightfall_combat_levelups_total", ""), 1.0);
    assert_eq!(value(&m, "nightfall_combat_xp_gained_total", ""), 37.0);
    let text = m.render().unwrap();
    assert!(!text.contains(&player.to_string()));
    assert!(!text.contains(&npc.to_string()));
    assert!(!text.contains("private"));
    // Restoring metadata must not emit metrics; despawn removes it.
    let mut restored = CombatConsumer {
        metrics: m.clone(),
        kinds: state
            .snapshot()
            .entities
            .iter()
            .map(|e| (e.id, e.kind))
            .collect(),
    };
    batch.events = vec![ZoneEvent::EntityDespawn {
        entity: npc,
        tick: Tick(2),
    }];
    restored.admitted(&batch);
    assert!(!restored.kinds.contains_key(&npc));
}

#[test]
fn intention_labels_and_checkpoint_hooks_are_bounded_and_initialized() {
    let m = Metrics::detached();
    for intention in NpcIntention::ALL {
        m.record_npc_intention_transition(intention);
    }
    let text = m.render().unwrap();
    let lines: Vec<_> = text
        .lines()
        .filter(|l| l.starts_with("nightfall_npc_intention_transitions_total{"))
        .collect();
    assert_eq!(lines.len(), 5);
    for label in ["idle", "active", "attack", "return_home", "dead"] {
        assert_eq!(
            value(&m, "nightfall_npc_intention_transitions_total{", &format!("to=\"{label}\"")),
            1.0
        );
    }
    for (name, label, count) in [
        ("attacks", "outcome", 3),
        ("deaths", "kind", 2),
        ("respawns", "kind", 2),
    ] {
        let prefix = format!("nightfall_combat_{name}_total{{");
        let lines: Vec<_> = text.lines().filter(|l| l.starts_with(&prefix)).collect();
        assert_eq!(lines.len(), count);
        assert!(lines.iter().all(|l| l.contains(&format!("{label}=\""))
            && !l.contains("entity")
            && !l.contains("account")));
    }
    assert_eq!(value(&m, "nightfall_checkpoint_lag_seconds", ""), 0.0);
    assert_eq!(value(&m, "nightfall_checkpoint_failures_total", ""), 0.0);
    m.consumer(&fresh().snapshot()).stats(TickStats {
        duration_micros: 1250,
        ..TickStats::default()
    });
    assert_eq!(value(&m, "nightfall_combat_tick_duration_seconds_count", ""), 1.0);
    assert_eq!(value(&m, "nightfall_combat_tick_duration_seconds_sum", ""), 0.00125);
    m.set_checkpoint_lag(Duration::from_secs(3));
    m.record_checkpoint_failure();
    assert_eq!(value(&m, "nightfall_checkpoint_lag_seconds", ""), 3.0);
    assert_eq!(value(&m, "nightfall_checkpoint_failures_total", ""), 1.0);
    m.set_checkpoint_lag(Duration::ZERO);
    assert_eq!(value(&m, "nightfall_checkpoint_lag_seconds", ""), 0.0);
}

struct HoldFirstAttack(AtomicBool);
impl TickGate for HoldFirstAttack {
    fn admit(
        &self,
        tick: &AppliedTick,
    ) -> impl std::future::Future<Output = Result<(), GateError>> + Send {
        if tick
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackResult { .. }))
            && !self.0.swap(true, Ordering::Relaxed)
        {
            return std::future::ready(Err(GateError("test hold".into())));
        }
        std::future::ready(Ok(()))
    }
}

fn arena(pairs: u32) -> (ZoneState, Vec<ZoneInput>) {
    use crate::infrastructure::{rules_data, zone_data};
    let rules = rules_data::load_rules(&rules_data::RulesSource::embedded())
        .unwrap()
        .rules;
    let def = zone_data::parse_zone(zone_data::TEST_ZONE_TOML).unwrap();
    let monster = NpcCombat::from_template(&rules, &def.npc_templates[0]).unwrap();
    let mut state = fresh().with_rules(rules);
    let mut inputs = Vec::new();
    for n in 0..pairs {
        let entity = EntityId::from_uuid(uuid::Uuid::from_u128(u128::from(n) + 1));
        let x = i32::try_from(n % 16 * 15 + 1).unwrap();
        let y = i32::try_from(n / 16 * 15 + 1).unwrap();
        inputs.extend([
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity,
                name: format!("p{n}"),
                pos: Vec2Fixed::from_tiles(x, y),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(PlayerLoad::fresh("human_fighter"))),
            }),
            ZoneInput::system(ZoneCommand::SpawnNpc {
                name: "Keltir".into(),
                pos: Vec2Fixed::from_tiles(x + 1, y),
                speed: Speed::DEFAULT,
                combat: Some(Box::new(monster.clone())),
            }),
        ]);
    }
    let draft = state.draft(inputs);
    let batch = state.run_tick(draft).unwrap();
    let npcs: Vec<_> = batch
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::EntitySpawn {
                entity,
                kind: EntityKind::Npc,
                ..
            } => Some(*entity),
            _ => None,
        })
        .collect();
    let inputs = (0..pairs)
        .zip(npcs)
        .flat_map(|(n, target)| {
            let entity = EntityId::from_uuid(uuid::Uuid::from_u128(u128::from(n) + 1));
            [
                ZoneInput::system(ZoneCommand::SetTarget {
                    entity,
                    target: Some(target),
                }),
                ZoneInput::system(ZoneCommand::Attack { entity }),
            ]
        })
        .collect();
    (state, inputs)
}

#[tokio::test]
async fn admission_holds_fanout_and_replay_do_not_duplicate_metrics() {
    use crate::application::{
        replay::{verify_with, VerifyOptions},
        replay_log::AppliedTickRecord,
    };
    let m = Metrics::detached();
    let (state, inputs) = arena(2);
    let initial = state.snapshot();
    let (ticks, driver) = manual_ticks();
    let handle = ZoneActor::spawn_gated_with_telemetry(
        state,
        ticks,
        HoldFirstAttack(AtomicBool::new(false)),
        Some(m.consumer(&initial)),
    );
    let mut observer = handle.subscribe();
    let mut other_observer = handle.subscribe();
    for input in inputs {
        handle.send(input).unwrap();
    }
    let mut records = Vec::new();
    let mut attacks = 0;
    let mut held = 0;
    for _ in 0..150 {
        let before = m.render().unwrap();
        let outcome = driver.step().await.unwrap();
        if matches!(outcome, TickOutcome::Held(_)) {
            held += 1;
            assert!(observer.try_recv().is_err());
            // A held attack cannot increment any event counter.
            let after = m.render().unwrap();
            let counters = |s: &str| {
                s.lines()
                    .filter(|l| l.starts_with("nightfall_combat_") && l.contains("_total"))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            };
            assert_eq!(counters(&before), counters(&after));
            continue;
        }
        // The actor logs idle ticks too; capture those by reproducing the draft below.
        if let Ok(batch) = observer.try_recv() {
            assert_eq!(*batch, *other_observer.try_recv().unwrap());
            attacks += batch
                .events
                .iter()
                .filter(|e| matches!(e, ZoneEvent::AttackResult { .. }))
                .count();
            records.push(AppliedTickRecord::from_applied(ZoneId(1), &batch));
        }
    }
    assert_eq!(held, 1);
    assert!(attacks > 0);
    let counted: f64 = ["miss", "hit", "crit"]
        .iter()
        .map(|o| value(&m, "nightfall_combat_attacks_total{", &format!("outcome=\"{o}\"")))
        .sum();
    assert_eq!(counted, attacks as f64);
    assert_eq!(value(&m, "nightfall_combat_tick_duration_seconds_count", ""), 150.0);
    // Fill idle gaps with the same domain runner used by the replay tool. Nothing in
    // that path has a telemetry subscriber, even while this catalogue is alive.
    let before = m.render().unwrap();
    let mut replay_state = ZoneState::from_snapshot(initial.clone()).unwrap();
    let mut complete = Vec::new();
    for record in records {
        while replay_state.snapshot().tick < record.tick {
            let draft = replay_state.draft(vec![]);
            let idle = replay_state.run_tick(draft).unwrap();
            complete.push(Ok(AppliedTickRecord::from_applied(ZoneId(1), &idle)));
        }
        let draft = crate::domain::zone::AppliedTickDraft {
            epoch: record.epoch,
            tick: record.tick,
            commands: record.commands.clone(),
        };
        replay_state.run_tick(draft).unwrap();
        complete.push(Ok(record));
    }
    verify_with(
        ZoneState::from_snapshot(initial).unwrap(),
        tokio_stream::iter(complete),
        VerifyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(before, m.render().unwrap());
}

/// A repeatable actor-only timing sample, not the 200-WebSocket-session acceptance load.
#[tokio::test]
#[ignore = "release performance check: run moon run api:perf-check (p50/p99 for 50 and 200 fighting pairs)"]
#[allow(clippy::print_stdout)] // Explicitly invoked benchmark reports its measurement.
async fn combat_tick_p99_simulation() {
    use crate::application::zone_actor::OpenGate;
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "perf-check must run with --release"
    );
    for pairs in [200, 50] {
        let m = Metrics::detached();
        let (state, inputs) = arena(pairs);
        let initial = state.snapshot();
        let (ticks, driver) = manual_ticks();
        let handle = ZoneActor::spawn_gated_with_telemetry(
            state,
            ticks,
            OpenGate,
            Some(m.consumer(&initial)),
        );
        for input in inputs {
            handle.send(input).unwrap();
        }
        let readings = handle.stats();
        let mut samples = Vec::new();
        for _ in 0..500 {
            driver.step().await.unwrap();
            samples.push(readings.borrow().duration_micros);
        }
        samples.sort_unstable();
        let attacks: f64 = ["miss", "hit", "crit"]
            .iter()
            .map(|o| value(&m, "nightfall_combat_attacks_total{", &format!("outcome=\"{o}\"")))
            .sum();
        assert!(attacks > 0.0);
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        println!("{pairs} players, {pairs} Keltirs initially fighting, 500 manual ticks, OpenGate, {profile} build: p50={} us; p99={} us; attacks={attacks}", samples[249], samples[494]);
        assert!(samples[494] < 20_000, "combat p99 must be below 20 ms");
    }
}

#[test]
fn admitted_intention_changes_count_by_target_intention_only() {
    use crate::domain::zone::{AppliedTick, Intention, ZoneEvent};
    let m = Metrics::detached();
    let mut consumer = m.consumer(&fresh().snapshot());
    let npc = EntityId::from_uuid(uuid::Uuid::from_u128(9));
    let change = |from, to| ZoneEvent::NpcIntentionChanged {
        tick: Tick(3),
        entity: npc,
        from,
        to,
    };
    consumer.admitted(&AppliedTick {
        epoch: 1,
        tick: Tick(3),
        server_time_ms: 0,
        commands: Vec::new(),
        dispositions: Vec::new(),
        events: vec![
            change(Intention::Idle, Intention::Active),
            change(Intention::Active, Intention::Attack),
            change(Intention::Attack, Intention::ReturnHome),
            change(Intention::ReturnHome, Intention::Active),
        ],
        outputs: std::collections::BTreeMap::new(),
        state_digest: [0; 32],
        digest_version: crate::domain::zone::StateDigestVersion::BinaryV2,
    });
    for (label, n) in [
        ("idle", 0.0),
        ("active", 2.0),
        ("attack", 1.0),
        ("return_home", 1.0),
        ("dead", 0.0),
    ] {
        assert_eq!(
            value(&m, "nightfall_npc_intention_transitions_total{", &format!("to=\"{label}\"")),
            n,
            "{label}"
        );
    }
}
