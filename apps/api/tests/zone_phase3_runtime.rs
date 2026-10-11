//! Real tick -> immutable record -> snapshot replay -> protobuf skill projection.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm,
    clippy::panic
)]
use nightfall_api::application::replay_log::{decode_snapshot, encode_snapshot, AppliedTickRecord};
use nightfall_api::domain::character_progression::{
    CharacterAppearance, CharacterIdentity, ClassState,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::subclass::LearnedSkill;
use nightfall_api::domain::zone::skill::SkillId;
use nightfall_api::domain::zone::*;
use nightfall_api::domain::{AccountId, Race};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::{load_rules, RulesSource},
    skill_data::{load_phase3_rules, Phase3DataSource},
    zone_data::{parse_zone, TEST_ZONE_TOML},
};
use nightfall_api::interface::{grpc::pb, zone_mapping::observer_output_to_pb};
use uuid::Uuid;

fn setup() -> (ZoneState, EntityId, EntityId) {
    setup_at(Vec2Fixed::from_tiles(10, 10), Vec2Fixed::from_tiles(12, 10))
}
fn setup_at(player_pos: Vec2Fixed, npc_pos: Vec2Fixed) -> (ZoneState, EntityId, EntityId) {
    let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
    let npc =
        NpcCombat::from_template(&stats, &parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0])
            .unwrap();
    let mut zone = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 45,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stats,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap();
    let player = EntityId::from_uuid(Uuid::from_u128(1));
    let mut ledger = ClassState::new(ClassId(10));
    ledger.learned_skills = vec![LearnedSkill {
        key: "l2.skill.1177".into(),
        level: 1,
    }];
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = Some(Box::new(PlayerProgression {
        identity: CharacterIdentity {
            account_id: AccountId::from_uuid(Uuid::from_u128(2)),
            race: Race::Human,
            base_class_id: ClassId(10),
            appearance: CharacterAppearance::default(),
        },
        class_state: ledger,
        max_cp: 0,
    }));
    let draft = zone.draft(vec![
        ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: player,
            name: "Caster".into(),
            pos: player_pos,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        }),
        ZoneInput::system(ZoneCommand::SpawnNpc {
            name: "Keltir".into(),
            pos: npc_pos,
            speed: parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0].move_speed,
            combat: Some(Box::new(npc)),
        }),
    ]);
    let tick = zone.run_tick(draft).unwrap();
    assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
    let npc = zone
        .snapshot()
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap()
        .id;
    (zone, player, npc)
}
fn cast(player: EntityId, target: EntityId) -> ZoneInput {
    ZoneInput {
        source: CommandSource::Session {
            entity: player,
            generation: SessionGeneration(1),
        },
        seq: Some(1),
        command: ZoneCommand::CastSkill {
            entity: player,
            skill_id: SkillId(1177),
            target: Some(target),
        },
    }
}
fn combat(zone: &ZoneState, id: EntityId) -> CombatState {
    zone.snapshot()
        .entities
        .iter()
        .find(|e| e.id == id)
        .unwrap()
        .combat
        .clone()
        .unwrap()
}
#[test]
fn wind_strike_live_tick_cost_damage_wire_and_midcast_replay() {
    let (mut zone, player, npc) = setup();
    let mp = combat(&zone, player).mp;
    let hp = combat(&zone, npc).hp;
    let start = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    assert!(start.dispositions.is_empty());
    assert_eq!(combat(&zone, player).mp, mp.checked_sub(2).unwrap());
    let complete = start
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastStarted { complete_at, .. } => Some(*complete_at),
            _ => None,
        })
        .unwrap();
    let bytes = encode_snapshot(&zone.snapshot()).unwrap();
    let mut replay = ZoneState::from_snapshot(decode_snapshot(&bytes).unwrap()).unwrap();
    let mut result = None;
    let mut end_mp = 0;
    for _ in 0..=complete.0 {
        let draft = zone.draft(vec![]);
        if draft.tick == complete {
            end_mp = combat(&zone, player).mp;
        }
        let tick = zone.run_tick(draft.clone()).unwrap();
        let reproduced = replay.run_tick(draft).unwrap();
        let record = AppliedTickRecord::from_applied(ZoneId(1), &tick);
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
        assert_eq!(
            record.encode(),
            AppliedTickRecord::from_applied(ZoneId(1), &reproduced).encode()
        );
        if tick.tick == complete {
            result = Some(tick);
            break;
        }
    }
    let result = result.unwrap();
    assert_eq!(combat(&zone, player).mp, end_mp.checked_sub(7).unwrap());
    let row = result
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastResult { targets, .. } => targets.first(),
            _ => None,
        })
        .unwrap();
    assert_eq!(row.target.entity, npc);
    assert!(row.rolled > 0);
    assert_eq!(row.cp_absorbed, 0);
    assert_eq!(combat(&zone, npc).hp, row.target_hp_after);
    assert_eq!(row.hp_removed, hp.checked_sub(row.target_hp_after).unwrap());
    assert!(result
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::VitalsChanged { entity, .. } if *entity == npc)));
    assert_eq!(
        result
            .events
            .iter()
            .filter(|e| matches!(e,
        ZoneEvent::HateChanged { npc: subject, target, hate: 92, damage: 0, .. }
            if *subject == npc && *target == player))
            .count(),
        1
    );
    let wire: Vec<_> = result.outputs[&player]
        .iter()
        .flat_map(|o| observer_output_to_pb(o, result.server_time_ms))
        .collect();
    assert!(wire.iter().any(|m| matches!(&m.payload, Some(pb::server_message::Payload::Event(pb::WorldEvent { event: Some(pb::world_event::Event::SkillCastResult(r)) })) if r.targets[0].hp_removed == row.hp_removed)));
    ZoneState::from_snapshot(zone.snapshot()).unwrap();
}
#[test]
fn cast_then_cancel_has_no_cost_or_rng_and_move_cancels_live_cast() {
    let (mut zone, player, npc) = setup();
    let before = zone.snapshot();
    let mut cancel = cast(player, npc);
    cancel.seq = Some(2);
    cancel.command = ZoneCommand::CancelCast {
        entity: player,
        cast_id: None,
    };
    let tick = zone
        .run_tick(zone.draft(vec![cast(player, npc), cancel.clone()]))
        .unwrap();
    assert!(tick.dispositions.is_empty());
    assert!(!tick
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::SkillCastStarted { .. })));
    assert_eq!(before.rng, zone.snapshot().rng);
    let started = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    assert!(started.dispositions.is_empty());
    let paid = combat(&zone, player).mp;
    cancel.command = ZoneCommand::MoveTo {
        entity: player,
        dest: Vec2Fixed::from_tiles(11, 10),
    };
    let moved = zone.run_tick(zone.draft(vec![cancel])).unwrap();
    assert!(moved.events.iter().any(|e| matches!(
        e,
        ZoneEvent::SkillCastCancelled {
            reason: state_skill::CastCancelReason::Moved,
            ..
        }
    )));
    assert_eq!(combat(&zone, player).mp, paid);
    ZoneState::from_snapshot(zone.snapshot()).unwrap();
}

