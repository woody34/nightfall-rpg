//! Player death and respawn (Phase 1 E2.4) and kill credit, XP and level transitions
//! (E2.6) against the embedded HF rules and the Keltir template.

use std::sync::{Arc, OnceLock};

use uuid::Uuid;

use super::super::ai::SpawnSlotSpec;
use super::super::combat::{CombatRole, NpcCombat, PlayerLoad, Swing};
use super::super::command::{
    AppliedTick, ObserverOutput, ProgressionDelta, RejectReason, SessionGeneration, ZoneCommand,
    ZoneEvent, ZoneInput,
};
use super::super::entity::{EntityId, EntityKind, Tick};
use super::super::fixed::{Speed, Vec2Fixed};
use super::super::progression::xp_cap;
use super::super::scaled::{Scaled, Q};
use super::super::stat_rules::StatRules;
use super::*;
use crate::application::replay_log::AppliedTickRecord;
use crate::infrastructure::rules_data::{load_rules, RulesSource};
use crate::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};

const GEN1: SessionGeneration = SessionGeneration(1);
const SAFE: (i32, i32) = (126, 126);

fn rules() -> Arc<StatRules> {
    static RULES: OnceLock<Arc<StatRules>> = OnceLock::new();
    RULES
        .get_or_init(|| load_rules(&RulesSource::embedded()).unwrap().rules)
        .clone()
}

fn keltir() -> NpcCombat {
    let def = parse_zone(TEST_ZONE_TOML).unwrap();
    NpcCombat::from_template(&rules(), &def.npc_templates[0]).unwrap()
}

/// A Keltir that one-shots any starter character.
fn brute() -> NpcCombat {
    let mut k = keltir();
    k.stats.p_atk = Scaled::from_raw(5_000 * Q);
    k
}

fn id(n: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(n))
}

fn zone_epoch(epoch: u64) -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(7),
            epoch,
        },
        bounds,
        1_700_000_000_000,
    )
    .with_rules(rules())
    .with_safe_point(Vec2Fixed::from_tiles(SAFE.0, SAFE.1))
}

fn zone() -> ZoneState {
    zone_epoch(3)
}

fn load(xp: u64) -> PlayerLoad {
    PlayerLoad {
        level: super::super::progression::level_for_xp(&rules(), xp),
        xp,
        ..PlayerLoad::fresh("human_fighter")
    }
}

fn spawn_player(n: u128, x: i32, y: i32, l: PlayerLoad) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: id(n),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::DEFAULT,
        generation: GEN1,
        load: Some(Box::new(l)),
    })
}

fn spawn_npc(x: i32, y: i32, c: NpcCombat) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "Keltir".into(),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::from_milli_tiles_per_tick(400),
        combat: Some(Box::new(c)),
    })
}

fn session(n: u128, seq: u32, command: ZoneCommand) -> ZoneInput {
    ZoneInput::session(id(n), GEN1, seq, command)
}

fn target(n: u128, seq: u32, t: EntityId) -> ZoneInput {
    session(
        n,
        seq,
        ZoneCommand::SetTarget {
            entity: id(n),
            target: Some(t),
        },
    )
}

fn attack(n: u128, seq: u32) -> ZoneInput {
    session(n, seq, ZoneCommand::Attack { entity: id(n) })
}

fn respawn(n: u128, seq: u32) -> ZoneInput {
    session(n, seq, ZoneCommand::Respawn { entity: id(n) })
}

fn run(z: &mut ZoneState, inputs: Vec<ZoneInput>) -> AppliedTick {
    let draft = z.draft(inputs);
    z.run_tick(draft).unwrap()
}

/// Idle ticks until `done` holds for one (inclusive), at most `max`.
fn until(z: &mut ZoneState, max: usize, done: impl Fn(&AppliedTick) -> bool) -> Vec<AppliedTick> {
    let mut out = Vec::new();
    for _ in 0..max {
        let t = run(z, Vec::new());
        let stop = done(&t);
        out.push(t);
        if stop {
            return out;
        }
    }
    panic!("condition not reached in {max} ticks");
}

fn died(t: &AppliedTick, who: EntityId) -> bool {
    t.events
        .iter()
        .any(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == who))
}

