//! Actual V5 tick budget preflight on the legacy benchmark's 8-tile grid. All combatants
//! enter through `run_tick` with the shipped rule bundles. The 8/44-effect cases add
//! validated synthetic periodic containers before cold restore; zero is the admission
//! budget gate, while full-pool stress costs are reported separately. No client/broker soak.
#![allow(clippy::indexing_slicing, clippy::too_many_lines)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use nightfall_api::domain::character_progression::{
    CharacterAppearance, CharacterIdentity, ClassState,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::zone::{
    EntityId, NpcCombat, PlayerLoad, PlayerProgression, SessionGeneration, Speed, Vec2Fixed,
    ZoneBounds, ZoneCommand, ZoneId, ZoneInput, ZoneSeed, ZoneState,
};
use nightfall_api::domain::{AccountId, Race};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::{load_rules, RulesSource},
    skill_data::{load_phase3_rules, Phase3DataSource},
    zone_data::{parse_zone, TEST_ZONE_TOML},
};
use uuid::Uuid;

fn moving_zone(players: bool, effects: u32) -> ZoneState {
    let rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let template = parse_zone(TEST_ZONE_TOML).unwrap().npc_templates.remove(0);
    let spec = NpcCombat::from_template(&rules, &template).unwrap();
    let mut zone = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 9,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        rules,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap();
    let mut inputs = Vec::new();
    for index in 0..1000_u32 {
        let entity = EntityId::from_uuid(Uuid::from_u128(u128::from(index) + 1));
        let pos = Vec2Fixed::from_tiles(
            i32::try_from(index % 32).unwrap() * 8,
            i32::try_from(index / 32).unwrap() * 8,
        );
        let command = if players {
            let mut load = PlayerLoad::fresh("human_mystic");
            load.progression = Some(Box::new(PlayerProgression {
                identity: CharacterIdentity {
                    account_id: AccountId::from_uuid(entity.as_uuid()),
                    race: Race::Human,
                    base_class_id: ClassId(10),
                    appearance: CharacterAppearance::default(),
                },
                class_state: ClassState::new(ClassId(10)),
                max_cp: 0,
            }));
            ZoneCommand::SpawnPlayer {
                entity,
                name: "Caster".into(),
                pos,
                speed: Speed::DEFAULT,
                generation: SessionGeneration(1),
                load: Some(Box::new(load)),
            }
        } else {
            ZoneCommand::SpawnNpc {
                name: "Keltir".into(),
                pos,
                speed: template.move_speed,
                combat: Some(Box::new(spec.clone())),
            }
        };
        inputs.push(ZoneInput::system(command));
    }
    let admitted = zone.run_tick(zone.draft(inputs)).unwrap();
    assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
    assert_eq!(zone.entity_count(), 1000);
    let mut snapshot = zone.snapshot();
    if effects != 0 {
        super::live_effect_fixture::fill(&mut snapshot, effects);
    }
    assert_eq!(snapshot.phase3.as_ref().unwrap().entities.len(), 1000);
    snapshot.validate_version().unwrap();
    // Exercise the actual cold validator before timing the restored live state.
    zone = ZoneState::from_snapshot(snapshot).unwrap();
    let inputs = zone
        .snapshot()
        .entities
        .iter()
        .map(|entity| {
            let x = entity.pos.x.raw() / 1000;
            let y = entity.pos.y.raw() / 1000;
            ZoneInput::system(ZoneCommand::MoveTo {
                entity: entity.id,
                dest: Vec2Fixed::from_tiles(if x < 128 { x + 60 } else { x - 60 }, y),
            })
        })
        .collect();
    let moved = zone.run_tick(zone.draft(inputs)).unwrap();
    assert!(moved.dispositions.is_empty(), "{:?}", moved.dispositions);
    assert!(zone.snapshot().entities.iter().all(|e| e.dest.is_some()));
    zone
}

pub(super) fn run() -> bool {
    let mut passed = true;
    for effects in [0, 8, 44] {
        for (players, budget) in [
            (false, Duration::from_millis(2)),
            (true, Duration::from_millis(10)),
        ] {
            let kind = if players {
                "players/all_observers"
            } else {
                "npcs"
            };
            println!("live_v5 setup: 1000 {kind}, full shipped rules, {effects} synthetic periodic effects/entity");
            let mut zone = moving_zone(players, effects);
            let mut samples = Vec::new();
            let mut total = Duration::ZERO;
            for _ in 0..100 {
                let draft = zone.draft(Vec::new());
                let start = Instant::now();
                let applied = zone.run_tick(draft).unwrap();
                let elapsed = start.elapsed();
                total += elapsed;
                samples.push(elapsed);
                black_box(applied);
            }
            samples.sort_unstable();
            let mean = total / 100;
            let case_passed = mean < budget;
            passed &= effects != 0 || case_passed;
            let gate = if effects == 0 {
                "BASELINE_ADMISSION"
            } else {
                "SYNTHETIC_STRESS_REPORT"
            };
            println!("live_v5 full_tick: 1000 {kind} effects={effects} mean={mean:?} p50={:?} p99={:?} baseline_ceiling={budget:?} within_baseline_ceiling={case_passed} scope={gate} packaged_soak=NOT_RUN durability=NOT_MEASURED", samples[49], samples[98]);
        }
    }
    passed
}
