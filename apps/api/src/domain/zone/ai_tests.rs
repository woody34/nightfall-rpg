//! NPC AI (Phase 1 E3.2), chase/leash/return (E3.3) and corpse/respawn (E3.4) against the
//! Keltir template under fixed seeds and ticks. RNG-dependent outcomes are re-derived from the
//! generator state by an independent oracle, so the documented draw order is pinned.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use proptest::prelude::*;
use rand_chacha::rand_core::{Rng, SeedableRng};
use rand_chacha::ChaCha12Rng;
use uuid::Uuid;

use super::super::ai::{
    Intention, NpcAi, SpawnSlotSpec, MAX_ATTACK_TIMEOUT_TICKS, MAX_DRIFT_RANGE, RANDOM_WALK_RATE,
    THINK_INTERVAL_TICKS,
};
use super::super::combat::{NpcCombat, PlayerLoad};
use super::super::command::{
    AppliedTick, AttackOutcome, ObserverOutput, RejectReason, SessionGeneration, ZoneCommand,
    ZoneEvent, ZoneInput,
};
use super::super::entity::{EntityId, EntityKind, Tick};
use super::super::fixed::{Fixed, Speed, Vec2Fixed};
use super::super::npc_template::{NpcTemplate, SpawnSlot};
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

fn template() -> NpcTemplate {
    parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].clone()
}

/// A one-member Keltir slot at tile `(x, y)`: aggressive, clan "keltir", 30 s + 0..=10 s.
fn slot(id: &str, x: i32, y: i32) -> SpawnSlotSpec {
    let t = template();
    let combat = NpcCombat::from_template(&rules(), &t).unwrap();
    SpawnSlotSpec::new(
        &SpawnSlot {
            id: id.into(),
            template: t.id.clone(),
            home: Vec2Fixed::from_tiles(x, y),
            count: 1,
            respawn_delay_secs: 30,
            respawn_random_secs: 10,
        },
        &t,
        combat,
    )
}

fn passive(mut s: SpawnSlotSpec) -> SpawnSlotSpec {
    s.brain.aggressive = false;
    s
}

fn rooted(mut s: SpawnSlotSpec) -> SpawnSlotSpec {
    s.speed = Speed::from_milli_tiles_per_tick(0);
    s
}

fn id(n: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(n))
}

fn zone_with_epoch(epoch: u64, slots: Vec<SpawnSlotSpec>) -> ZoneState {
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
    .with_spawn_slots(slots)
}