fn reasons(t: &AppliedTick) -> Vec<RejectReason> {
    t.dispositions.iter().map(|d| d.reason).collect()
}

fn npcs(z: &ZoneState) -> Vec<EntityId> {
    z.entities
        .values()
        .filter(|e| e.kind == EntityKind::Npc)
        .map(|e| e.id)
        .collect()
}

fn combat(z: &ZoneState, e: EntityId) -> &CombatState {
    z.entities[&e].combat.as_ref().unwrap()
}

fn combat_mut(z: &mut ZoneState, e: EntityId) -> &mut CombatState {
    z.entities.get_mut(&e).unwrap().combat.as_mut().unwrap()
}

fn xp_of(z: &ZoneState, e: EntityId) -> u64 {
    match combat(z, e).role {
        CombatRole::Player { xp, .. } => xp,
        CombatRole::Npc { .. } => unreachable!(),
    }
}

fn deltas(ticks: &[AppliedTick]) -> Vec<&ProgressionDelta> {
    ticks.iter().flat_map(AppliedTick::progression).collect()
}

fn xp_events(ticks: &[AppliedTick]) -> Vec<(EntityId, u64, u64)> {
    ticks
        .iter()
        .flat_map(|t| &t.events)
        .filter_map(|e| match e {
            ZoneEvent::XpGained {
                entity,
                amount,
                total,
                ..
            } => Some((*entity, *amount, *total)),
            _ => None,
        })
        .collect()
}

fn sent_to(t: &AppliedTick, who: EntityId) -> Vec<&ZoneEvent> {
    t.outputs
        .get(&who)
        .into_iter()
        .flatten()
        .filter_map(|o| match o {
            ObserverOutput::Event(e) => Some(e),
            _ => None,
        })
        .collect()
}

/// Kills the one NPC `n` targets (its HP set to 1), returning the ticks through the death.
fn kill_npc(z: &mut ZoneState, n: u128, npc: EntityId) -> Vec<AppliedTick> {
    combat_mut(z, npc).hp = 1;
    let mut ticks = vec![run(z, vec![target(n, 1, npc), attack(n, 2)])];
    if !died(&ticks[0], npc) {
        ticks.extend(until(z, 400, |t| died(t, npc)));
    }
    ticks
}

#[test]
fn slot_owned_deaths_credit_once_and_replay_through_each_new_life() {
    let def = parse_zone(TEST_ZONE_TOML).unwrap();
    let mut spec = SpawnSlotSpec::new(&def.spawn_slots[0], &def.npc_templates[0], keltir());
    spec.count = 1;
    spec.home = Vec2Fixed::from_tiles(11, 10);
    spec.speed = Speed::from_milli_tiles_per_tick(0);
    spec.brain.aggressive = false;
    spec.brain.corpse_decay_ticks = 5;
    spec.respawn_delay_secs = 2;
    spec.respawn_random_secs = 3;
    let reward = spec.combat.xp_reward;
    let mut z = zone().with_spawn_slots(vec![spec]);
    run(&mut z, vec![spawn_player(1, 10, 10, load(0))]);
    let npc = npcs(&z)[0];

    for life in 1..=2 {
        let ticks = kill_npc(&mut z, 1, npc);
        assert_eq!(xp_events(&ticks), vec![(id(1), reward, reward * u64::from(life))]);
        assert_eq!(
            ticks
                .iter()
                .flat_map(|t| &t.events)
                .filter(|e| matches!(
                    e, ZoneEvent::EntityDied { entity, incarnation, .. }
                    if *entity == npc && *incarnation == life
                ))
                .count(),
            1
        );
        let snapshot = z.snapshot();
        assert_eq!(snapshot.spawn_members.len(), 1);
        let due = snapshot.spawn_members[0].respawn_at.unwrap();
        let death = ticks.last().unwrap().tick;
        assert!((death.0 + 20..=death.0 + 50).contains(&due.0));

        // Another lethal notification must not award XP, redraw jitter or reschedule.
        let mut repeated = Vec::new();
        z.kill(death, npc, None, &mut repeated);
        assert!(repeated.is_empty());
        assert_eq!(z.snapshot(), snapshot);
        let encoded = crate::application::replay_log::encode_snapshot(&snapshot).unwrap();
        let mut restored = ZoneState::from_snapshot(
            crate::application::replay_log::decode_snapshot(&encoded).unwrap(),
        )
        .unwrap();

        let mut later = Vec::new();
        while z.next_tick <= due {
            let t = run(&mut z, Vec::new());
            let replayed = run(&mut restored, Vec::new());
            assert_eq!(
                AppliedTickRecord::from_applied(ZoneId(7), &t),
                AppliedTickRecord::from_applied(ZoneId(7), &replayed)
            );
            later.push(t);
        }
        assert!(xp_events(&later).is_empty());
        for expected in [
            ZoneEvent::EntityDespawn {
                tick: Tick(death.0 + 5),
                entity: npc,
            },
            ZoneEvent::EntityRespawned {
                tick: due,
                entity: npc,
                position: Vec2Fixed::from_tiles(11, 10),
                hp: combat(&z, npc).sheet.max_hp(),
                incarnation: life + 1,
            },
        ] {
            assert_eq!(
                later
                    .iter()
                    .flat_map(|t| &t.events)
                    .filter(|e| **e == expected)
                    .count(),
                1
            );
            assert_eq!(
                later
                    .iter()
                    .flat_map(|t| sent_to(t, id(1)))
                    .filter(|e| **e == expected)
                    .count(),
                1
            );
        }
        assert_eq!(z.entity_count(), 2);
        assert_eq!(combat(&z, npc).incarnation, life + 1);
        assert_eq!(z.spawn_members().next().unwrap().respawn_at, None);
        assert_eq!(xp_of(&z, id(1)), reward * u64::from(life));
    }
}

