//! Combat state, auto-attack and hate (Phase 1 E2.2, E2.3, E2.5) against the embedded HF
//! rules and the Keltir template. Impacts are checked against an independent re-derivation
//! from the snapshotted generator state, so the draw order is pinned, not just stable.

use std::sync::{Arc, OnceLock};

use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use uuid::Uuid;

use super::super::combat::{
    CombatRole, HateEntry, HateLedger, NpcCombat, PlayerLoad, Swing, SwingCancel,
};
use super::super::combat_math::{
    attack_timing, crit_lands, damage_hate, hit_chance_permille, hit_lands, physical_damage,
};
use super::super::command::{
    AppliedTick, AttackOutcome, CommandSource, ObserverOutput, RejectReason, SessionGeneration,
    ZoneCommand, ZoneEvent, ZoneInput,
};
use super::super::entity::{EntityId, EntityKind, Tick};
use super::super::fixed::{Fixed, Speed, Vec2Fixed};
use super::super::stat_rules::StatRules;
use super::*;
use crate::application::replay_log::{decode_snapshot, encode_snapshot, AppliedTickRecord};
use crate::infrastructure::rules_data::{load_rules, RulesSource};
use crate::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};

const GEN1: SessionGeneration = SessionGeneration(1);

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

fn id(n: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(n))
}

fn zone() -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(7),
            epoch: 3,
        },
        bounds,
        1_700_000_000_000,
    )
    .with_rules(rules())
}

fn spawn_player(n: u128, x: i32, y: i32) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: id(n),
        name: format!("p{n}"),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::DEFAULT,
        generation: GEN1,
        load: Some(Box::new(PlayerLoad::fresh("human_fighter"))),
    })
}

