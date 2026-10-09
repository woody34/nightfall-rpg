//! Zone tick cost with 1,000 moving entities (Story 3.1 acceptance: under 2 ms per tick).
//!
//! `cargo bench -p nightfall-api --bench zone_step`. Criterion reports three cases on the
//! 256x256-tile fixture zone, every entity walking 60 tiles:
//!
//! * `step/1000_movers`: the movement phase alone.
//! * `run_tick/1000_npcs`: the full tick (draft, apply, step, AOI) with no observers.
//! * `run_tick/1000_players`: the full tick where every mover is also an observer, so every
//!   entity's AOI is diffed. Worst case: ~16 entities per 32-tile cell.
//!
//! After Criterion, a plain timing loop asserts the budgets and exits non-zero if one is
//! missed: 2 ms for a full tick with 1,000 moving entities (the Story 3.1 acceptance), and a
//! 10 ms regression ceiling for the all-players worst case, where each of 1,000 players is
//! sent ~144 moves per tick (144k outputs; the network saturates long before the tick does).

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion};
use nightfall_api::domain::zone::{
    EntityId, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneBounds, ZoneCommand, ZoneEvent,
    ZoneId, ZoneInput, ZoneSeed, ZoneState,
};
use uuid::Uuid;

const ENTITIES: u32 = 1_000;
const BUDGET: Duration = Duration::from_millis(2);
const ALL_PLAYERS_CEILING: Duration = Duration::from_millis(10);

/// A zone with `ENTITIES` entities on an 8-tile grid, each already walking 60 tiles.
fn moving_zone(players: bool) -> ZoneState {
    let bounds =
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap();
    let mut z = ZoneState::new(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 1,
        },
        bounds,
        0,
    );
    let mut spawns = Vec::new();
    let mut moves = Vec::new();
    for i in 0..ENTITIES {
        let x = i32::try_from(i % 32).unwrap() * 8;
        let y = i32::try_from(i / 32).unwrap() * 8;
        let entity = EntityId::from_uuid(Uuid::from_u128(u128::from(i) + 1));
        let pos = Vec2Fixed::from_tiles(x, y);
        let dest = Vec2Fixed::from_tiles(if x < 128 { x + 60 } else { x - 60 }, y);
        let spawn = if players {
            ZoneCommand::SpawnPlayer {
                entity,
                name: format!("p{i}"),
                pos,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: None,
            }
        } else {
            // System-placed NPCs get RNG ids, so steer them by the id the spawn reports.
            ZoneCommand::SpawnNpc {
                name: format!("n{i}"),
                pos,
                speed: Speed::DEFAULT,
                combat: None,
            }
        };
        spawns.push(ZoneInput::system(spawn));
        moves.push((entity, dest));
    }
    let draft = z.draft(spawns);
    let spawned = z.run_tick(draft).unwrap();
    let ids: Vec<EntityId> = spawned.events.iter().map(ZoneEvent::entity).collect();
    assert_eq!(ids.len(), ENTITIES as usize);
    let inputs = ids
        .iter()
        .zip(&moves)
        .map(|(id, (_, dest))| {
            ZoneInput::system(ZoneCommand::MoveTo {
                entity: *id,
                dest: *dest,
            })
        })
        .collect();
    let draft = z.draft(inputs);
    let t = z.run_tick(draft).unwrap();
    assert!(t.dispositions.is_empty());
    assert_eq!(t.events.len(), ENTITIES as usize);
    z
}

fn bench(c: &mut Criterion) {
    let npcs = moving_zone(false);
    let players = moving_zone(true);
    c.bench_function("step/1000_movers", |b| {
        b.iter_batched_ref(|| npcs.clone(), |z| black_box(z.step(Tick(2))), BatchSize::SmallInput);
    });
    for (name, zone) in [
        ("run_tick/1000_npcs", &npcs),
        ("run_tick/1000_players", &players),
    ] {
        c.bench_function(name, |b| {
            b.iter_batched_ref(
                || zone.clone(),
                |z| {
                    let draft = z.draft(Vec::new());
                    black_box(z.run_tick(draft).unwrap())
                },
                BatchSize::SmallInput,
            );
        });
    }
}

/// Mean full-tick time over the first 100 ticks of the walk.
fn mean_tick(zone: &ZoneState) -> Duration {
    let mut z = zone.clone();
    let mut total = Duration::ZERO;
    for _ in 0..100 {
        let draft = z.draft(Vec::new());
        let started = Instant::now();
        let t = z.run_tick(draft).unwrap();
        total += started.elapsed();
        black_box(t);
    }
    total / 100
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench(&mut c);
    c.final_summary();

    let cases = [
        ("1000 moving npcs", moving_zone(false), BUDGET),
        ("1000 moving players, all observers", moving_zone(true), ALL_PLAYERS_CEILING),
    ];
    for (name, zone, budget) in cases {
        let mean = mean_tick(&zone);
        println!("budget check: {name}: mean full tick {mean:?} (budget {budget:?})");
        assert!(mean < budget, "{name}: mean tick {mean:?} exceeds {budget:?}");
    }
}