#[test]
fn the_plans_xp_60_70_41_example_kill_then_death_then_respawn() {
    let mut z = zone();
    let mut reward = keltir();
    reward.xp_reward = 10;
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10, load(60)),
            spawn_player(2, 12, 12, load(0)),
            spawn_npc(11, 10, reward),
        ],
    );
    let npc = npcs(&z)[0];
    let ticks = kill_npc(&mut z, 1, npc);
    let last = ticks.last().unwrap();
    // E2.6: one award, one level, final stats; owner-only.
    assert_eq!(xp_events(&ticks), vec![(id(1), 10, 70)]);
    assert!(last
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::LevelUp { entity, level: 2, .. } if *entity == id(1))));
    assert!(last.events.iter().any(|e| matches!(
        e,
        ZoneEvent::StatsChanged { entity, level: 2, xp: 70, .. } if *entity == id(1)
    )));
    let mine = sent_to(last, id(1));
    assert!(mine.iter().any(|e| matches!(e, ZoneEvent::XpGained { .. })));
    assert!(mine.iter().any(|e| matches!(e, ZoneEvent::LevelUp { .. })));
    assert!(sent_to(last, id(2)).iter().all(|e| !matches!(
        e,
        ZoneEvent::XpGained { .. } | ZoneEvent::LevelUp { .. } | ZoneEvent::StatsChanged { .. }
    )));
    assert_eq!(combat(&z, id(1)).sheet.level(), 2);
    let d = deltas(&ticks);
    assert_eq!(d.len(), 1);
    assert_eq!(
        (
            d[0].entity,
            d[0].level_before,
            d[0].level,
            d[0].xp_before,
            d[0].xp,
            d[0].xp_gained
        ),
        (id(1), 1, 2, 60, 70, 10)
    );
    assert_eq!(d[0].levels_gained, vec![2]);
    assert!(d[0].alive && d[0].died.is_none() && !d[0].respawned);

    // E2.4: an NPC kills the level-2 player: R((363−68)*0.09875) = 29, XP 41, level 1.
    run(&mut z, vec![spawn_npc(10, 11, brute())]);
    let killer = *npcs(&z).iter().find(|n| **n != npc).unwrap();
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::AddAggro {
            npc: killer,
            target: id(1),
        })],
    );
    assert!(t.dispositions.is_empty());
    let ticks = until(&mut z, 400, |t| died(t, id(1)));
    let last = ticks.last().unwrap();
    assert_eq!(xp_of(&z, id(1)), 41);
    assert_eq!(combat(&z, id(1)).sheet.level(), 1);
    assert!(last.events.iter().any(|e| matches!(
        e,
        ZoneEvent::StatsChanged { entity, hp: 0, level: 1, xp: 41, .. } if *entity == id(1)
    )));
    let d = deltas(&ticks);
    assert_eq!(d.len(), 1);
    let death = d[0].died.as_ref().unwrap();
    assert_eq!(
        (death.xp_lost, death.killer, death.killer_template.as_deref()),
        (29, Some(killer), Some("keltir"))
    );
    assert_eq!((d[0].level_before, d[0].level, d[0].xp, d[0].alive), (2, 1, 41, false));
    let c = combat(&z, id(1));
    assert!(c.mp <= c.sheet.max_mp() && c.swing.is_none() && !c.auto_attack);
    assert!(z.hate_ledger(killer).is_none(), "hate forgotten on death");
    assert_eq!(z.entities[&killer].targeting.target, None);

    // Respawn at the safe point with 65 % HP, 0 MP, protection; no XP refund.
    let max_hp = combat(&z, id(1)).sheet.max_hp();
    let want_hp = (max_hp * 65 / 100).max(1);
    let t = run(&mut z, vec![respawn(1, 3)]);
    assert!(t.dispositions.is_empty());
    let safe = Vec2Fixed::from_tiles(SAFE.0, SAFE.1);
    assert!(t.events.iter().any(|e| *e
        == ZoneEvent::EntityRespawned {
            entity: id(1),
            tick: t.tick,
            position: safe,
            hp: want_hp,
            incarnation: 2,
        }));
    assert!(t.events.iter().any(|e| matches!(
        e,
        ZoneEvent::StatsChanged { entity, mp: 0, xp: 41, level: 1, hp, .. }
            if *entity == id(1) && *hp == want_hp
    )));
    let e = &z.entities[&id(1)];
    assert!(!e.targeting.dead);
    assert_eq!(e.pos, safe);
    let c = combat(&z, id(1));
    assert_eq!((c.hp, c.mp, c.incarnation), (want_hp, 0, 2));
    assert_eq!(c.protected_until, Some(Tick(t.tick.0 + 6000)));
    let d = deltas(std::slice::from_ref(&t));
    assert!(d[0].respawned && d[0].alive && d[0].died.is_none());
    assert_eq!((d[0].xp, d[0].pos), (41, safe));

    // A second Respawn cannot revive the living actor or touch anything.
    let before = z.snapshot();
    let t = run(&mut z, vec![respawn(1, 4)]);
    assert_eq!(reasons(&t), vec![RejectReason::NotDead]);
    assert_eq!(before.entities, z.snapshot().entities);
    assert_eq!(before.rng, z.snapshot().rng);
    assert!(t.events.is_empty());
}