fn spawn_keltir(x: i32, y: i32) -> ZoneInput {
    ZoneInput::system(ZoneCommand::SpawnNpc {
        name: "Keltir".into(),
        pos: Vec2Fixed::from_tiles(x, y),
        speed: Speed::from_milli_tiles_per_tick(400),
        combat: Some(Box::new(keltir())),
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

fn run(z: &mut ZoneState, inputs: Vec<ZoneInput>) -> AppliedTick {
    let draft = z.draft(inputs);
    z.run_tick(draft).unwrap()
}

fn idle(z: &mut ZoneState, n: usize) -> Vec<AppliedTick> {
    (0..n).map(|_| run(z, Vec::new())).collect()
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

/// Player 1 at (10, 10) next to one Keltir at (11, 10), both in reach of each other.
fn duel() -> (ZoneState, EntityId) {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 10, 10), spawn_keltir(11, 10)]);
    let npc = npcs(&z)[0];
    (z, npc)
}

fn events_of(ticks: &[AppliedTick]) -> impl Iterator<Item = &ZoneEvent> {
    ticks.iter().flat_map(|t| t.events.iter())
}

fn started(ticks: &[AppliedTick], attacker: EntityId) -> Vec<(Tick, Tick, Tick)> {
    events_of(ticks)
        .filter_map(|e| match e {
            ZoneEvent::AttackStarted {
                tick,
                attacker: a,
                impact,
                ready,
                ..
            } if *a == attacker => Some((*tick, *impact, *ready)),
            _ => None,
        })
        .collect()
}

fn results(ticks: &[AppliedTick], attacker: EntityId) -> Vec<(Tick, AttackOutcome, u32, u32)> {
    events_of(ticks)
        .filter_map(|e| match e {
            ZoneEvent::AttackResult {
                attacker: a,
                tick,
                outcome,
                damage,
                target_hp_after,
                ..
            } if *a == attacker => Some((*tick, *outcome, *damage, *target_hp_after)),
            _ => None,
        })
        .collect()
}

/// Independent re-derivation of an impact from the generator state just before it: the
/// plan's draw order with rejection sampling written out here, not borrowed from the zone.
fn oracle(rng: RngState, attacker: &StatSheet, target: &StatSheet) -> (AttackOutcome, u32, u128) {
    let mut r = ChaCha12Rng::from_seed(rng.key);
    r.set_stream(rng.stream);
    r.set_word_pos(rng.word_pos);
    let mut below = |n: u32| loop {
        let v = u64::from(r.next_u32());
        let limit = (1_u64 << 32) - (1_u64 << 32) % u64::from(n);
        if v < limit {
            return u32::try_from(v % u64::from(n)).unwrap();
        }
    };
    let rules = rules();
    let c = rules.constants();
    let chance = hit_chance_permille(c, attacker.accuracy(), target.evasion()).unwrap();
    let out = if hit_lands(c, chance, below(c.hit_roll_range)).unwrap() {
        let crit = crit_lands(c, attacker.crit_permille(), below(c.crit_roll_range)).unwrap();
        let rd = attacker.random_damage();
        let j = i64::from(below(2 * rd + 1)) - i64::from(rd);
        let d = physical_damage(c, attacker, target, crit, j).unwrap();
        (
            if crit {
                AttackOutcome::Crit
            } else {
                AttackOutcome::Hit
            },
            d,
        )
    } else {
        (AttackOutcome::Miss, 0)
    };
    (out.0, out.1, r.get_word_pos())
}

// ---- E2.2: combat state, snapshot, log -------------------------------------------------------

#[test]
fn a_loaded_player_spawns_with_its_sheet_and_only_it_is_told_its_stats() {
    let mut z = zone();
    let t = run(&mut z, vec![spawn_player(1, 10, 10), spawn_player(2, 11, 10)]);
    let c = combat(&z, id(1));
    let sheet = StatSheet::for_player(
        &rules(),
        rules().class("human_fighter").unwrap(),
        1,
        Some(rules().starter_weapon()),
    )
    .unwrap();
    assert_eq!(c.sheet, sheet);
    assert_eq!((c.hp, c.mp), (sheet.max_hp(), sheet.max_mp()));
    assert_eq!(c.attack_range, Fixed::from_raw(1250), "40 L2 units at 32 per tile");
    assert_eq!(c.incarnation, 1);
    let stats = |p: u128| {
        t.outputs[&id(p)]
            .iter()
            .filter(|o| matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { .. })))
            .map(|o| match o {
                ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. }) => *entity,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(stats(1), vec![id(1)]);
    assert_eq!(stats(2), vec![id(2)]);
    // The other player's spawn carries public combat state.
    let spawn = t.outputs[&id(1)]
        .iter()
        .find_map(|o| match o {
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, combat, .. })
                if *entity == id(2) =>
            {
                combat.clone()
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (spawn.hp, spawn.max_hp, spawn.level, spawn.dead),
        (sheet.max_hp(), sheet.max_hp(), 1, false)
    );
}

#[test]
fn an_invalid_load_is_refused_without_changing_state_or_rng() {
    let mut z = zone();
    for load in [
        PlayerLoad::fresh("no_such_class"),
        PlayerLoad {
            level: 2,
            ..PlayerLoad::fresh("human_fighter")
        },
        PlayerLoad {
            xp: u64::MAX,
            ..PlayerLoad::fresh("human_fighter")
        },
    ] {
        let before = z.snapshot();
        let t = run(
            &mut z,
            vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: id(1),
                name: "x".into(),
                pos: Vec2Fixed::from_tiles(1, 1),
                speed: Speed::DEFAULT,
                generation: GEN1,
                load: Some(Box::new(load)),
            })],
        );
        assert_eq!(reasons(&t), vec![RejectReason::InvalidLoad]);
        assert_eq!(before.entities, z.snapshot().entities);
        assert_eq!(before.rng, z.snapshot().rng);
    }
    // Saved HP is clamped to the maximum; zero spawns the character dead.
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: id(2),
            name: "y".into(),
            pos: Vec2Fixed::from_tiles(1, 1),
            speed: Speed::DEFAULT,
            generation: GEN1,
            load: Some(Box::new(PlayerLoad {
                hp: Some(0),
                mp: Some(u32::MAX),
                ..PlayerLoad::fresh("human_fighter")
            })),
        })],
    );
    assert!(t.dispositions.is_empty());
    let c = combat(&z, id(2));
    assert_eq!((c.hp, c.mp), (0, c.sheet.max_mp()));
    assert!(z.entities[&id(2)].targeting.dead);
}