fn zone(slots: Vec<SpawnSlotSpec>) -> ZoneState {
    zone_with_epoch(3, slots)
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

fn set_target(n: u128, t: EntityId) -> ZoneInput {
    ZoneInput::session(
        id(n),
        GEN1,
        1,
        ZoneCommand::SetTarget {
            entity: id(n),
            target: Some(t),
        },
    )
}

fn add_aggro(npc: EntityId, n: u128) -> ZoneInput {
    ZoneInput::system(ZoneCommand::AddAggro { npc, target: id(n) })
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

/// The entity of slot `i`'s first member.
fn member(z: &ZoneState, slot: u16) -> EntityId {
    z.spawn_members()
        .find(|m| m.member.slot == slot && m.member.member == 0)
        .and_then(|m| m.entity)
        .unwrap()
}

fn ai(z: &ZoneState, npc: EntityId) -> &NpcAi {
    z.entities[&npc].ai.as_ref().unwrap()
}

fn intention(z: &ZoneState, npc: EntityId) -> Intention {
    ai(z, npc).intention
}

fn transitions(ticks: &[AppliedTick], npc: EntityId) -> Vec<(u64, Intention, Intention)> {
    ticks
        .iter()
        .flat_map(|t| t.events.iter())
        .filter_map(|e| match e {
            ZoneEvent::NpcIntentionChanged {
                tick,
                entity,
                from,
                to,
            } if *entity == npc => Some((tick.0, *from, *to)),
            _ => None,
        })
        .collect()
}

fn hate_of(z: &ZoneState, npc: EntityId) -> Vec<(EntityId, u64)> {
    z.hate_ledger(npc)
        .map(|l| l.iter().map(|(t, e)| (t, e.hate)).collect())
        .unwrap_or_default()
}

/// Moves an entity, keeping the AOI index consistent.
fn place(z: &mut ZoneState, e: EntityId, to: Vec2Fixed) {
    let from = z.entities[&e].pos;
    z.aoi.relocate(e, from, to);
    z.entities.get_mut(&e).unwrap().pos = to;
}

/// Kills `npc` as the impact phase would, on the tick about to run.
fn kill_now(z: &mut ZoneState, npc: EntityId) -> Tick {
    let death = z.next_tick;
    let mut events = Vec::new();
    z.kill(death, npc, None, &mut events);
    death
}

/// Independent `roll_below`: unbiased rejection over 32-bit words of a restored generator.
struct Oracle(ChaCha12Rng);

impl Oracle {
    fn at(rng: RngState) -> Self {
        let mut r = ChaCha12Rng::from_seed(rng.key);
        r.set_stream(rng.stream);
        r.set_word_pos(rng.word_pos);
        Self(r)
    }

    fn below(&mut self, n: u32) -> u32 {
        loop {
            let v = u64::from(self.0.next_u32());
            let limit = (1_u64 << 32) - (1_u64 << 32) % u64::from(n);
            if v < limit {
                return u32::try_from(v % u64::from(n)).unwrap();
            }
        }
    }

    fn word_pos(&self) -> u128 {
        self.0.get_word_pos()
    }
}

// ---------------------------------------------------------------------------------------
// E3.4 first lives and E3.2 transitions
// ---------------------------------------------------------------------------------------

#[test]
fn first_lives_spawn_on_tick_zero_idle_at_home_and_nothing_happens_without_players() {
    let mut a = slot("a", 100, 100);
    a.count = 2;
    let mut z = zone(vec![a, slot("b", 104, 102)]);
    let t0 = run(&mut z, Vec::new());
    let spawned: Vec<EntityId> = t0
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::EntitySpawn { entity, .. } => Some(*entity),
            _ => None,
        })
        .collect();
    let members: Vec<_> = z.spawn_members().copied().collect();
    assert_eq!(
        members
            .iter()
            .map(|m| (m.member.slot, m.member.member, m.incarnation, m.respawn_at))
            .collect::<Vec<_>>(),
        vec![(0, 0, 1, None), (0, 1, 1, None), (1, 0, 1, None)]
    );
    assert_eq!(
        members
            .iter()
            .map(|m| m.entity.unwrap())
            .collect::<Vec<_>>(),
        spawned,
        "slot members spawn in (slot, member) order"
    );
    for (npc, home) in spawned.iter().zip([(100, 100), (100, 100), (104, 102)]) {
        let e = &z.entities[npc];
        let c = e.combat.as_ref().unwrap();
        assert_eq!(e.pos, Vec2Fixed::from_tiles(home.0, home.1));
        assert_eq!((c.hp, c.incarnation), (c.sheet.max_hp(), 1));
        assert!(e.targeting.attackable);
        assert_eq!(intention(&z, *npc), Intention::Idle);
        assert_eq!(ai(&z, *npc).home, e.pos);
    }
    assert!(transitions(&[t0], spawned[0]).is_empty());
    let rng = z.snapshot().rng;
    assert!(idle(&mut z, 60).iter().all(|t| t.events.is_empty()));
    assert_eq!(z.snapshot().rng, rng, "idle NPCs draw nothing");
}

#[test]
fn idle_wakes_on_its_think_tick_when_a_player_is_nearby_and_sleeps_when_they_leave() {
    let mut z = zone(vec![passive(slot("a", 100, 100))]);
    run(&mut z, vec![spawn_player(1, 110, 100)]);
    let npc = member(&z, 0);
    let phase = NpcAi::think_phase(npc);
    let ticks = idle(&mut z, 10);
    let woke = transitions(&ticks, npc);
    assert_eq!(woke.len(), 1);
    assert_eq!(woke[0].0 % THINK_INTERVAL_TICKS, phase);
    assert_eq!((woke[0].1, woke[0].2), (Intention::Idle, Intention::Active));
    // No player in its AOI: Active → Idle on the next think.
    let gone = run(&mut z, vec![ZoneInput::system(ZoneCommand::Despawn { entity: id(1) })]);
    let mut ticks = vec![gone];
    ticks.extend(idle(&mut z, 10));
    let slept = transitions(&ticks, npc);
    assert_eq!(slept.len(), 1);
    assert_eq!(slept[0].0 % THINK_INTERVAL_TICKS, phase);
    assert_eq!((slept[0].1, slept[0].2), (Intention::Active, Intention::Idle));
}