#[test]
fn the_plans_town_respawn_vector_restores_81_of_126_hp() {
    let mut z = zone();
    run(
        &mut z,
        vec![spawn_player(
            1,
            10,
            10,
            PlayerLoad {
                alive: false,
                ..load(0)
            },
        )],
    );
    // The vector is about the arithmetic, so pin maxHP 126 on the sheet.
    let mut stats = combat(&z, id(1)).sheet.to_final();
    stats.max_hp = 126;
    combat_mut(&mut z, id(1)).sheet =
        super::super::stat_sheet::StatSheet::from_final(stats).unwrap();
    run(&mut z, vec![respawn(1, 1)]);
    assert_eq!(combat(&z, id(1)).hp, 81);
}

#[test]
fn same_tick_lethal_hits_and_repeated_respawns_charge_and_revive_once() {
    let mut deaths = 0;
    for epoch in 0..40 {
        let mut z = zone_epoch(epoch);
        run(
            &mut z,
            vec![
                spawn_player(1, 10, 10, load(70)),
                spawn_npc(11, 10, brute()),
                spawn_npc(10, 11, brute()),
            ],
        );
        let now = z.next_tick;
        for npc in npcs(&z) {
            let e = z.entities.get_mut(&npc).unwrap();
            e.targeting.target = Some(id(1));
            let c = e.combat.as_mut().unwrap();
            c.auto_attack = true;
            c.swing = Some(Swing {
                target: id(1),
                target_incarnation: 1,
                start: Tick(0),
                impact: now,
                ready: Tick(now.0 + 12),
            });
        }
        let t = run(&mut z, Vec::new());
        let n = t
            .events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == id(1)))
            .count();
        assert!(n <= 1);
        if n == 0 {
            continue;
        }
        deaths += 1;
        assert_eq!(xp_of(&z, id(1)), 41, "the loss is charged exactly once");
        assert_eq!(deltas(std::slice::from_ref(&t)).len(), 1);
        // Two Respawns on one tick: the first revives, the second finds a living actor.
        let t = run(&mut z, vec![respawn(1, 1), respawn(1, 2)]);
        assert_eq!(reasons(&t), vec![RejectReason::NotDead]);
        assert_eq!(combat(&z, id(1)).incarnation, 2);
        let respawned = t
            .events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::EntityRespawned { .. }))
            .count();
        assert_eq!(respawned, 1);
        assert_eq!(xp_of(&z, id(1)), 41);
    }
    assert!(deaths > 0);
}

