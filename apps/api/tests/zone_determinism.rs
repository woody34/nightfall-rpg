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
    clippy::arithmetic_side_effects
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
            })
        }),
        (point(), 0..1500_u32).prop_map(|(pos, s)| ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "npc".to_owned(),
            pos,
            speed: Speed::from_milli_tiles_per_tick(s),
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
                e @ (ZoneEvent::AttackResult { .. }
                | ZoneEvent::EntityDied { .. }
                | ZoneEvent::EntityRespawned { .. }
                | ZoneEvent::StatsChanged { .. }
                | ZoneEvent::XpGained { .. }
                | ZoneEvent::LevelUp { .. }
                | ZoneEvent::TargetChanged { .. }),
            ) => (4, 0, Some(e.entity())),
        };
        let ranks: Vec<_> = out.iter().map(rank).collect();
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