#[test]
fn a_mid_swing_snapshot_continues_byte_identically() {
    let (mut z, npc) = duel();
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    // Every boundary from the swing start through a kill: swing pending, cooldown, hate.
    let mut a = z.clone();
    let mut boundary = 0;
    loop {
        let snap = a.snapshot();
        let json = encode_snapshot(&snap).unwrap();
        let decoded = decode_snapshot(&json).unwrap();
        assert_eq!(encode_snapshot(&decoded).unwrap(), json, "snapshot JSON is canonical");
        let mut b = ZoneState::from_snapshot(decoded).unwrap();
        assert_eq!(b, a);
        let ta = idle(&mut a, 25);
        let tb = idle(&mut b, 25);
        for (x, y) in ta.iter().zip(&tb) {
            assert_eq!(
                AppliedTickRecord::from_applied(ZoneId(7), x).encode(),
                AppliedTickRecord::from_applied(ZoneId(7), y).encode()
            );
        }
        assert_eq!(ta, tb);
        if a.entities[&npc].targeting.dead || boundary > 20 {
            break;
        }
        boundary += 1;
        // Restart from a boundary one tick later.
        a = z.clone();
        idle(&mut a, boundary);
    }
    assert!(z
        .entities
        .values()
        .any(|e| e.combat.as_ref().is_some_and(|c| c.swing.is_some())));
}

#[test]
fn off_aoi_combat_is_recorded_but_not_sent() {
    let (mut z, npc) = duel();
    run(&mut z, vec![spawn_player(2, 200, 200)]);
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    let ticks = idle(&mut z, 20);
    assert!(!results(&ticks, id(1)).is_empty());
    for t in &ticks {
        let record = AppliedTickRecord::from_applied(ZoneId(7), t);
        assert_eq!(
            crate::application::replay_log::decode_events(&record.events).unwrap(),
            t.events
        );
        assert!(t
            .outputs
            .get(&id(2))
            .is_none_or(|out| out.iter().all(|o| !matches!(
                o,
                ObserverOutput::Event(
                    ZoneEvent::AttackResult { .. }
                        | ZoneEvent::AttackStarted { .. }
                        | ZoneEvent::HateChanged { .. }
                        | ZoneEvent::StatsChanged { .. }
                )
            ))));
    }
    assert!(events_of(&ticks).any(|e| matches!(e, ZoneEvent::HateChanged { .. })));
}

#[test]
fn aoi_entry_and_replacement_reconstruct_combat_state() {
    let (mut z, npc) = duel();
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    // Find a tick boundary with the player's swing in flight.
    while combat(&z, id(1)).swing.is_none() {
        run(&mut z, Vec::new());
    }
    let swing = combat(&z, id(1)).swing;
    let hp = combat(&z, npc).hp;
    let t = run(&mut z, vec![spawn_player(2, 12, 10)]);
    let seen = |out: &Vec<ObserverOutput>, who: EntityId| {
        out.iter().find_map(|o| match o {
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, combat, .. })
                if *entity == who =>
            {
                combat.clone()
            },
            _ => None,
        })
    };
    let attacker = seen(&t.outputs[&id(2)], id(1)).unwrap();
    assert!(attacker.swing.is_some() || swing.is_some_and(|s| s.impact == t.tick));
    let monster = seen(&t.outputs[&id(2)], npc).unwrap();
    assert_eq!(monster.template.as_deref(), Some("keltir"));
    assert!(monster.attackable && monster.hp <= hp);
    // A replacement session gets the AOI again, its stats and its selection.
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: id(1),
            generation: SessionGeneration(2),
        })],
    );
    let out = &t.outputs[&id(1)];
    assert!(seen(out, npc).is_some());
    assert!(out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. }) if *entity == id(1))));
    assert!(out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::TargetChanged { target: Some(t), .. }) if *t == npc)));
}