#[test]
fn aggressive_npc_aggroes_the_nearest_player_with_one_hate_ties_to_the_lowest_id() {
    // Players 1 and 2 at equal distance (3 tiles); 2 is spawned first and sits west.
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(&mut z, vec![spawn_player(2, 97, 100), spawn_player(1, 103, 100)]);
    let npc = member(&z, 0);
    let ticks = idle(&mut z, 10);
    assert_eq!(hate_of(&z, npc), vec![(id(1), 1)]);
    assert_eq!(z.entities[&npc].targeting.target, Some(id(1)));
    let t = transitions(&ticks, npc);
    assert_eq!(
        t.iter().map(|(_, f, to)| (*f, *to)).collect::<Vec<_>>(),
        vec![
            (Intention::Idle, Intention::Active),
            (Intention::Active, Intention::Attack)
        ]
    );
    assert_eq!(t[0].0, t[1].0, "wake and aggro on the same think");

    // A nearer player wins regardless of id.
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(
        &mut z,
        vec![
            spawn_player(1, 97, 100),
            spawn_player(9, 100, 102),
            spawn_player(2, 100, 115),
        ],
    );
    let npc = member(&z, 0);
    idle(&mut z, 10);
    assert_eq!(hate_of(&z, npc), vec![(id(9), 1)]);
}

#[test]
fn passive_npc_never_aggroes_and_out_of_range_players_are_ignored() {
    let mut z = zone(vec![passive(slot("a", 100, 100)), slot("b", 160, 100)]);
    run(&mut z, vec![spawn_player(1, 101, 100), spawn_player(2, 167, 100)]);
    let (a, b) = (member(&z, 0), member(&z, 1));
    idle(&mut z, 40);
    assert!(hate_of(&z, a).is_empty());
    assert_eq!(intention(&z, a), Intention::Active);
    assert!(hate_of(&z, b).is_empty(), "7 tiles is outside a 6-tile aggro range");
    assert_eq!(intention(&z, b), Intention::Active);
}

#[test]
fn clan_call_helps_same_clan_in_range_once_in_id_order_without_recursion() {
    // The caller A aggroes the player; helpers are rooted and passive so only the call can
    // give them hate. B, G: same clan within 8 tiles of A. C: same clan, within 8 of B but 14
    // from A (recursion would reach it). D: other clan within range. E: same clan, 12.7 away.
    let helper = |id: &str, x, y| rooted(passive(slot(id, x, y)));
    let mut d = helper("d", 100, 107);
    d.brain.clan = Some("wolf".into());
    let mut z = zone(vec![
        slot("a", 100, 100),
        helper("b", 107, 100),
        helper("c", 114, 100),
        d,
        helper("e", 91, 91),
        helper("g", 100, 93),
    ]);
    run(&mut z, vec![spawn_player(1, 95, 100)]);
    let [a, b, c, d, e, g] = [0, 1, 2, 3, 4, 5].map(|s| member(&z, s));
    let ticks = idle(&mut z, 12);
    assert_eq!(hate_of(&z, a), vec![(id(1), 1)]);
    for helper in [b, g] {
        assert_eq!(hate_of(&z, helper), vec![(id(1), 1)]);
        assert_eq!(intention(&z, helper), Intention::Attack);
        assert!(ai(&z, helper).called_help);
    }
    for bystander in [c, d, e] {
        assert!(hate_of(&z, bystander).is_empty());
        assert_ne!(intention(&z, bystander), Intention::Attack);
    }
    // The call happens on A's aggro tick, allies in id order.
    let aggro_tick = transitions(&ticks, a)
        .iter()
        .find(|(_, _, to)| *to == Intention::Attack)
        .unwrap()
        .0;
    let helped: Vec<EntityId> = ticks
        .iter()
        .flat_map(|t| t.events.iter())
        .filter_map(|ev| match ev {
            ZoneEvent::HateChanged { npc, tick, .. } if *npc != a => {
                assert_eq!(tick.0, aggro_tick);
                Some(*npc)
            },
            _ => None,
        })
        .collect();
    let mut sorted = vec![b, g];
    sorted.sort();
    assert_eq!(helped, sorted);
}

#[test]
fn a_dead_target_re_evaluates_to_the_next_most_hated_then_an_empty_list_returns_home() {
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(&mut z, vec![spawn_player(1, 101, 100), spawn_player(2, 99, 100)]);
    let npc = member(&z, 0);
    idle(&mut z, 10);
    assert_eq!(z.entities[&npc].targeting.target, Some(id(1)));
    run(&mut z, vec![add_aggro(npc, 2)]);
    assert_eq!(hate_of(&z, npc), vec![(id(1), 1), (id(2), 1)]);
    z.entities
        .get_mut(&id(1))
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 1;
    let mut ticks = Vec::new();
    while !z.entities[&id(1)].targeting.dead {
        ticks.push(run(&mut z, Vec::new()));
        assert!(ticks.len() < 300, "the keltir eventually lands a hit");
    }
    assert_eq!(z.entities[&npc].targeting.target, Some(id(2)));
    assert_eq!(intention(&z, npc), Intention::Attack);
    assert!(transitions(&ticks, npc).is_empty(), "re-evaluation keeps Attack");
    let t = run(&mut z, vec![ZoneInput::system(ZoneCommand::Despawn { entity: id(2) })]);
    let next = run(&mut z, Vec::new());
    assert_eq!(
        transitions(&[t, next], npc)
            .iter()
            .map(|(_, f, to)| (*f, *to))
            .collect::<Vec<_>>(),
        vec![
            (Intention::Attack, Intention::ReturnHome),
            (Intention::ReturnHome, Intention::Active)
        ],
        "already home: arrives on the next tick"
    );
}

