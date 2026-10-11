//! Isolated Phase 3 effect/regen kernel costs, not a claim of live V5 tick performance.
//! `cargo bench -p nightfall-api --bench phase3_effects -- --test` prints repeatable workload
//! means. `--snapshot-capacity` instead measures a synthetic maximum-pool snapshot and
//! exits unsuccessfully if the unchanged production codec refuses it. The existing
//! `zone_step` acceptance budgets remain unchanged and run separately.

#![allow(
    missing_docs,
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::arithmetic_side_effects
)]

#[path = "support/snapshot_capacity.rs"]
mod snapshot_capacity;

use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use criterion::Criterion;
use nightfall_api::domain::zone::effects::{
    ActiveEffect, ActiveHandler, ActivePeriodic, EffectCapacities, EffectFlags, EffectId,
    EffectList, EffectPool, EffectSource, EffectSourceKind, FrozenRatio, PeriodicPower,
    PeriodicResource, PulseSchedule,
};
use nightfall_api::domain::zone::skill::{SkillId, SkillRef};
use nightfall_api::domain::zone::skill_math::ExactRatio;
use nightfall_api::domain::zone::state_effects::Phase3Schedule;
use nightfall_api::domain::zone::state_regen::RegenSchedule;
use nightfall_api::domain::zone::state_regen::{restore_regen, RegenAmounts, RegenResource};
use nightfall_api::domain::zone::{
    CastState, EntityId, EntityOrdinal, Phase3Entity, ResourceFractions, Tick,
};

const ENTITIES: usize = 1_000;

/// Synthetic distinct stack identities fill all maximum finite pools. This measures the
/// data structure; source admission/handler eligibility belongs to the live engine benchmark.
fn full_effects() -> EffectList {
    let mut result = EffectList::default();
    let mut id = 1;
    for (pool, count) in [
        (EffectPool::Buff, 24),
        (EffectPool::SongDance, 12),
        (EffectPool::Debuff, 8),
    ] {
        for _ in 0..count {
            result
                .admit(
                    ActiveEffect {
                        id: EffectId(id),
                        skill: SkillRef {
                            id: SkillId(u32::try_from(id).unwrap()),
                            rank: 1,
                        },
                        source: EffectSource {
                            binding: None,
                            kind: EffectSourceKind::System,
                        },
                        target_incarnation: 1,
                        applied_at: Tick(0),
                        expires_at: Some(Tick(1200)),
                        pool,
                        abnormal_type: format!("BENCH_{id}"),
                        abnormal_level: 1,
                        flags: EffectFlags {
                            stay_after_death: false,
                            stay_on_subclass_change: true,
                            removed_on_damage: false,
                            removed_on_action_except_move: false,
                            irreplaceable: false,
                        },
                        handlers: vec![ActiveHandler {
                            xml_ordinal: 0,
                            periodic: Some(ActivePeriodic {
                                resource: PeriodicResource::Hp,
                                power: PeriodicPower::Diff {
                                    amount: FrozenRatio::new(ExactRatio::whole(1)),
                                },
                                schedule: PulseSchedule::new(Tick(0), 666).unwrap(),
                            }),
                        }],
                    },
                    EffectCapacities::for_enlarge(4),
                )
                .unwrap();
            id += 1;
        }
    }
    assert_eq!(result.entries().len(), 44);
    result
}

fn pulse_scan(lists: &[EffectList]) -> usize {
    lists
        .iter()
        .map(|list| list.pulses(Tick(7)).unwrap().len())
        .sum()
}
fn expiry_scan(lists: &[EffectList]) -> usize {
    lists
        .iter()
        .map(|list| list.expired(Tick(1200)).len())
        .sum()
}
fn regeneration() -> u64 {
    let resources = RegenResource {
        current: 1,
        maximum: 10000,
    };
    let amounts = RegenAmounts {
        hp_q: 1_584_000,
        mp_q: 1_100_000,
        cp_q: Some(1_584_000),
    };
    let mut granted = 0;
    for _ in 0..ENTITIES {
        let result = restore_regen(
            black_box(resources),
            resources,
            Some(resources),
            black_box(ResourceFractions {
                hp: 584_000,
                mp: 100_000,
                cp: 584_000,
            }),
            amounts,
        )
        .unwrap();
        granted += u64::from(result.hp_granted + result.mp_granted + result.cp_granted);
        black_box(result);
    }
    granted
}
fn report(name: &str, mut operation: impl FnMut()) {
    const ITERATIONS: u32 = 200;
    for _ in 0..10 {
        operation();
    }
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        operation();
    }
    println!("{name}: mean {:?} ({ITERATIONS} iterations)", started.elapsed() / ITERATIONS);
}
fn main() -> std::process::ExitCode {
    if std::env::args().any(|argument| argument == "--snapshot-capacity") {
        return if snapshot_capacity::run() {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        };
    }
    let lists = vec![full_effects(); ENTITIES];
    let entities = lists
        .iter()
        .enumerate()
        .map(|(index, effects)| {
            let ordinal = u64::try_from(index).unwrap() + 1;
            (
                EntityId::from_uuid(uuid::Uuid::from_u128(u128::from(ordinal))),
                Phase3Entity {
                    ordinal: EntityOrdinal(ordinal),
                    cast: CastState::Idle,
                    reuse: BTreeMap::new(),
                    effects: effects.clone(),
                    modifier_install_order: BTreeMap::new(),
                    regen: RegenSchedule {
                        next_at: Some(Tick(30)),
                    },
                    fractions: ResourceFractions::default(),
                    player: None,
                },
            )
        })
        .collect();
    // Synthetic scheduling load only: excludes canonical source validation and live tick
    // handlers, exactly like the scan kernels above. Construction is outside selection time.
    let schedule = Phase3Schedule::rebuild(&entities).unwrap();
    let mut criterion = Criterion::default().configure_from_args();
    criterion.bench_function("phase3/regen_1000", |b| b.iter(|| black_box(regeneration())));
    criterion.bench_function("phase3/pulses_1000_full_pools", |b| {
        b.iter(|| black_box(pulse_scan(&lists)));
    });
    criterion.bench_function("phase3/expiry_1000_full_pools", |b| {
        b.iter(|| black_box(expiry_scan(&lists)));
    });
    criterion.bench_function("phase3/index_idle_1000_full_pools", |b| {
        b.iter(|| black_box(schedule.effect_pulses(Tick(6))));
    });
    criterion.bench_function("phase3/index_due_1000_full_pools", |b| {
        b.iter(|| black_box(schedule.effect_pulses(Tick(7))));
    });
    criterion.final_summary();
    assert_eq!(pulse_scan(&lists), 44_000);
    assert_eq!(expiry_scan(&lists), 44_000);
    assert!(schedule.effect_pulses(Tick(6)).is_empty());
    assert_eq!(schedule.effect_pulses(Tick(7)).len(), 44_000);
    report("phase3 kernel regen/1000", || {
        black_box(regeneration());
    });
    report("phase3 kernel pulses/1000x44", || {
        black_box(pulse_scan(&lists));
    });
    report("phase3 kernel expiry/1000x44", || {
        black_box(expiry_scan(&lists));
    });
    report("phase3 indexed idle/1000x44", || {
        black_box(schedule.effect_pulses(Tick(6)));
    });
    report("phase3 indexed due selection/1000x44", || {
        black_box(schedule.effect_pulses(Tick(7)));
    });
    report("phase3 indexed synchronized regen selection/1000", || {
        black_box(schedule.regen_candidates(Tick(30)));
    });
    std::process::ExitCode::SUCCESS
}