#[test]
fn snapshots_of_another_schema_or_with_inconsistent_combat_are_refused() {
    let (mut z, npc) = duel();
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    idle(&mut z, 3);
    let snap = z.snapshot();
    assert_eq!(snap.meta.schema_version, 5);
    let mut compatible = snap.clone();
    compatible.meta.schema_version = 4;
    assert!(ZoneState::from_snapshot(compatible).is_ok());
    for v in [1, 2, 3] {
        let mut old = snap.clone();
        old.meta.schema_version = v;
        assert_eq!(ZoneState::from_snapshot(old), Err(SnapshotError::Schema(v)));
    }
    let mut outside = snap.clone();
    outside.safe_point = Some(Vec2Fixed::from_tiles(300, 0));
    assert_eq!(ZoneState::from_snapshot(outside), Err(SnapshotError::SafePointOutOfBounds));
    let mut no_rules = snap.clone();
    no_rules.rules = None;
    assert_eq!(ZoneState::from_snapshot(no_rules), Err(SnapshotError::CombatMismatch));
    let mut bad_rules = snap.clone();
    if let Some(parts) = bad_rules.rules.as_mut() {
        parts.max_level = 0;
    }
    assert!(matches!(ZoneState::from_snapshot(bad_rules), Err(SnapshotError::Rules(_))));
    let mut stray = snap.clone();
    stray.hate.push(NpcHate {
        npc: id(1),
        ledger: HateLedger::default(),
    });
    assert_eq!(ZoneState::from_snapshot(stray), Err(SnapshotError::CombatMismatch));
    let json = encode_snapshot(&snap).unwrap();
    let tampered = String::from_utf8(json)
        .unwrap()
        .replacen("\"p_def\":", "\"p_def\":-", 1);
    assert!(decode_snapshot(tampered.as_bytes()).is_err(), "a sheet the constructor refuses");
}

#[test]
fn the_state_digest_covers_state_no_one_observes() {
    let (mut z, npc) = duel();
    let mut drifted = z.clone();
    drifted
        .hate
        .entry(npc)
        .or_default()
        .add(rules().constants(), id(9), 5, 0);
    let a = run(&mut z, Vec::new());
    let b = run(&mut drifted, Vec::new());
    assert_eq!(a.outputs, b.outputs);
    assert_eq!(a.events, b.events);
    assert_ne!(a.state_digest, b.state_digest);
}

// ---- E2.3: auto-attack and damage -------------------------------------------------------------

#[test]
fn attack_starts_a_swing_on_the_stat_engines_ticks_and_impacts_match_the_oracle() {
    let (mut z, npc) = duel();
    let t = run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    assert!(t.dispositions.is_empty());
    let sheet = combat(&z, id(1)).sheet;
    let timing = attack_timing(rules().constants(), sheet.attack_speed()).unwrap();
    // HF human fighter, Squire's Sword, DEX 30 (plan vector 5, E1.4 source rounding).
    assert_eq!((timing.impact_ticks, timing.cycle_ticks), (6, 12));
    let s = started(std::slice::from_ref(&t), id(1));
    assert_eq!(s, vec![(t.tick, Tick(t.tick.0 + 6), Tick(t.tick.0 + 12))]);
    let target_sheet = combat(&z, npc).sheet;
    // Run to the impact and compare with the oracle drawn from the state just before it.
    idle(&mut z, 5);
    let before = z.snapshot().rng;
    let impact = run(&mut z, Vec::new());
    assert_eq!(impact.tick, Tick(t.tick.0 + 6));
    let (outcome, damage, word_pos) = oracle(before, &sheet, &target_sheet);
    let got = results(std::slice::from_ref(&impact), id(1));
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].1, got[0].2), (outcome, damage));
    assert_eq!(got[0].3, 44 - damage);
    assert_eq!(z.snapshot().rng.word_pos, word_pos, "exactly the specified draws");
}

#[test]
fn repeating_attack_adds_no_swing_and_keeps_the_cooldown() {
    let (mut z, npc) = duel();
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    let swing = combat(&z, id(1)).swing.unwrap();
    let t = run(&mut z, vec![attack(1, 3), attack(1, 4)]);
    assert!(t.dispositions.is_empty());
    assert_eq!(combat(&z, id(1)).swing, Some(swing));
    let ticks = idle(&mut z, 30);
    let starts: Vec<u64> = started(&ticks, id(1)).iter().map(|s| s.0 .0).collect();
    assert!(starts.windows(2).all(|w| w[1] - w[0] == 12), "{starts:?}");
    assert!(starts.first().is_none_or(|s| *s == swing.ready.0));
}