#[test]
fn protection_lasts_6000_ticks_and_an_accepted_attack_ends_it_early() {
    for attack_first in [false, true] {
        let mut z = zone();
        run(
            &mut z,
            vec![
                spawn_player(
                    1,
                    10,
                    10,
                    PlayerLoad {
                        alive: false,
                        ..load(0)
                    },
                ),
                spawn_npc(SAFE.0 + 1, SAFE.1, keltir()),
            ],
        );
        let npc = npcs(&z)[0];
        let r = run(&mut z, vec![respawn(1, 1)]).tick;
        let aggro = ZoneInput::system(ZoneCommand::AddAggro { npc, target: id(1) });
        if attack_first {
            let t = run(&mut z, vec![target(1, 2, npc), attack(1, 3), aggro]);
            assert!(t.dispositions.is_empty());
            assert_eq!(combat(&z, id(1)).protected_until, None);
            continue;
        }
        // A rejected Attack (no target) changes nothing, protection included.
        let t = run(&mut z, vec![attack(1, 2)]);
        assert_eq!(reasons(&t), vec![RejectReason::UnknownEntity]);
        assert_eq!(combat(&z, id(1)).protected_until, Some(Tick(r.0 + 6000)));
        let t = run(&mut z, vec![aggro.clone()]);
        assert_eq!(reasons(&t), vec![RejectReason::Protected]);
        while z.next_tick.0 < r.0 + 5999 {
            run(&mut z, Vec::new());
        }
        let t = run(&mut z, vec![aggro.clone()]);
        assert_eq!(t.tick.0, r.0 + 5999);
        assert_eq!(reasons(&t), vec![RejectReason::Protected]);
        let t = run(&mut z, vec![aggro]);
        assert!(t.dispositions.is_empty(), "protection expired at tick {}", r.0 + 6000);
        assert_eq!(z.entities[&npc].targeting.target, Some(id(1)));
    }
}

#[test]
fn a_dead_load_and_a_replacement_session_preserve_death() {
    let mut z = zone();
    let t = run(
        &mut z,
        vec![
            spawn_player(
                1,
                10,
                10,
                PlayerLoad {
                    alive: false,
                    hp: Some(50),
                    ..load(70)
                },
            ),
            spawn_npc(11, 10, keltir()),
        ],
    );
    let npc = npcs(&z)[0];
    assert!(z.entities[&id(1)].targeting.dead);
    assert_eq!(combat(&z, id(1)).hp, 0);
    assert!(sent_to(&t, id(1)).iter().any(|e| matches!(
        e,
        ZoneEvent::EntitySpawn { entity, combat: Some(v), .. } if *entity == id(1) && v.dead && v.hp == 0
    )));
    // The record carries the flag across replay.
    let record = AppliedTickRecord::from_applied(ZoneId(7), &t);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    for (seq, command) in [
        (
            1,
            ZoneCommand::MoveTo {
                entity: id(1),
                dest: Vec2Fixed::from_tiles(12, 12),
            },
        ),
        (
            2,
            ZoneCommand::SetTarget {
                entity: id(1),
                target: Some(npc),
            },
        ),
        (3, ZoneCommand::Attack { entity: id(1) }),
    ] {
        let before = z.snapshot();
        let t = run(&mut z, vec![session(1, seq, command)]);
        assert_eq!(reasons(&t), vec![RejectReason::DeadActor]);
        assert_eq!(before.entities, z.snapshot().entities);
    }
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: id(1),
            generation: SessionGeneration(2),
        })],
    );
    assert!(z.entities[&id(1)].targeting.dead);
    assert!(sent_to(&t, id(1)).iter().any(|e| matches!(
        e,
        ZoneEvent::StatsChanged { entity, hp: 0, xp: 70, level: 2, .. } if *entity == id(1)
    )));
    // A replayed snapshot keeps it too; respawning charges nothing more.
    let mut z = ZoneState::from_snapshot(z.snapshot()).unwrap();
    let t = run(
        &mut z,
        vec![ZoneInput::session(
            id(1),
            SessionGeneration(2),
            4,
            ZoneCommand::Respawn { entity: id(1) },
        )],
    );
    assert!(t.dispositions.is_empty());
    assert_eq!(xp_of(&z, id(1)), 70);
}