#[test]
fn replacement_hydrates_final_owner_state_and_cast_after_identity_batch() {
    let (mut zone, player, npc) = setup();
    zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    let before = zone.snapshot();
    let result = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: player,
            generation: SessionGeneration(2),
        })]))
        .unwrap();
    let out = &result.outputs[&player];
    let last_spawn = out
        .iter()
        .rposition(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { .. })))
        .unwrap();
    let hydration = out
        .iter()
        .position(|o| {
            matches!(
                o,
                ObserverOutput::Event(ZoneEvent::SkillCastStarted {
                    baseline: Some(_),
                    ..
                })
            )
        })
        .unwrap();
    assert!(hydration > last_spawn);
    assert!(out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::SkillStateSync { baseline: Some(t), learned, .. }) if *t == result.tick && learned.iter().any(|s| s.id == SkillId(1177)))));
    assert!(out.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { class: Some(c), .. }) if c.phase3.as_ref().is_some_and(|p| p.baseline == Some(result.tick)))));
    let old = &before.phase3.as_ref().unwrap().entities[&player];
    let after = zone.snapshot();
    let new = &after.phase3.as_ref().unwrap().entities[&player];
    assert_eq!(new.cast, old.cast);
    assert_eq!(new.reuse, old.reuse);
    let record = AppliedTickRecord::from_applied(ZoneId(1), &result);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    let mut legacy = record;
    legacy.digest_version = StateDigestVersion::BinaryV4;
    assert!(AppliedTickRecord::decode(&legacy.encode()).is_err());
}