#[test]
fn landed_hits_reset_the_attack_timeout_and_misses_do_not() {
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(&mut z, vec![spawn_player(1, 101, 100)]);
    let npc = member(&z, 0);
    let ticks = idle(&mut z, 200);
    let mut last_landed = None;
    for t in &ticks {
        for e in &t.events {
            if let ZoneEvent::AttackResult {
                attacker, outcome, ..
            } = e
            {
                if *attacker == npc && *outcome != AttackOutcome::Miss {
                    last_landed = Some(t.tick);
                }
            }
        }
    }
    assert_eq!(Some(ai(&z, npc).last_hit), last_landed);
}

#[test]
fn rooted_npc_gives_up_after_1200_ticks_without_a_hit_and_heals_on_the_spot() {
    // Speed 0: it can never reach a player 4 tiles away.
    let mut z = zone(vec![rooted(slot("a", 100, 100))]);
    run(&mut z, vec![spawn_player(1, 104, 100)]);
    let npc = member(&z, 0);
    z.entities
        .get_mut(&npc)
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 3;
    let ticks = idle(&mut z, 1215);
    let t = transitions(&ticks, npc);
    let attack = t
        .iter()
        .find(|(_, _, to)| *to == Intention::Attack)
        .unwrap()
        .0;
    let back = t
        .iter()
        .find(|(_, _, to)| *to == Intention::ReturnHome)
        .unwrap()
        .0;
    assert_eq!(back - attack, MAX_ATTACK_TIMEOUT_TICKS);
    let home = t
        .iter()
        .find(|(_, _, to)| *to == Intention::Active && t.iter().any(|(b, _, _)| b == &back))
        .map(|x| x.0);
    assert!(t.contains(&(back + 1, Intention::ReturnHome, Intention::Active)), "{home:?}");
    let retired = &ticks[usize::try_from(back - ticks[0].tick.0).unwrap()];
    assert!(retired.events.contains(&ZoneEvent::HateChanged {
        tick: Tick(back),
        npc,
        target: id(1),
        hate: 0,
        damage: 0,
    }));
    let c = z.entities[&npc].combat.as_ref().unwrap();
    assert_eq!(c.hp, c.sheet.max_hp());
    assert_eq!(z.entities[&npc].pos, Vec2Fixed::from_tiles(100, 100));
}

#[test]
fn chase_stops_at_attack_range() {
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(&mut z, vec![spawn_player(1, 105, 100)]);
    let npc = member(&z, 0);
    let player = Vec2Fixed::from_tiles(105, 100);
    for _ in 0..60 {
        let t = run(&mut z, Vec::new());
        let swung = t
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::AttackStarted { attacker, .. } if *attacker == npc));
        let e = &z.entities[&npc];
        let reach = e.combat.as_ref().unwrap().attack_range;
        if swung {
            assert!(e.pos.within(player, reach));
            assert!(!e.pos.within(player, Fixed::from_raw(reach.raw() - 400)));
            assert_eq!(e.dest, None);
            return;
        }
        assert!(!e.pos.within(player, Fixed::from_raw(reach.raw() - 1)) || e.dest.is_none());
    }
    panic!("the keltir never swung");
}