#[test]
fn a_multi_level_reward_emits_each_level_then_one_stats_change_and_keeps_hp() {
    let mut z = zone();
    let mut reward = keltir();
    reward.xp_reward = rules().xp_to_level(5).unwrap() + 10;
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10, load(0)),
            spawn_npc(11, 10, reward.clone()),
        ],
    );
    let npc = npcs(&z)[0];
    let ticks = kill_npc(&mut z, 1, npc);
    let last = ticks.last().unwrap();
    let ups: Vec<u32> = last
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::LevelUp { entity, level, .. } if *entity == id(1) => Some(*level),
            _ => None,
        })
        .collect();
    assert_eq!(ups, vec![2, 3, 4, 5]);
    let after_last_up = last
        .events
        .iter()
        .rposition(|e| matches!(e, ZoneEvent::LevelUp { .. }))
        .unwrap();
    let stats: Vec<_> = last.events[after_last_up..]
        .iter()
        .filter(|e| matches!(e, ZoneEvent::StatsChanged { entity, .. } if *entity == id(1)))
        .collect();
    assert_eq!(stats.len(), 1);
    assert!(matches!(stats[0], ZoneEvent::StatsChanged { level: 5, .. }));
    assert_eq!(xp_events(&ticks), vec![(id(1), reward.xp_reward, reward.xp_reward)]);
    let c = combat(&z, id(1));
    assert_eq!(c.sheet.level(), 5);
    // HP/MP are kept (not refilled), and stay within the new maxima.
    assert!(c.hp <= c.sheet.max_hp() && c.mp <= c.sheet.max_mp());
    let fresh = super::super::stat_sheet::StatSheet::for_player(
        &rules(),
        rules().class("human_fighter").unwrap(),
        1,
        Some(rules().starter_weapon()),
    )
    .unwrap();
    assert!(c.hp <= fresh.max_hp(), "a level-up does not refill HP");
    let d = deltas(&ticks);
    assert_eq!(d[0].levels_gained, vec![2, 3, 4, 5]);
}

#[test]
fn xp_is_capped_at_the_sentinel_minus_one_without_a_level_86() {
    let cap = xp_cap(&rules()).unwrap();
    assert_eq!(cap, 16_890_558_727);
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10, load(cap - 5)),
            spawn_npc(11, 10, keltir()),
        ],
    );
    let npc = npcs(&z)[0];
    let ticks = kill_npc(&mut z, 1, npc);
    assert_eq!(xp_events(&ticks), vec![(id(1), 5, cap)]);
    assert!(ticks
        .iter()
        .flat_map(|t| &t.events)
        .all(|e| !matches!(e, ZoneEvent::LevelUp { .. })));
    assert_eq!(combat(&z, id(1)).sheet.level(), 85);
    // Already capped: the next kill awards 0 but still reports it.
    run(&mut z, vec![spawn_npc(11, 10, keltir())]);
    let next = *npcs(&z).iter().find(|n| **n != npc).unwrap();
    let ticks = kill_npc(&mut z, 1, next);
    assert_eq!(xp_events(&ticks), vec![(id(1), 0, cap)]);
}

