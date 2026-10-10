//! Determinism properties of the zone (plan D7, §8 #3, #5, #6): the same seed and inputs give
//! the same `AppliedTick` stream across actors and runs; recorded drafts replay to the same
//! records; a serialised snapshot continues identically; movement arrives exactly; per-player
//! output order is canonical.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::wildcard_enum_match_arm
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use nightfall_api::application::zone_actor::{manual_ticks, ZoneActor};
use nightfall_api::domain::zone::{
    AppliedTick, AppliedTickDraft, EntityId, Fixed, ObserverOutput, SessionGeneration, Speed, Tick,
    Vec2Fixed, ZoneBounds, ZoneCommand, ZoneEvent, ZoneId, ZoneInput, ZoneSeed, ZoneSnapshot,
    ZoneState,
};
use proptest::prelude::*;
use uuid::Uuid;

/// 256x256 tiles, so entities can be outside each other's 3x3 AOI.
fn fresh(epoch: u64) -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    ZoneState::new(
        ZoneSeed {
            zone: ZoneId(11),
            epoch,
        },
        bounds,
        1_700_000_000_000,
    )
}

fn id(n: u8) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(u128::from(n) + 1))
}

fn point() -> impl Strategy<Value = Vec2Fixed> {
    // A little outside the bounds on every side, so rejections are exercised.
    (-5_000..262_000_i32, -5_000..262_000_i32)
        .prop_map(|(x, y)| Vec2Fixed::new(Fixed::from_raw(x), Fixed::from_raw(y)))
}

fn input() -> impl Strategy<Value = ZoneInput> {
    let who = 0..4_u8;
    let generation = (1..3_u64).prop_map(SessionGeneration);
    prop_oneof![
        (who.clone(), point(), 0..1500_u32, generation.clone()).prop_map(|(n, pos, s, g)| {
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: id(n),
                name: format!("p{n}"),
                pos,
                speed: Speed::from_milli_tiles_per_tick(s),
                generation: g,
                load: None,
            })
        }),
        (point(), 0..1500_u32).prop_map(|(pos, s)| ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "npc".to_owned(),
            pos,
            speed: Speed::from_milli_tiles_per_tick(s),
            combat: None,
        })),
        who.clone()
            .prop_map(|n| ZoneInput::system(ZoneCommand::Despawn { entity: id(n) })),
        (who.clone(), who.clone(), generation.clone(), point(), any::<u32>()).prop_map(
            |(src, n, g, dest, seq)| ZoneInput::session(
                id(src),
                g,
                seq,
                ZoneCommand::MoveTo {
                    entity: id(n),
                    dest
                }
            )
        ),
        (who.clone(), point()).prop_map(|(n, dest)| ZoneInput::system(ZoneCommand::MoveTo {
            entity: id(n),
            dest
        })),
        (who.clone(), generation.clone(), any::<u32>()).prop_map(|(n, g, seq)| {
            ZoneInput::session(id(n), g, seq, ZoneCommand::StopMove { entity: id(n) })
        }),
        (who, 1..4_u64).prop_map(|(n, g)| ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: id(n),
            generation: SessionGeneration(g),
        })),
    ]
}

/// Inputs per tick. Long enough for entities to walk across AOI cells.
fn script() -> impl Strategy<Value = Vec<Vec<ZoneInput>>> {
    prop::collection::vec(prop::collection::vec(input(), 0..6), 1..60)
}