#[test]
fn stop_attack_cancels_without_a_draw_and_restarting_waits_for_the_cooldown() {
    let (mut z, npc) = duel();
    let t0 = run(&mut z, vec![target(1, 1, npc), attack(1, 2)]).tick;
    let rng = z.snapshot().rng;
    let t = run(&mut z, vec![session(1, 3, ZoneCommand::StopAttack { entity: id(1) })]);
    assert!(t.events.iter().any(|e| matches!(
        e,
        ZoneEvent::AttackCancelled {
            reason: SwingCancel::Stopped,
            ..
        }
    )));
    assert_eq!(z.snapshot().rng, rng, "a cancelled swing draws nothing");
    assert!(!combat(&z, id(1)).auto_attack);
    // Stop is idempotent.
    let t = run(&mut z, vec![session(1, 4, ZoneCommand::StopAttack { entity: id(1) })]);
    assert!(t.dispositions.is_empty() && t.events.is_empty());
    let ticks: Vec<AppliedTick> = std::iter::once(run(&mut z, vec![attack(1, 5)]))
        .chain(idle(&mut z, 15))
        .collect();
    let s = started(&ticks, id(1));
    assert_eq!(s[0].0, Tick(t0.0 + 12), "the cancelled swing's cooldown still applies");
}

#[test]
fn changing_or_clearing_the_target_and_moving_cancel_the_swing() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            spawn_keltir(11, 10),
            spawn_keltir(10, 11),
        ],
    );
    let (a, b) = (npcs(&z)[0], npcs(&z)[1]);
    run(&mut z, vec![target(1, 1, a), attack(1, 2)]);
    let t = run(&mut z, vec![target(1, 3, b)]);
    assert!(t.events.iter().any(|e| matches!(e, ZoneEvent::AttackCancelled { reason: SwingCancel::TargetChanged, target, .. } if *target == a)));
    assert!(!combat(&z, id(1)).auto_attack, "a new target needs a new Attack");
    run(&mut z, vec![attack(1, 4)]);
    idle(&mut z, 12);
    let t = run(
        &mut z,
        vec![session(
            1,
            5,
            ZoneCommand::MoveTo {
                entity: id(1),
                dest: Vec2Fixed::from_tiles(20, 20),
            },
        )],
    );
    assert!(!combat(&z, id(1)).auto_attack);
    assert!(combat(&z, id(1)).swing.is_none());
    assert!(t
        .events
        .iter()
        .all(|e| !matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == id(1))));
}

#[test]
fn an_out_of_reach_target_is_chased_then_swung_at_and_rechecked_at_impact() {
    let mut z = zone();
    run(&mut z, vec![spawn_player(1, 10, 10), spawn_keltir(16, 10)]);
    let npc = npcs(&z)[0];
    let ticks: Vec<AppliedTick> =
        std::iter::once(run(&mut z, vec![target(1, 1, npc), attack(1, 2)]))
            .chain(idle(&mut z, 12))
            .collect();
    let moved = events_of(&ticks)
        .filter(|e| matches!(e, ZoneEvent::EntityMove { entity, .. } if *entity == id(1)))
        .count();
    assert!(moved >= 2, "the player walked toward the target");
    let s = started(&ticks, id(1));
    assert!(!s.is_empty());
    assert!(z.entities[&id(1)].dest.is_none(), "the chase stops in reach");
    let reach = combat(&z, id(1)).attack_range.raw() + combat(&z, npc).collision_radius.raw();
    assert!(z.entities[&id(1)]
        .pos
        .within(z.entities[&npc].pos, Fixed::from_raw(reach)));
    // The target walks away mid-swing: the impact is cancelled without a draw.
    let mut z2 = zone();
    run(&mut z2, vec![spawn_player(1, 10, 10), spawn_keltir(11, 10)]);
    let npc = npcs(&z2)[0];
    let t0 = run(&mut z2, vec![target(1, 1, npc), attack(1, 2)]).tick;
    z2.entities.get_mut(&npc).unwrap().speed = Speed::from_milli_tiles_per_tick(2000);
    z2.entities.get_mut(&npc).unwrap().dest = Some(Vec2Fixed::from_tiles(30, 10));
    idle(&mut z2, 5);
    let rng = z2.snapshot().rng;
    let impact = run(&mut z2, Vec::new());
    assert_eq!(impact.tick, Tick(t0.0 + 6));
    assert!(impact.events.iter().any(|e| matches!(
        e,
        ZoneEvent::AttackCancelled {
            reason: SwingCancel::OutOfRange,
            ..
        }
    )));
    assert_eq!(z2.snapshot().rng.word_pos, rng.word_pos);
}