#[test]
fn magic_caller_rng_sequence_and_living_killer_sp_are_exact() {
    let (zone, player, npc) = setup();
    let mut snapshot = zone.snapshot();
    snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == npc)
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 1;
    let mut zone = ZoneState::from_snapshot(snapshot).unwrap();
    let before = zone.snapshot().rng;
    let start = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    assert_eq!(zone.snapshot().rng, before, "cast identities/timing/reuse consume no RNG");
    let complete = start
        .events
        .iter()
        .find_map(|e| {
            if let ZoneEvent::SkillCastStarted { complete_at, .. } = e {
                Some(*complete_at)
            } else {
                None
            }
        })
        .unwrap();
    while zone.draft(vec![]).tick < complete {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let mut ledger = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    // Pinned MagicalAttackRange.java:77-89 / Formulas.java:845-871. Equal level gives
    // success99; no shield, idle target, no stun. Vengeance still draws at chance zero.
    ledger.roll_below(1000);
    if ledger.roll_below(100) >= 99 {
        ledger.roll_below(100);
    }
    ledger.roll_below(21);
    ledger.roll_below(100);
    let result = zone.run_tick(zone.draft(vec![])).unwrap();
    assert_eq!(zone.snapshot().rng, ledger.snapshot().rng);
    assert!(result.events.iter().any(|e| matches!(e, ZoneEvent::SpGained { amount: 2, total: 2, entity, .. } if *entity == player)));
    assert_eq!(combat(&zone, npc).hp, 0);
    assert_eq!(zone.snapshot().phase3.unwrap().entities[&npc].regen.next_at, None);
    ZoneState::from_snapshot(zone.snapshot()).unwrap();
}

#[test]
fn lost_target_cancels_without_impact_cost_or_handler_rng() {
    let (mut zone, player, npc) = setup();
    let start = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    let complete = start
        .events
        .iter()
        .find_map(|e| {
            if let ZoneEvent::SkillCastStarted { complete_at, .. } = e {
                Some(*complete_at)
            } else {
                None
            }
        })
        .unwrap();
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: npc })]))
        .unwrap();
    let before = zone.snapshot();
    let mut mp = combat(&zone, player).mp;
    let mut canceled = false;
    while zone.draft(vec![]).tick <= complete {
        let result = zone.run_tick(zone.draft(vec![])).unwrap();
        canceled |= result.events.iter().any(|e| {
            matches!(
                e,
                ZoneEvent::SkillCastCancelled {
                    reason: state_skill::CastCancelReason::TargetLost,
                    ..
                }
            )
        });
        let after_mp = combat(&zone, player).mp;
        assert!(after_mp >= mp, "regen may add MP, but cancellation cannot charge MP2");
        mp = after_mp;
        assert!(
            result.dispositions.is_empty(),
            "accepted starts never get a late IntentRejected"
        );
    }
    assert!(canceled);
    assert_eq!(zone.snapshot().rng, before.rng);
    assert_eq!(combat(&zone, player).mp, mp);
}