/// Runs a script through a real actor with manual ticks; returns every broadcast record.
async fn run_actor(state: ZoneState, script: &[Vec<ZoneInput>]) -> Vec<Arc<AppliedTick>> {
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn(state, ticks);
    let mut rx = zone.subscribe();
    let mut out = Vec::new();
    for inputs in script {
        for i in inputs {
            zone.send(i.clone()).unwrap();
        }
        driver.step().await.unwrap();
        while let Ok(t) = rx.try_recv() {
            out.push(t);
        }
    }
    out
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Every record serialised, so "identical" means identical bytes, not just `PartialEq`.
fn bytes(records: &[Arc<AppliedTick>]) -> Vec<u8> {
    serde_json::to_vec(&records.iter().map(AsRef::as_ref).collect::<Vec<_>>()).unwrap()
}

/// See `domain::zone::state` tests: per player, own dispositions by ordinal, then AOI
/// despawns, spawns, moves, each in entity-id order. Players in id order is the `BTreeMap`.
fn assert_output_order(t: &AppliedTick) {
    for out in t.outputs.values() {
        let rank = |o: &ObserverOutput| match o {
            ObserverOutput::Rejected(d) => (0, d.ordinal.0, None),
            ObserverOutput::Accepted { ordinal, .. } => (0, ordinal.0, None),
            ObserverOutput::Event(ZoneEvent::EntityDespawn { entity, .. }) => (1, 0, Some(*entity)),
            ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) => (2, 0, Some(*entity)),
            ObserverOutput::Event(ZoneEvent::EntityMove { entity, .. }) => (3, 0, Some(*entity)),
            ObserverOutput::Event(
                _e @ (ZoneEvent::AttackResult { .. }
                | ZoneEvent::EntityDied { .. }
                | ZoneEvent::EntityRespawned { .. }
                | ZoneEvent::ClassChanged { .. }
                | ZoneEvent::ClassTransfer { .. }
                | ZoneEvent::StatsChanged { .. }
                | ZoneEvent::XpGained { .. }
                | ZoneEvent::LevelUp { .. }
                | ZoneEvent::TargetChanged { .. }
                | ZoneEvent::AttackStarted { .. }
                | ZoneEvent::AttackCancelled { .. }
                | ZoneEvent::HateChanged { .. }
                | ZoneEvent::NpcIntentionChanged { .. }
                | ZoneEvent::Progression(_)),
            ) => (4, 0, None),
        };
        let mut ranks: Vec<_> = out.iter().map(rank).collect();
        // Facts keep their causal order: rank them by position.
        for (i, r) in ranks.iter_mut().enumerate() {
            if r.0 == 4 {
                r.1 = u64::try_from(i).unwrap();
            }
        }
        let mut canonical = ranks.clone();
        canonical.sort();
        canonical.dedup();
        assert_eq!(ranks, canonical, "tick {:?}: output order not canonical", t.tick);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn same_seed_and_inputs_give_identical_streams_across_actors_and_runs(script in script()) {
        let rt = runtime();
        let (a, b) = rt.block_on(async {
            tokio::join!(run_actor(fresh(1), &script), run_actor(fresh(1), &script))
        });
        let c = runtime().block_on(run_actor(fresh(1), &script));
        prop_assert_eq!(bytes(&a), bytes(&b));
        prop_assert_eq!(bytes(&a), bytes(&c));
        for t in &a {
            assert_output_order(t);
        }
    }

    #[test]
    fn recorded_drafts_replay_to_identical_records(script in script()) {
        let recorded = runtime().block_on(run_actor(fresh(2), &script));
        let by_tick: BTreeMap<Tick, &AppliedTick> =
            recorded.iter().map(|t| (t.tick, t.as_ref())).collect();
        let mut replay = fresh(2);
        let mut replayed = Vec::new();
        for n in 0..script.len() {
            let tick = Tick(u64::try_from(n).unwrap());
            let commands = by_tick.get(&tick).map(|t| t.commands.clone()).unwrap_or_default();
            let t = replay.run_tick(AppliedTickDraft { epoch: 2, tick, commands }).unwrap();
            if !t.is_idle() {
                replayed.push(Arc::new(t));
            }
        }
        prop_assert_eq!(bytes(&replayed), bytes(&recorded));
    }

    #[test]
    fn a_serialised_snapshot_continues_identically(
        before in script(),
        after in script(),
    ) {
        let rt = runtime();
        let (original_tail, restored_tail) = rt.block_on(async {
            let (ticks, driver) = manual_ticks();
            let zone = ZoneActor::spawn(fresh(3), ticks);
            for inputs in &before {
                for i in inputs {
                    zone.send(i.clone()).unwrap();
                }
                driver.step().await.unwrap();
            }
            // Flush anything deferred by the session budget so the boundary is clean.
            let snap = loop {
                if zone.stats().borrow().commands_deferred == 0 {
                    break zone.snapshot().await.unwrap();
                }
                driver.step().await.unwrap();
            };
            let json = serde_json::to_string(&snap).unwrap();
            let restored: ZoneSnapshot = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, snap);

            let mut rx = zone.subscribe();
            let mut original = Vec::new();
            for inputs in &after {
                for i in inputs {
                    zone.send(i.clone()).unwrap();
                }
                driver.step().await.unwrap();
                while let Ok(t) = rx.try_recv() {
                    original.push(t);
                }
            }
            let state = ZoneState::from_snapshot(restored).unwrap();
            (original, run_actor(state, &after).await)
        });
        prop_assert_eq!(bytes(&original_tail), bytes(&restored_tail));
    }

    #[test]
    fn movement_never_overshoots_and_arrives_exactly(
        start in (-1_000_000..1_000_000_i32, -1_000_000..1_000_000_i32),
        delta in (-20_000..20_000_i32, -20_000..20_000_i32),
        speed in prop_oneof![1..=3_u32, 1..=20_000_u32],
    ) {
        let pos = Vec2Fixed::new(Fixed::from_raw(start.0), Fixed::from_raw(start.1));
        let dest = Vec2Fixed::new(
            Fixed::from_raw(start.0 + delta.0),
            Fixed::from_raw(start.1 + delta.1),
        );
        walk(pos, dest, Speed::from_milli_tiles_per_tick(speed))?;
    }

    #[test]
    fn movement_is_exact_at_the_i32_extremes(
        a in (any::<i32>(), any::<i32>()),
        b in (any::<i32>(), any::<i32>()),
        speed in (1_u32 << 22)..=u32::MAX,
    ) {
        let pos = Vec2Fixed::new(Fixed::from_raw(a.0), Fixed::from_raw(a.1));
        let dest = Vec2Fixed::new(Fixed::from_raw(b.0), Fixed::from_raw(b.1));
        walk(pos, dest, Speed::from_milli_tiles_per_tick(speed))?;
    }
}