#[test]
fn leash_breach_returns_home_untouchable_without_hate_and_full_hp_on_exact_arrival() {
    let mut s = slot("a", 100, 100);
    s.brain.leash_radius = Fixed::from_tiles(3);
    let mut z = zone(vec![s]);
    run(&mut z, vec![spawn_player(1, 105, 100)]);
    let npc = member(&z, 0);
    let home = Vec2Fixed::from_tiles(100, 100);
    z.entities
        .get_mut(&npc)
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 5;
    let mut ticks = Vec::new();
    while intention(&z, npc) != Intention::ReturnHome {
        ticks.push(run(&mut z, Vec::new()));
        assert!(ticks.len() < 40);
    }
    let e = &z.entities[&npc];
    // It breached the 3-tile leash by at most one step, then took its first step home.
    assert!(!e.pos.within(home, Fixed::from_raw(2600)));
    assert!(e.pos.within(home, Fixed::from_raw(3400)), "at most one step past the leash");
    assert!(z.hate_ledger(npc).is_none());
    assert_eq!(e.targeting.target, None);
    assert!(!e.targeting.attackable);
    assert_eq!(e.dest, Some(home));
    // Returning NPCs cannot be targeted, damaged or aggroed.
    let t = run(&mut z, vec![set_target(1, npc), add_aggro(npc, 1)]);
    assert_eq!(
        reasons(&t),
        vec![
            RejectReason::NonAttackableTarget,
            RejectReason::NotPermitted
        ]
    );
    // The player leaves: an empty region does not stop the return.
    run(&mut z, vec![ZoneInput::system(ZoneCommand::Despawn { entity: id(1) })]);
    let mut last = z.entities[&npc].pos.distance_sq(home);
    for _ in 0..30 {
        run(&mut z, Vec::new());
        let d = z.entities[&npc].pos.distance_sq(home);
        assert!(d < last || d == 0);
        last = d;
        if intention(&z, npc) == Intention::Active {
            break;
        }
    }
    let e = &z.entities[&npc];
    let c = e.combat.as_ref().unwrap();
    assert_eq!(e.pos, home);
    assert_eq!(intention(&z, npc), Intention::Active);
    assert_eq!(c.hp, c.sheet.max_hp());
    assert!(e.targeting.attackable);
}

#[test]
fn active_npc_wanders_one_think_in_thirty_within_drift_range_clamped_to_the_zone() {
    // Home in the corner, so the clamp is exercised on both axes.
    let mut z = zone(vec![passive(slot("a", 1, 1))]);
    run(&mut z, vec![spawn_player(1, 20, 20)]);
    let npc = member(&z, 0);
    let home = Vec2Fixed::from_tiles(1, 1);
    let phase = NpcAi::think_phase(npc);
    let (mut thinks, mut wanders) = (0, 0);
    for _ in 0..3000 {
        let before = z.snapshot();
        let tick = z.next_tick;
        let t = run(&mut z, Vec::new());
        let e = &z.entities[&npc];
        assert!(z.bounds.contains(e.pos));
        if tick.0 % THINK_INTERVAL_TICKS != phase {
            assert_eq!(before.rng, z.snapshot().rng, "draws only on think ticks");
            continue;
        }
        thinks += 1;
        let mut o = Oracle::at(before.rng);
        if o.below(RANDOM_WALK_RATE) == 0 {
            let r = MAX_DRIFT_RANGE.raw();
            let w = u32::try_from(2 * r + 1).unwrap();
            let dx = i32::try_from(o.below(w)).unwrap() - r;
            let dy = i32::try_from(o.below(w)).unwrap() - r;
            let want = Vec2Fixed::new(
                Fixed::from_raw((home.x.raw() + dx).clamp(0, 256_000)),
                Fixed::from_raw((home.y.raw() + dy).clamp(0, 256_000)),
            );
            assert!((want.x.raw() - home.x.raw()).abs() <= r);
            assert!((want.y.raw() - home.y.raw()).abs() <= r);
            let from = before.entities.iter().find(|x| x.id == npc).unwrap().pos;
            if want != from {
                wanders += 1;
                let moved = t.events.iter().find_map(|ev| match ev {
                    ZoneEvent::EntityMove {
                        entity, pos, dest, ..
                    } if *entity == npc => Some((*pos, *dest)),
                    _ => None,
                });
                let (pos, dest) = moved.unwrap();
                assert!(dest == Some(want) || (dest.is_none() && pos == want));
            }
        }
        assert_eq!(z.snapshot().rng.word_pos, o.word_pos(), "documented draw order");
    }
    assert_eq!(thinks, 300);
    assert!(wanders >= 3, "1 in 30 over 300 thinks; got {wanders}");
}