#[test]
fn a_lethal_hit_kills_once_clears_attackers_and_dead_actors_are_refused() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            spawn_player(2, 10, 11),
            spawn_keltir(11, 10),
        ],
    );
    let npc = npcs(&z)[0];
    run(
        &mut z,
        vec![
            target(1, 1, npc),
            attack(1, 2),
            target(2, 1, npc),
            attack(2, 2),
        ],
    );
    z.entities
        .get_mut(&npc)
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 1;
    let ticks = idle(&mut z, 40);
    let died: Vec<_> = events_of(&ticks)
        .filter(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == npc))
        .collect();
    assert_eq!(died.len(), 1);
    let ZoneEvent::EntityDied {
        killer,
        incarnation,
        ..
    } = died[0]
    else {
        unreachable!()
    };
    assert_eq!(*incarnation, 1);
    assert!(killer.is_some_and(|k| k == id(1) || k == id(2)));
    assert!(z.entities[&npc].targeting.dead);
    assert_eq!(combat(&z, npc).hp, 0);
    for p in [1, 2] {
        assert_eq!(z.entities[&id(p)].targeting.target, None);
        assert!(!combat(&z, id(p)).auto_attack && combat(&z, id(p)).swing.is_none());
    }
    assert!(!z.hate.contains_key(&npc));
    // Simultaneous impacts on one tick: the second finds the target dead and is cancelled.
    let death_tick = ticks
        .iter()
        .find(|t| {
            t.events
                .iter()
                .any(|e| matches!(e, ZoneEvent::EntityDied { .. }))
        })
        .unwrap();
    let hits = death_tick
        .events
        .iter()
        .filter(|e| matches!(e, ZoneEvent::AttackResult { target, .. } if *target == npc))
        .count();
    assert_eq!(hits, 1);
    let t = run(&mut z, vec![target(1, 3, npc)]);
    assert_eq!(reasons(&t), vec![RejectReason::NonAttackableTarget]);
    // A dead player cannot steer itself.
    z.entities.get_mut(&id(2)).unwrap().targeting.dead = true;
    for (seq, command) in [
        (
            5,
            ZoneCommand::MoveTo {
                entity: id(2),
                dest: Vec2Fixed::from_tiles(12, 12),
            },
        ),
        (6, ZoneCommand::StopMove { entity: id(2) }),
        (7, ZoneCommand::Attack { entity: id(2) }),
        (8, ZoneCommand::StopAttack { entity: id(2) }),
        (
            9,
            ZoneCommand::SetTarget {
                entity: id(2),
                target: None,
            },
        ),
    ] {
        let t = run(&mut z, vec![session(2, seq, command)]);
        assert_eq!(reasons(&t), vec![RejectReason::DeadActor]);
    }
}