/// Steps from `pos` to `dest`, checking every step: the remaining distance strictly
/// decreases, no step is longer than `speed`, neither axis passes `dest`, and the walk ends
/// exactly on `dest`.
fn walk(mut pos: Vec2Fixed, dest: Vec2Fixed, speed: Speed) -> Result<(), TestCaseError> {
    let s = u128::from(speed.milli_tiles_per_tick());
    while pos != dest {
        let next = pos.step_toward(dest, speed);
        prop_assert!(next.distance_sq(dest) < pos.distance_sq(dest), "no progress at {pos:?}");
        prop_assert!(pos.distance_sq(next) <= s.saturating_mul(s), "step longer than speed");
        for (p, n, d) in [(pos.x, next.x, dest.x), (pos.y, next.y, dest.y)] {
            prop_assert!(p.min(d) <= n && n <= p.max(d), "axis passed the destination");
        }
        pos = next;
    }
    prop_assert_eq!(pos, dest);
    Ok(())
}

// ---- Combat (Phase 1 E2.2, E2.3, E2.5) --------------------------------------------------------

/// The embedded HF rules, loaded once.
fn rules() -> Arc<nightfall_api::domain::zone::StatRules> {
    static RULES: std::sync::OnceLock<Arc<nightfall_api::domain::zone::StatRules>> =
        std::sync::OnceLock::new();
    RULES
        .get_or_init(|| {
            use nightfall_api::infrastructure::rules_data::{load_rules, RulesSource};
            load_rules(&RulesSource::embedded()).unwrap().rules
        })
        .clone()
}

/// A combat zone after a fixed setup tick: three players and three Keltirs within a few
/// tiles of each other. Player 0 enters nearly dead and with XP to lose, so fights cover
/// death, de-level and respawn (E2.4, E2.6).
/// Returns the zone and the monsters' (seeded) ids.
fn arena(epoch: u64) -> (ZoneState, Vec<EntityId>) {
    use nightfall_api::domain::zone::{NpcCombat, PlayerLoad};
    use nightfall_api::infrastructure::zone_data::{parse_zone, TEST_ZONE_TOML};
    let def = parse_zone(TEST_ZONE_TOML).unwrap();
    let keltir = NpcCombat::from_template(&rules(), &def.npc_templates[0]).unwrap();
    let mut z = fresh(epoch)
        .with_rules(rules())
        .with_safe_point(Vec2Fixed::from_tiles(14, 14));
    let mut setup: Vec<ZoneInput> = (0..3_u8)
        .map(|n| {
            ZoneInput::system(ZoneCommand::SpawnPlayer {
                entity: id(n),
                name: format!("p{n}"),
                pos: Vec2Fixed::from_tiles(10 + i32::from(n), 10),
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(if n == 0 {
                    PlayerLoad {
                        level: 2,
                        xp: 70,
                        hp: Some(5),
                        ..PlayerLoad::fresh("human_fighter")
                    }
                } else {
                    PlayerLoad::fresh("human_fighter")
                })),
            })
        })
        .collect();
    setup.extend((0..3).map(|n| {
        ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Keltir".into(),
            pos: Vec2Fixed::from_tiles(12 + n, 12),
            speed: Speed::from_milli_tiles_per_tick(400),
            combat: Some(Box::new(keltir.clone())),
        })
    }));
    let draft = z.draft(setup);
    let t = z.run_tick(draft).unwrap();
    let npcs = t
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::EntitySpawn {
                entity,
                kind: nightfall_api::domain::zone::EntityKind::Npc,
                ..
            } => Some(*entity),
            _ => None,
        })
        .collect();
    (z, npcs)
}