// ---------------------------------------------------------------------------------------
// E3.3 property tests
// ---------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Every step gets strictly closer, stays inside the box spanned by the step's start and
    /// the destination (no overshoot), covers at most `speed`, and the walk ends exactly there.
    #[test]
    fn step_toward_progresses_without_overshoot(
        fx in 0..20_000_i32, fy in 0..20_000_i32,
        tx in 0..20_000_i32, ty in 0..20_000_i32,
        speed in 0..2_000_u32,
    ) {
        let dest = Vec2Fixed::new(Fixed::from_raw(tx), Fixed::from_raw(ty));
        let mut p = Vec2Fixed::new(Fixed::from_raw(fx), Fixed::from_raw(fy));
        let s = Speed::from_milli_tiles_per_tick(speed);
        if speed == 0 {
            prop_assert_eq!(p.step_toward(dest, s), p);
            return Ok(());
        }
        let mut steps = 0_u32;
        while p != dest {
            let next = p.step_toward(dest, s);
            prop_assert!(next.distance_sq(dest) < p.distance_sq(dest));
            prop_assert!(next.distance_sq(p) <= u128::from(speed) * u128::from(speed));
            let (lo_x, hi_x) = (p.x.min(dest.x), p.x.max(dest.x));
            let (lo_y, hi_y) = (p.y.min(dest.y), p.y.max(dest.y));
            prop_assert!(lo_x <= next.x && next.x <= hi_x && lo_y <= next.y && next.y <= hi_y);
            p = next;
            steps += 1;
            prop_assert!(steps <= 60_000);
        }
    }

    /// A returning NPC from anywhere in the zone gets strictly closer every tick, arrives
    /// exactly at home with full HP, and a snapshot taken mid-path (round-tripped through the
    /// log encoding) reproduces the remaining ticks byte for byte.
    #[test]
    fn return_home_arrives_exactly_and_mid_path_restore_replays_identically(
        x in 0..=256_000_i32, y in 0..=256_000_i32, cut in 0..100_u32,
    ) {
        let mut z = zone(vec![slot("a", 128, 128)]);
        run(&mut z, Vec::new());
        let npc = member(&z, 0);
        let home = Vec2Fixed::from_tiles(128, 128);
        place(&mut z, npc, Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y)));
        {
            let e = z.entities.get_mut(&npc).unwrap();
            e.targeting.attackable = false;
            e.combat.as_mut().unwrap().hp = 1;
            e.ai.as_mut().unwrap().intention = Intention::ReturnHome;
        }
        let total = usize::try_from(z.entities[&npc].pos.distance_sq(home).isqrt() / 400).unwrap() + 3;
        let cut = total * usize::try_from(cut).unwrap() / 100;
        let mut a = z.clone();
        let mut last = a.entities[&npc].pos.distance_sq(home);
        let mut first = Vec::new();
        for _ in 0..cut {
            first.push(run(&mut a, Vec::new()));
            let d = a.entities[&npc].pos.distance_sq(home);
            prop_assert!(d < last || d == 0);
            last = d;
        }
        let json = encode_snapshot(&a.snapshot()).unwrap();
        let mut b = ZoneState::from_snapshot(decode_snapshot(&json).unwrap()).unwrap();
        for _ in cut..total {
            let ta = run(&mut a, Vec::new());
            let tb = run(&mut b, Vec::new());
            prop_assert_eq!(
                AppliedTickRecord::from_applied(ZoneId(7), &ta).encode(),
                AppliedTickRecord::from_applied(ZoneId(7), &tb).encode()
            );
            prop_assert_eq!(&ta, &tb);
            let d = a.entities[&npc].pos.distance_sq(home);
            prop_assert!(d < last || d == 0);
            last = d;
        }
        let e = &a.entities[&npc];
        let c = e.combat.as_ref().unwrap();
        prop_assert_eq!(e.pos, home);
        prop_assert_eq!(c.hp, c.sheet.max_hp());
        prop_assert!(matches!(intention(&a, npc), Intention::Active | Intention::Idle));
    }
}

// ---------------------------------------------------------------------------------------
// E3.4 corpse and respawn scheduler
// ---------------------------------------------------------------------------------------

#[test]
fn respawn_jitter_covers_min_and_max_from_the_seeded_stream_and_zero_spread_draws_nothing() {
    let mut seen = BTreeSet::new();
    for epoch in 0..60 {
        let mut s = slot("a", 100, 100);
        (s.respawn_delay_secs, s.respawn_random_secs) = (3, 2);
        s.brain.corpse_decay_ticks = 5;
        let mut z = zone_with_epoch(epoch, vec![s]);
        run(&mut z, Vec::new());
        let npc = member(&z, 0);
        let rng = z.snapshot().rng;
        let death = kill_now(&mut z, npc);
        let mut o = Oracle::at(rng);
        let jitter = u64::from(o.below(3));
        assert_eq!(z.snapshot().rng.word_pos, o.word_pos(), "one jitter draw per death");
        let m = z.spawn_members().next().copied().unwrap();
        assert_eq!(m.respawn_at, Some(Tick(death.0 + 10 * (3 + jitter))));
        seen.insert(jitter);
    }
    assert_eq!(seen, BTreeSet::from([0, 1, 2]), "inclusive min and max both occur");

    let mut s = slot("a", 100, 100);
    s.respawn_random_secs = 0;
    let mut z = zone(vec![s]);
    run(&mut z, Vec::new());
    let npc = member(&z, 0);
    let rng = z.snapshot().rng;
    let death = kill_now(&mut z, npc);
    assert_eq!(z.snapshot().rng, rng, "zero spread consumes no draw");
    assert_eq!(z.spawn_members().next().unwrap().respawn_at, Some(Tick(death.0 + 300)));
}