#[test]
fn visible_cast_keeps_hidden_target_private_and_owner_stats_stay_private() {
    let (mut zone, player, npc) =
        setup_at(Vec2Fixed::from_tiles(60, 10), Vec2Fixed::from_tiles(70, 10));
    let observer = EntityId::from_uuid(Uuid::from_u128(3));
    let CombatRole::Player { progression, .. } = combat(&zone, player).role else {
        panic!("player")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    let admission = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: observer,
            name: "Observer".into(),
            pos: Vec2Fixed::from_tiles(0, 10),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })]))
        .unwrap();
    assert!(admission.dispositions.is_empty());
    assert!(admission.outputs[&observer].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if *entity == player)));
    assert!(!admission.outputs[&observer].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { entity, .. }) if *entity == npc)));
    let start = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
    let output = &start.outputs[&observer];
    assert!(output.iter().any(|o| matches!(
        o,
        ObserverOutput::Event(ZoneEvent::SkillCastStarted { target: None, .. })
    )));
    assert!(!output.iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::StatsChanged { entity, .. } | ZoneEvent::SkillReuse { entity, .. } | ZoneEvent::SkillStateSync { entity, .. }) if *entity == player)));
    let complete = start
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastStarted { complete_at, .. } => Some(*complete_at),
            _ => None,
        })
        .unwrap();
    while zone.draft(vec![]).tick < complete {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let result = zone.run_tick(zone.draft(vec![])).unwrap();
    assert!(result.outputs[&observer].iter().any(|o| matches!(o, ObserverOutput::Event(ZoneEvent::SkillCastResult { targets, .. }) if targets.is_empty())));
    let record = AppliedTickRecord::from_applied(ZoneId(1), &result);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
}

#[test]
fn live_v5_restore_preserves_name_xp_and_collision_guards() {
    let (zone, player, _) = setup();
    for field in 0..3 {
        let mut snapshot = zone.snapshot();
        let entity = snapshot
            .entities
            .iter_mut()
            .find(|e| e.id == player)
            .unwrap();
        match field {
            0 => entity.name = String::new(),
            1 => entity.combat.as_mut().unwrap().collision_radius = Fixed::from_raw(0),
            _ => {
                if let CombatRole::Player { xp, .. } = &mut entity.combat.as_mut().unwrap().role {
                    *xp = u64::MAX;
                }
            },
        }
        assert!(ZoneState::from_snapshot(snapshot).is_err());
    }
}

#[tokio::test]
async fn real_v5_actor_byte_admission_preserves_fifo_and_terminal_inputs_never_use_rng() {
    use nightfall_api::application::zone_actor::{
        manual_ticks, OpenGate, ZoneActor, ZoneSendError,
    };
    use nightfall_api::application::zone_admission::RecordAdmissionPolicy;
    let (state, player, _) = setup();
    let before_rng = state.snapshot().rng;
    let (ticks, driver) = manual_ticks();
    let actor = ZoneActor::try_spawn_gated_with_admission(
        state,
        ticks,
        OpenGate,
        RecordAdmissionPolicy {
            version: 1,
            record_budget: 20_000,
            max_command_bytes: 16_384,
            max_observers: 16,
        },
    )
    .unwrap();
    let oversized = ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: EntityId::from_uuid(Uuid::from_u128(1234)),
        name: "x".repeat(16_384),
        pos: Vec2Fixed::from_tiles(10, 10),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: None,
    });
    assert!(matches!(actor.send(oversized), Err(ZoneSendError::AdmissionTooLarge(_))));
    let mut output = actor.subscribe();
    for seq in 1..=10 {
        actor
            .send(ZoneInput::session(
                player,
                SessionGeneration(1),
                seq,
                ZoneCommand::StopMove { entity: player },
            ))
            .unwrap();
    }
    driver.step().await.unwrap();
    let first = output.recv().await.unwrap();
    assert_eq!(first.commands.len(), 8);
    assert_eq!(actor.stats().borrow().commands_deferred, 2);
    assert_eq!(first.digest_version, StateDigestVersion::BinaryV5);
    driver.step().await.unwrap();
    let second = output.recv().await.unwrap();
    assert_eq!(
        second.commands.iter().map(|c| c.seq).collect::<Vec<_>>(),
        vec![Some(9), Some(10)]
    );
    let after = actor.snapshot().await.unwrap();
    assert_eq!(after.rng, before_rng);
    assert_eq!(after.entities.len(), 2);
    for tick in [first, second] {
        let record = AppliedTickRecord::from_applied(ZoneId(1), &tick);
        assert!(record.with_output_digests().encoded_len() <= 20_000);
    }
}