/// An abstract combat step; NPCs are named by index into the arena's monsters.
#[derive(Debug, Clone)]
enum Op {
    Target(u8, Option<usize>),
    Attack(u8),
    Stop(u8),
    Move(u8, i32, i32),
    Aggro(usize, u8),
    Leave(u8),
    Respawn(u8),
}

fn op() -> impl Strategy<Value = Op> {
    let p = 0..3_u8;
    let m = 0..3_usize;
    prop_oneof![
        3 => (p.clone(), proptest::option::of(m.clone())).prop_map(|(p, m)| Op::Target(p, m)),
        3 => p.clone().prop_map(Op::Attack),
        1 => p.clone().prop_map(Op::Stop),
        1 => (p.clone(), 5..20_i32, 5..20_i32).prop_map(|(p, x, y)| Op::Move(p, x, y)),
        2 => (m, p.clone()).prop_map(|(m, p)| Op::Aggro(m, p)),
        1 => p.clone().prop_map(Op::Leave),
        2 => p.prop_map(Op::Respawn),
    ]
}

/// Ticks of ops; most ticks are idle so swings land, monsters die and cooldowns pass.
fn fight() -> impl Strategy<Value = Vec<Vec<Op>>> {
    prop::collection::vec(
        prop_oneof![4 => Just(Vec::new()), 1 => prop::collection::vec(op(), 1..4)],
        10..160,
    )
}

fn resolve(ops: &[Vec<Op>], npcs: &[EntityId]) -> Vec<Vec<ZoneInput>> {
    let mut seq = 0_u32;
    ops.iter()
        .map(|tick| {
            tick.iter()
                .map(|o| {
                    seq += 1;
                    let s = |n: u8, c| ZoneInput::session(id(n), SessionGeneration(1), seq, c);
                    match *o {
                        Op::Target(p, m) => s(
                            p,
                            ZoneCommand::SetTarget {
                                entity: id(p),
                                target: m.map(|i| npcs[i]),
                            },
                        ),
                        Op::Attack(p) => s(p, ZoneCommand::Attack { entity: id(p) }),
                        Op::Stop(p) => s(p, ZoneCommand::StopAttack { entity: id(p) }),
                        Op::Move(p, x, y) => s(
                            p,
                            ZoneCommand::MoveTo {
                                entity: id(p),
                                dest: Vec2Fixed::from_tiles(x, y),
                            },
                        ),
                        Op::Aggro(m, p) => ZoneInput::system(ZoneCommand::AddAggro {
                            npc: npcs[m],
                            target: id(p),
                        }),
                        Op::Leave(p) => ZoneInput::system(ZoneCommand::Despawn { entity: id(p) }),
                        Op::Respawn(p) => s(p, ZoneCommand::Respawn { entity: id(p) }),
                    }
                })
                .collect()
        })
        .collect()
}

/// Durable record bytes of every tick, the replay log's unit.
fn record_bytes(records: &[Arc<AppliedTick>]) -> Vec<Vec<u8>> {
    use nightfall_api::application::replay_log::AppliedTickRecord;
    records
        .iter()
        .map(|t| AppliedTickRecord::from_applied(ZoneId(11), t).encode())
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn identical_fights_give_identical_bytes_across_fresh_actors(ops in fight()) {
        let (za, npcs) = arena(5);
        let (zb, npcs_b) = arena(5);
        prop_assert_eq!(&npcs, &npcs_b);
        let script = resolve(&ops, &npcs);
        let rt = runtime();
        let (a, b) = rt.block_on(async { tokio::join!(run_actor(za, &script), run_actor(zb, &script)) });
        prop_assert_eq!(bytes(&a), bytes(&b));
        prop_assert_eq!(record_bytes(&a), record_bytes(&b));
        for t in &a {
            assert_output_order(t);
        }
    }

    #[test]
    fn a_mid_fight_snapshot_restore_continues_byte_identically(ops in fight(), split in 0..160_usize) {
        let (mut z, npcs) = arena(6);
        let script = resolve(&ops, &npcs);
        let split = split.min(script.len());
        for inputs in &script[..split] {
            let draft = z.draft(inputs.clone());
            z.run_tick(draft).unwrap();
        }
        let json = nightfall_api::application::replay_log::encode_snapshot(&z.snapshot()).unwrap();
        let restored = ZoneState::from_snapshot(
            nightfall_api::application::replay_log::decode_snapshot(&json).unwrap(),
        )
        .unwrap();
        prop_assert_eq!(&restored, &z);
        let rt = runtime();
        let (a, b) = rt.block_on(async {
            tokio::join!(run_actor(z, &script[split..]), run_actor(restored, &script[split..]))
        });
        prop_assert_eq!(record_bytes(&a), record_bytes(&b));
        prop_assert_eq!(bytes(&a), bytes(&b));
    }
}