#[test]
fn corpse_decays_before_the_respawn_even_when_the_delay_is_shorter() {
    let mut s = slot("a", 100, 100);
    (s.respawn_delay_secs, s.respawn_random_secs) = (1, 0);
    s.brain.corpse_decay_ticks = 40;
    let mut z = zone(vec![s]);
    run(&mut z, Vec::new());
    let npc = member(&z, 0);
    let death = kill_now(&mut z, npc);
    assert_eq!(z.spawn_members().next().unwrap().respawn_at, Some(Tick(death.0 + 41)));
}

#[test]
fn death_corpse_decay_and_respawn_make_a_new_incarnation_and_reject_stale_targets() {
    let mut s = slot("a", 100, 100);
    (s.respawn_delay_secs, s.respawn_random_secs) = (3, 0);
    s.brain.corpse_decay_ticks = 20;
    let mut z = zone(vec![s]);
    // An observer 30 tiles away: in AOI, out of aggro range.
    run(&mut z, vec![spawn_player(1, 130, 100)]);
    let npc = member(&z, 0);
    idle(&mut z, 5);
    let death = kill_now(&mut z, npc);
    assert_eq!(intention(&z, npc), Intention::Dead);
    assert_eq!(ai(&z, npc).corpse_until, Some(Tick(death.0 + 20)));
    let mut ticks = Vec::new();
    while z.next_tick.0 < death.0 + 20 {
        let t = run(&mut z, vec![set_target(1, npc)]);
        assert_eq!(reasons(&t), vec![RejectReason::NonAttackableTarget], "a corpse");
        ticks.push(t);
    }
    // Corpse expiry: gone from the world and from the observer's AOI.
    let t = run(&mut z, Vec::new());
    assert!(t.events.contains(&ZoneEvent::EntityDespawn {
        tick: Tick(death.0 + 20),
        entity: npc
    }));
    assert!(t.outputs[&id(1)].contains(&ObserverOutput::Event(ZoneEvent::EntityDespawn {
        tick: Tick(death.0 + 20),
        entity: npc
    })));
    assert!(z.entity(npc).is_none());
    // Stale references to the old life: unknown.
    let t = run(&mut z, vec![set_target(1, npc), add_aggro(npc, 1)]);
    assert_eq!(reasons(&t), vec![RejectReason::UnknownEntity, RejectReason::UnknownEntity]);
    while z.next_tick.0 < death.0 + 30 {
        let t = run(&mut z, Vec::new());
        assert!(t.events.is_empty(), "no players near a hidden member: nothing happens");
    }
    // The new life: same id, next incarnation, full stats, Idle at home.
    let t = run(&mut z, Vec::new());
    let tick = Tick(death.0 + 30);
    let max = z.entities[&npc].combat.as_ref().unwrap().sheet.max_hp();
    assert!(t.events.contains(&ZoneEvent::EntityRespawned {
        entity: npc,
        tick,
        position: Vec2Fixed::from_tiles(100, 100),
        hp: max,
        incarnation: 2,
    }));
    assert!(t.events.contains(&ZoneEvent::NpcIntentionChanged {
        tick,
        entity: npc,
        from: Intention::Dead,
        to: Intention::Idle,
    }));
    let out = &t.outputs[&id(1)];
    let spawn = out
        .iter()
        .position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, combat: Some(c), .. }) if *entity == npc && c.incarnation == 2))
        .unwrap();
    let respawned = out
        .iter()
        .position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntityRespawned { .. })))
        .unwrap();
    assert!(spawn < respawned, "spawn precedes the fact about it");
    let e = &z.entities[&npc];
    let c = e.combat.as_ref().unwrap();
    assert_eq!((c.incarnation, c.hp, e.targeting.dead), (2, max, false));
    let m = z.spawn_members().next().copied().unwrap();
    assert_eq!((m.entity, m.incarnation, m.respawn_at), (Some(npc), 2, None));
    let t = run(&mut z, vec![set_target(1, npc)]);
    assert!(t.dispositions.is_empty(), "the new life is targetable");
}