/// Two players whose swings at one 1-HP NPC are both due on the same tick; player 2 has
/// `p2_damage` recorded on the ledger. Returns the tick, or `None` if player 1 missed.
fn two_killers(epoch: u64, p2_damage: u64) -> Option<(ZoneState, AppliedTick, EntityId)> {
    let mut z = zone_epoch(epoch);
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10, load(0)),
            spawn_player(2, 10, 11, load(0)),
            spawn_npc(11, 10, keltir()),
        ],
    );
    let npc = npcs(&z)[0];
    combat_mut(&mut z, npc).hp = 1;
    if p2_damage > 0 {
        let c = rules();
        z.hate
            .entry(npc)
            .or_default()
            .add(c.constants(), id(2), 1, p2_damage);
    }
    let now = z.next_tick;
    for p in [1, 2] {
        let e = z.entities.get_mut(&id(p)).unwrap();
        e.targeting.target = Some(npc);
        let c = e.combat.as_mut().unwrap();
        c.auto_attack = true;
        c.swing = Some(Swing {
            target: npc,
            target_incarnation: 1,
            start: Tick(0),
            impact: now,
            ready: Tick(now.0 + 12),
        });
    }
    let t = run(&mut z, Vec::new());
    let p1_hit = t.events.iter().any(|e| {
        matches!(e, ZoneEvent::AttackResult { attacker, damage, .. } if *attacker == id(1) && *damage > 0)
    });
    p1_hit.then_some((z, t, npc))
}

fn blow(t: &AppliedTick) -> u64 {
    t.events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::AttackResult {
                attacker, damage, ..
            } if *attacker == id(1) => Some(u64::from(*damage)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn only_the_landed_killing_blow_receives_credit_regardless_of_damage_history() {
    let epoch = (0..40)
        .find(|e| two_killers(*e, 0).is_some())
        .expect("some seed lands player 1's swing");
    let (mut z, t, npc) = two_killers(epoch, 0).unwrap();
    let d = blow(&t);
    // Player 1 lands the blow; player 2's due swing is cancelled without a draw.
    assert!(t
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::EntityDied { killer: Some(k), .. } if *k == id(1))));
    assert_eq!(
        t.events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::AttackResult { target, .. } if *target == npc))
            .count(),
        1
    );
    // Player 1 landed the killing blow.
    assert_eq!(xp_events(std::slice::from_ref(&t)), vec![(id(1), 28, 28)]);
    // Repeated death: a dead NPC can't die again, so later ticks credit nothing.
    let later: Vec<_> = (0..30).map(|_| run(&mut z, Vec::new())).collect();
    assert!(xp_events(&later).is_empty());
    // Equal historical damage does not change the killing blow.
    let (_, t, _) = two_killers(epoch, d).unwrap();
    assert_eq!(blow(&t), d, "same seed, same draws");
    assert_eq!(xp_events(std::slice::from_ref(&t)), vec![(id(1), 28, 28)]);
    // Player 2 recorded more, but its pending swing never lands.
    let (z2, t, _) = two_killers(epoch, d + 1).unwrap();
    assert_eq!(xp_events(std::slice::from_ref(&t)), vec![(id(1), 28, 28)]);
    assert_eq!(xp_of(&z2, id(2)), 0);
    let d = deltas(std::slice::from_ref(&t));
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].entity, id(1));
}

#[test]
fn a_mid_fight_death_and_respawn_snapshot_continues_byte_identically() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10, load(70)),
            spawn_npc(11, 10, brute()),
        ],
    );
    let npc = npcs(&z)[0];
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    let inputs = |tick: u64| -> Vec<ZoneInput> {
        if tick.is_multiple_of(40) {
            vec![respawn(1, u32::try_from(tick).unwrap())]
        } else {
            Vec::new()
        }
    };
    for split in [5_u64, 37, 81, 160] {
        let mut a = z.clone();
        while a.next_tick.0 < split {
            let tick = a.next_tick.0;
            run(&mut a, inputs(tick));
        }
        let mut b = ZoneState::from_snapshot(a.snapshot()).unwrap();
        for _ in 0..200 {
            let tick = a.next_tick.0;
            let ta = run(&mut a, inputs(tick));
            let tb = run(&mut b, inputs(tick));
            assert_eq!(
                AppliedTickRecord::from_applied(ZoneId(7), &ta).encode(),
                AppliedTickRecord::from_applied(ZoneId(7), &tb).encode()
            );
        }
    }
}