#[test]
fn an_attacker_killed_earlier_in_the_tick_does_not_land_its_impact() {
    // Players 1 and u128::MAX bracket every NPC id, so both impact orders are exercised.
    for n in [1_u128, u128::MAX] {
        let mut z = zone();
        run(&mut z, vec![spawn_player(n, 10, 10), spawn_keltir(11, 10)]);
        let (player, npc) = (id(n), npcs(&z)[0]);
        let now = z.next_tick;
        for (me, foe) in [(player, npc), (npc, player)] {
            let e = z.entities.get_mut(&me).unwrap();
            e.targeting.target = Some(foe);
            let c = e.combat.as_mut().unwrap();
            c.hp = 1;
            c.auto_attack = true;
            c.swing = Some(Swing {
                target: foe,
                target_incarnation: 1,
                start: Tick(0),
                impact: now,
                ready: Tick(now.0 + 12),
            });
        }
        let t = run(&mut z, Vec::new());
        let (first, second) = if player < npc {
            (player, npc)
        } else {
            (npc, player)
        };
        let one = std::slice::from_ref(&t);
        let r = results(one, first);
        assert_eq!(r.len(), 1, "the lower id lands first");
        if r[0].1 == AttackOutcome::Miss {
            assert_eq!(results(one, second).len(), 1);
        } else {
            assert!(results(one, second).is_empty(), "killed before its impact");
            assert!(t.events.iter().any(|e| matches!(e, ZoneEvent::AttackCancelled { attacker, reason: SwingCancel::AttackerDied, .. } if *attacker == second)));
            assert!(t
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == second)));
        }
        let deaths = t
            .events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::EntityDied { .. }))
            .count();
        assert!(deaths <= 1);
    }
}

#[test]
fn the_client_has_no_way_to_supply_damage() {
    // Every damage figure comes from the zone; the commands that start combat carry ids only.
    let (mut z, npc) = duel();
    let t = run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    for c in &t.commands {
        assert!(matches!(c.command, ZoneCommand::SetTarget { .. } | ZoneCommand::Attack { .. }));
    }
    let ticks = idle(&mut z, 20);
    let sheet = combat(&z, id(1)).sheet;
    for (_, outcome, damage, _) in results(&ticks, id(1)) {
        let max = physical_damage(
            rules().constants(),
            &sheet,
            &combat(&z, npc).sheet,
            true,
            i64::from(sheet.random_damage()),
        )
        .unwrap();
        assert!(outcome == AttackOutcome::Miss && damage == 0 || (1..=max).contains(&damage));
    }
}

// ---- E2.5: aggro and hate --------------------------------------------------------------------

#[test]
fn landed_damage_adds_hf_hate_and_a_zero_value_attack_adds_one() {
    let (mut z, npc) = duel();
    run(&mut z, vec![target(1, 1, npc), attack(1, 2)]);
    let ticks = idle(&mut z, 7);
    let (_, outcome, damage, _) = results(&ticks, id(1))[0];
    let expected = if outcome == AttackOutcome::Miss {
        1
    } else {
        damage_hate(rules().constants(), damage, 1).unwrap().max(1)
    };
    // Keltir is level 1: F(d * 100 / 8).
    assert_eq!(
        z.hate[&npc].get(id(1)),
        Some(HateEntry {
            hate: expected,
            damage: damage.into()
        })
    );
    assert_eq!(
        z.entities[&npc].targeting.target,
        Some(id(1)),
        "the monster turns on its attacker"
    );
    assert!(combat(&z, npc).auto_attack);
}

#[test]
fn hate_is_capped_and_ties_keep_the_current_target_then_the_lowest_id() {
    let c = rules();
    let c = c.constants();
    let mut l = HateLedger::default();
    l.add(c, id(5), 999_999_990, 3);
    assert_eq!(
        l.add(c, id(5), 100, 4),
        HateEntry {
            hate: 999_999_999,
            damage: 7
        }
    );
    l.add(c, id(3), 999_999_999, 0);
    assert_eq!(l.most_hated(None, |_| true), Some(id(3)));
    assert_eq!(l.most_hated(Some(id(5)), |_| true), Some(id(5)));
    assert_eq!(l.most_hated(Some(id(9)), |_| true), Some(id(3)));
    assert_eq!(l.most_hated(None, |e| e != id(3)), Some(id(5)));
    l.add(c, id(4), 1, 0);
    assert!(l.forget(id(3)) && !l.forget(id(3)));
    assert_eq!(l.most_hated(Some(id(4)), |_| true), Some(id(5)));
}