#[test]
fn a_member_never_spawns_twice() {
    let mut a = slot("a", 100, 100);
    a.count = 2;
    let mut z = zone(vec![a]);
    run(&mut z, Vec::new());
    // Forge a due respawn for a living member.
    let first = z.members.keys().next().copied().unwrap();
    z.members.get_mut(&first).unwrap().respawn_at = Some(z.next_tick);
    let t = run(&mut z, Vec::new());
    assert!(!t
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::EntitySpawn { .. })));
    assert_eq!(z.entity_count(), 2);
    assert_eq!(z.members[&first].respawn_at, None);
    assert_eq!(z.members[&first].incarnation, 1);
}

#[test]
fn snapshots_during_dead_restore_and_continue_identically() {
    let mut s = slot("a", 100, 100);
    (s.respawn_delay_secs, s.respawn_random_secs) = (2, 3);
    s.brain.corpse_decay_ticks = 8;
    let mut z = zone(vec![s, slot("b", 104, 102)]);
    run(&mut z, vec![spawn_player(1, 120, 100)]);
    let npc = member(&z, 0);
    kill_now(&mut z, npc);
    // Every boundary from corpse through hidden to the new life.
    for _ in 0..60 {
        let json = encode_snapshot(&z.snapshot()).unwrap();
        let mut b = ZoneState::from_snapshot(decode_snapshot(&json).unwrap()).unwrap();
        assert_eq!(b, z);
        let (ta, tb) = (idle(&mut z.clone(), 60), idle(&mut b, 60));
        assert_eq!(ta, tb);
        run(&mut z, Vec::new());
    }
    assert_eq!(z.entities[&npc].combat.as_ref().unwrap().incarnation, 2);
}

#[test]
fn same_seed_same_ai_and_respawn_sequence() {
    let make = || {
        let mut s = slot("a", 100, 100);
        (s.respawn_delay_secs, s.respawn_random_secs) = (1, 5);
        s.brain.corpse_decay_ticks = 3;
        zone(vec![s, slot("b", 104, 102)])
    };
    let script = |z: &mut ZoneState| {
        let mut out = vec![run(
            z,
            vec![spawn_player(1, 99, 100), spawn_player(2, 106, 102)],
        )];
        for i in 0..400 {
            let npc = member(z, 0);
            if i % 100 == 50 && z.entity(npc).is_some_and(|e| !e.targeting.dead) {
                kill_now(z, npc);
            }
            out.push(run(z, Vec::new()));
        }
        out
    };
    let (mut a, mut b) = (make(), make());
    assert_eq!(script(&mut a), script(&mut b));
    assert_eq!(a.state_digest(), b.state_digest());
}

#[test]
fn snapshots_with_an_inconsistent_scheduler_are_refused() {
    let mut z = zone(vec![slot("a", 100, 100), slot("b", 104, 102)]);
    run(&mut z, Vec::new());
    let snap = z.snapshot();
    assert_eq!(snap.meta.schema_version, SNAPSHOT_SCHEMA_VERSION);
    assert!(ZoneState::from_snapshot(snap.clone()).is_ok());
    let mut missing = snap.clone();
    missing.spawn_members.pop();
    let mut duplicate = snap.clone();
    duplicate.spawn_members[1] = duplicate.spawn_members[0];
    let mut foreign = snap.clone();
    foreign.spawn_members[0].entity = Some(id(77));
    let mut no_slot = snap.clone();
    no_slot.spawn_slots.pop();
    for bad in [missing, duplicate, foreign, no_slot] {
        assert_eq!(ZoneState::from_snapshot(bad), Err(SnapshotError::SpawnMismatch));
    }
    let mut no_rules = snap;
    no_rules.rules = None;
    no_rules.entities.clear();
    no_rules.aoi.clear();
    assert_eq!(ZoneState::from_snapshot(no_rules), Err(SnapshotError::CombatMismatch));
}

#[test]
fn intention_changes_round_trip_through_the_record_codec_and_reach_no_client() {
    let mut z = zone(vec![slot("a", 100, 100)]);
    run(&mut z, vec![spawn_player(1, 103, 100)]);
    let ticks = idle(&mut z, 10);
    let t = ticks
        .iter()
        .find(|t| {
            t.events
                .iter()
                .any(|e| matches!(e, ZoneEvent::NpcIntentionChanged { .. }))
        })
        .unwrap();
    let record = AppliedTickRecord::from_applied(ZoneId(7), t);
    assert_eq!(crate::application::replay_log::decode_events(&record.events).unwrap(), t.events);
    assert!(!t
        .outputs
        .values()
        .flatten()
        .any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::NpcIntentionChanged { .. }))));
    assert!(t.events.iter().all(|e| e.entity() != id(0)));
    assert_eq!(z.entities[&id(1)].kind, EntityKind::Player);
}