#[test]
fn two_attackers_the_most_hated_is_chosen_and_retained_on_ties() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            spawn_player(2, 12, 10),
            spawn_keltir(11, 10),
        ],
    );
    let npc = npcs(&z)[0];
    let aggro = |n| ZoneInput::system(ZoneCommand::AddAggro { npc, target: id(n) });
    let t = run(&mut z, vec![aggro(2), aggro(1)]);
    assert!(t.dispositions.is_empty());
    assert_eq!(z.entities[&npc].targeting.target, Some(id(2)), "first in keeps a tie");
    run(&mut z, vec![aggro(1)]);
    assert_eq!(z.entities[&npc].targeting.target, Some(id(1)), "more hate wins");
    run(&mut z, vec![aggro(2)]);
    assert_eq!(
        z.entities[&npc].targeting.target,
        Some(id(1)),
        "a tie retains the current target"
    );
    // Disconnect: the player leaves every ledger and the monster re-selects.
    let t = run(&mut z, vec![ZoneInput::system(ZoneCommand::Despawn { entity: id(1) })]);
    assert!(t
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::HateChanged { target, hate: 0, .. } if *target == id(1))));
    assert_eq!(z.hate[&npc].get(id(1)), None);
    assert_eq!(z.entities[&npc].targeting.target, Some(id(2)));
    run(&mut z, vec![ZoneInput::system(ZoneCommand::Despawn { entity: id(2) })]);
    assert_eq!(z.entities[&npc].targeting.target, None);
    assert!(!combat(&z, npc).auto_attack);
    assert!(!z.hate.contains_key(&npc));
}

#[test]
fn a_killed_player_is_forgotten_and_aggro_commands_are_validated() {
    let (mut z, npc) = duel();
    run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::AddAggro {
            npc,
            target: id(1),
        })],
    );
    z.entities
        .get_mut(&id(1))
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 1;
    let ticks = idle(&mut z, 40);
    assert!(events_of(&ticks)
        .any(|e| matches!(e, ZoneEvent::EntityDied { entity, .. } if *entity == id(1))));
    assert!(!z.hate.contains_key(&npc));
    assert_eq!(z.entities[&npc].targeting.target, None);
    // Sessions cannot issue aggro; dead or noncombat targets are refused.
    let t = run(&mut z, vec![session(1, 9, ZoneCommand::AddAggro { npc, target: id(1) })]);
    assert_eq!(reasons(&t), vec![RejectReason::NotPermitted]);
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::AddAggro {
            npc,
            target: id(1),
        })],
    );
    assert_eq!(reasons(&t), vec![RejectReason::NonAttackableTarget]);
    let t = run(
        &mut z,
        vec![ZoneInput::system(ZoneCommand::AddAggro {
            npc: id(1),
            target: npc,
        })],
    );
    assert_eq!(reasons(&t), vec![RejectReason::NotPermitted]);
}

#[test]
fn rejected_attacks_add_no_hate() {
    let mut z = zone();
    run(
        &mut z,
        vec![
            spawn_player(1, 10, 10),
            spawn_keltir(11, 10),
            spawn_player(2, 200, 200),
        ],
    );
    let npc = npcs(&z)[0];
    let before = z.snapshot();
    // No target selected; a target outside the AOI; a player target.
    let t = run(&mut z, vec![attack(1, 1), attack(2, 1)]);
    assert_eq!(reasons(&t), vec![RejectReason::UnknownEntity; 2]);
    z.entities.get_mut(&id(2)).unwrap().targeting.target = Some(npc);
    let t = run(&mut z, vec![attack(2, 2)]);
    assert_eq!(reasons(&t), vec![RejectReason::TargetNotInAoi]);
    z.entities.get_mut(&id(2)).unwrap().targeting.target = Some(id(1));
    let t = run(&mut z, vec![attack(2, 3)]);
    assert_eq!(reasons(&t), vec![RejectReason::NonAttackableTarget]);
    idle(&mut z, 20);
    assert!(z.hate.is_empty());
    assert_eq!(before.rng, z.snapshot().rng);
    assert!(matches!(combat(&z, npc).role, CombatRole::Npc { .. }));
    let _ = CommandSource::System;
}
