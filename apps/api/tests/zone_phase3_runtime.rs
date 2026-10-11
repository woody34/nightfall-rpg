//! Real tick -> immutable record -> snapshot replay -> protobuf skill projection.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm,
    clippy::panic
)]
#[path = "common/hydration.rs"]
mod hydration_fixture;

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
use std::fmt::Write as _;
use uuid::Uuid;

fn setup() -> (ZoneState, EntityId, EntityId) {
    setup_at(Vec2Fixed::from_tiles(10, 10), Vec2Fixed::from_tiles(12, 10))
}
fn setup_at(player_pos: Vec2Fixed, npc_pos: Vec2Fixed) -> (ZoneState, EntityId, EntityId) {
    setup_with_skills(player_pos, npc_pos, &[(1177, 1)])
}
fn setup_with_skills(
    player_pos: Vec2Fixed,
    npc_pos: Vec2Fixed,
    skills: &[(u32, u32)],
) -> (ZoneState, EntityId, EntityId) {
    setup_with_rules(
        player_pos,
        npc_pos,
        skills,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
}
fn setup_with_rules(
    player_pos: Vec2Fixed,
    npc_pos: Vec2Fixed,
    skills: &[(u32, u32)],
    rules: Phase3Rules,
) -> (ZoneState, EntityId, EntityId) {
    let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
    let mut npc =
        NpcCombat::from_template(&stats, &parse_zone(TEST_ZONE_TOML).unwrap().npc_templates[0])
            .unwrap();
    let profile = &rules.profiles().npcs["keltir"].physical;
    npc.stats.level = profile.level;
    npc.stats.max_hp = profile.max_hp;
    npc.stats.max_mp = profile.max_mp;
    let mut zone = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 45,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stats,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        rules,
    )
    .unwrap();
    let player = EntityId::from_uuid(Uuid::from_u128(1));
    let mut ledger = ClassState::new(ClassId(10));
    ledger.learned_skills = skills
        .iter()
        .map(|(id, rank)| LearnedSkill {
            key: format!("l2.skill.{id}"),
            level: *rank,
        })
        .collect();
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
    let (mut zone, player, npc) = setup();
    let observer = EntityId::from_uuid(Uuid::from_u128(888));
    assert!(spawn_observer_v5(&mut zone, player, observer, 10)
        .dispositions
        .is_empty());
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
    let owner_wire = wire_events_for(&result, player);
    assert!(owner_wire
        .iter()
        .any(|e| matches!(e, pb::world_event::Event::SpGained(s)
        if s.entity == player.to_string() && s.amount == 2 && s.total == 2)));
    let observer_wire = wire_events_for(&result, observer);
    assert!(!observer_wire.iter().any(|e| matches!(e,
        pb::world_event::Event::SpGained(s) if s.entity == player.to_string())));
    assert!(!observer_wire.iter().any(|e| matches!(e,
        pb::world_event::Event::StatsChanged(s) if s.entity == player.to_string())));
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

fn finish_skill(
    zone: &mut ZoneState,
    player: EntityId,
    target: EntityId,
    skill: u32,
) -> AppliedTick {
    let mut input = cast(player, target);
    input.command = ZoneCommand::CastSkill {
        entity: player,
        target: Some(target),
        skill_id: SkillId(skill),
    };
    let start = zone.run_tick(zone.draft(vec![input])).unwrap();
    assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
    for _ in 0..200 {
        let applied = zone.run_tick(zone.draft(vec![])).unwrap();
        if applied
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::SkillCastResult { .. }))
        {
            return applied;
        }
    }
    panic!("skill did not complete");
}

#[test]
fn shield_live_install_replace_expire_and_hydrate_without_reapplying() {
    let (mut zone, player, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1)],
    );
    let initial = combat(&zone, npc).sheet.p_def();
    let applied = finish_skill(&mut zone, player, npc, 1040);
    let first = applied
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::EffectApplied { effect_id, .. } => Some(*effect_id),
            _ => None,
        })
        .unwrap();
    assert!(combat(&zone, npc).sheet.p_def() > initial);
    assert!(applied.outputs[&player]
        .iter()
        .flat_map(|o| observer_output_to_pb(o, applied.server_time_ms))
        .any(|m| matches!(
            m.payload,
            Some(pb::server_message::Payload::Event(pb::WorldEvent {
                event: Some(pb::world_event::Event::EffectApplied(_))
            }))
        )));
    let bytes = encode_snapshot(&zone.snapshot()).unwrap();
    let mut restored = ZoneState::from_snapshot(decode_snapshot(&bytes).unwrap()).unwrap();
    for _ in 0..22 {
        let draft = zone.draft(vec![]);
        let a = zone.run_tick(draft.clone()).unwrap();
        let b = restored.run_tick(draft).unwrap();
        assert_eq!(
            AppliedTickRecord::from_applied(ZoneId(1), &a).encode(),
            AppliedTickRecord::from_applied(ZoneId(1), &b).encode()
        );
    }
    let refreshed = finish_skill(&mut zone, player, npc, 1040);
    let removals: Vec<_> = refreshed
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::EffectRemoved {
                effect_id, reason, ..
            } => Some((*effect_id, *reason)),
            _ => None,
        })
        .collect();
    assert_eq!(removals, vec![(first, effects::EffectRemoveReason::Replaced)]);
    assert_eq!(combat(&zone, npc).sheet.p_def(), combat(&restored, npc).sheet.p_def());
    let before_replace = zone.state_digest();
    let replacement = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::ReplaceSession {
            entity: player,
            generation: SessionGeneration(2),
        })]))
        .unwrap();
    let out = &replacement.outputs[&player];
    let last_identity = out
        .iter()
        .rposition(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EntitySpawn { .. })))
        .unwrap();
    let hydration = out.iter().position(|o| matches!(o, ObserverOutput::Event(ZoneEvent::EffectApplied { baseline: Some(_), target, .. }) if target.entity == npc)).unwrap();
    assert!(hydration > last_identity);
    assert_ne!(zone.state_digest(), before_replace); // tick/generation changed, effect identity did not.
    assert_eq!(
        zone.snapshot().phase3.unwrap().entities[&npc]
            .effects
            .entries()
            .len(),
        1
    );
    // A bounded valid checkpoint suffix isolates expiry without replaying 1,200 idle seconds.
    let mut snapshot = zone.snapshot();
    let next = zone.draft(vec![]).tick;
    let side = snapshot
        .phase3
        .as_mut()
        .unwrap()
        .entities
        .get_mut(&npc)
        .unwrap();
    let mut entries = side.effects.entries().clone();
    for effect in entries.values_mut() {
        effect.expires_at = Some(next);
    }
    side.effects = effects::EffectList::from_entries(entries).unwrap();
    let mut suffix = ZoneState::from_snapshot(snapshot).unwrap();
    let expired = suffix.run_tick(suffix.draft(vec![])).unwrap();
    assert!(expired.events.iter().any(|e| matches!(
        e,
        ZoneEvent::EffectRemoved {
            reason: effects::EffectRemoveReason::Expired,
            ..
        }
    )));
    assert_eq!(combat(&suffix, npc).sheet.p_def(), initial);
}

#[test]
fn debuff_success_draws_duration_and_poison_pulses_survive_departure_and_restore() {
    let (mut zone, player, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1164, 1), (1168, 1)],
    );
    let initial_attack = combat(&zone, npc).sheet.p_atk();
    let before = zone.snapshot().rng;
    let mut ledger = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    // Debuff reflection at zero, then continuous success; duration/mastery consume none.
    ledger.roll_below(100);
    ledger.roll_below(100);
    let result = finish_skill(&mut zone, player, npc, 1164);
    assert_ne!(zone.snapshot().rng, before);
    assert_eq!(zone.snapshot().rng, ledger.snapshot().rng);
    let applied = result
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::EffectApplied { .. }));
    assert_eq!(combat(&zone, npc).sheet.p_atk() < initial_attack, applied);
    for _ in 0..10 {
        let result = finish_skill(&mut zone, player, npc, 1168);
        if result.events.iter().any(
            |e| matches!(e, ZoneEvent::EffectApplied { skill, .. } if skill.id == SkillId(1168)),
        ) {
            break;
        }
        for _ in 0..40 {
            zone.run_tick(zone.draft(vec![])).unwrap();
        }
    }
    let poisoned = zone.snapshot().phase3.unwrap().entities[&npc]
        .effects
        .entries()
        .values()
        .any(|e| e.skill.id == SkillId(1168));
    assert!(poisoned);
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: player })]))
        .unwrap();
    let mut replay = ZoneState::from_snapshot(
        decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap(),
    )
    .unwrap();
    let mut damaged = false;
    for _ in 0..100 {
        let hp = combat(&zone, npc).hp;
        let draft = zone.draft(vec![]);
        let a = zone.run_tick(draft.clone()).unwrap();
        let b = replay.run_tick(draft).unwrap();
        damaged |= combat(&zone, npc).hp < hp;
        assert!(combat(&zone, npc).hp >= 1);
        assert_eq!(
            AppliedTickRecord::from_applied(ZoneId(1), &a).encode(),
            AppliedTickRecord::from_applied(ZoneId(1), &b).encode()
        );
    }
    assert!(damaged);
}

#[test]
fn self_heal_runs_at_impact_with_one_magic_crit_draw_and_public_final_vitals() {
    let (zone, player, _) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1216, 1)],
    );
    let mut snapshot = zone.snapshot();
    snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == player)
        .unwrap()
        .combat
        .as_mut()
        .unwrap()
        .hp = 1;
    let mut zone = ZoneState::from_snapshot(snapshot).unwrap();
    let mut ledger = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    ledger.roll_below(1000);
    let result = finish_skill(&mut zone, player, player, 1216);
    assert_eq!(zone.snapshot().rng, ledger.snapshot().rng);
    assert!(combat(&zone, player).hp > 40);
    assert!(result.events.iter().any(|e| matches!(e, ZoneEvent::SkillCastResult { targets, .. } if targets.iter().any(|t| t.resolution_kind == SkillResolutionKind::Heal && t.heal > 0))));
    assert!(result
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::VitalsChanged { entity, .. } if *entity == player)));
    ZoneState::from_snapshot(zone.snapshot()).unwrap();
}

#[test]
fn lethal_magic_removes_npc_buffs_and_restores_a_dead_snapshot() {
    let (mut zone, player, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1177, 1)],
    );
    finish_skill(&mut zone, player, npc, 1040);
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
    let applied = finish_skill(&mut zone, player, npc, 1177);
    assert!(applied.events.iter().any(|e| matches!(e,
        ZoneEvent::EffectRemoved { target, reason: effects::EffectRemoveReason::Death, .. }
        if target.entity == npc)));
    assert_eq!(combat(&zone, npc).hp, 0);
    let snapshot = zone.snapshot();
    let side = &snapshot.phase3.as_ref().unwrap().entities[&npc];
    assert!(side.effects.entries().is_empty());
    assert_eq!(side.fractions, ResourceFractions::default());
    assert_eq!(side.regen.next_at, None);
    ZoneState::from_snapshot(snapshot).unwrap();
}

#[tokio::test]
async fn real_v5_actor_byte_admission_preserves_fifo_and_terminal_inputs_never_use_rng() {
    use nightfall_api::application::zone_actor::{
        manual_ticks, OpenGate, ZoneActor, ZoneSendError,
    };
    use nightfall_api::application::zone_admission::RecordAdmissionPolicy;
    let (state, player, _) = setup();
    let policy = RecordAdmissionPolicy {
        version: 1,
        record_budget: 20_000,
        max_command_bytes: 16_384,
        max_observers: 16,
    };
    let snapshot = state.snapshot();
    let (ticks, _) = manual_ticks();
    assert!(
        ZoneActor::try_spawn_gated_with_admission(state, ticks, OpenGate, policy).is_err(),
        "an override cannot disagree with recorded epoch inputs"
    );
    let mut snapshot = snapshot;
    snapshot.meta.phase3_admission_policy = Some(Phase3AdmissionPolicy {
        version: 1,
        record_budget: 20_000,
        max_command_bytes: 16_384,
        max_observers: 16,
    });
    let state = ZoneState::from_snapshot(snapshot).unwrap();
    let before_rng = state.snapshot().rng;
    let (ticks, driver) = manual_ticks();
    let actor = ZoneActor::spawn_gated(state, ticks, OpenGate);
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

#[tokio::test]
async fn checkpoint_lane_requires_an_admitted_player_revision_before_retained_output() {
    use nightfall_api::application::checkpoint::CheckpointService;
    use nightfall_api::application::zone_actor::{manual_ticks, OpenGate, TickOutcome, ZoneActor};
    use nightfall_api::infrastructure::memory::{
        InMemoryCharacterRepository, InMemorySessionAudit,
    };
    use nightfall_api::infrastructure::telemetry::Metrics;
    use std::sync::Arc;
    let (state, player, npc) = setup();
    let expected = state.next_tick();
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, OpenGate);
    zone.checkpoints.install(CheckpointService::new(
        Arc::new(InMemoryCharacterRepository::default()),
        Arc::new(InMemorySessionAudit::default()),
        Arc::new(Metrics::detached()),
    ));
    let mut output = zone.subscribe();
    zone.send(cast(player, npc)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(expected));
    zone.stopped().await;
    assert!(zone.persistence_failed());
    assert!(matches!(
        output.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert!(zone.snapshot().await.is_err());
}

#[test]
fn checkpoint_export_freezes_strict_timer_thresholds_and_relative_modifier_order() {
    let (mut zone, player, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1177, 1)],
    );
    let applied = finish_skill(&mut zone, player, player, 1040);
    let tick = applied.tick;
    let mut snapshot = zone.snapshot();
    let CombatRole::Player {
        progression: Some(progression),
        ..
    } = combat(&zone, player).role
    else {
        panic!("player")
    };
    let phase = snapshot.phase3.as_mut().unwrap();
    let side = phase.entities.get_mut(&player).unwrap();
    side.reuse.insert(SkillId(1040), Tick(tick.0 + 50));
    side.reuse.insert(SkillId(1177), Tick(tick.0 + 51));
    side.player.as_mut().unwrap().pvp.flag_until = Some(Tick(tick.0 + 1));
    side.fractions.mp = Q / 7;
    let mut effects = side.effects.entries().clone();
    let effect = effects.values_mut().next().unwrap();
    effect.expires_at = Some(Tick(tick.0 + 101));
    side.effects = effects::EffectList::from_entries(effects).unwrap();
    let checkpoint = phase.checkpoint(player, tick).unwrap();
    assert_eq!(checkpoint.flag_remaining_ticks, 1);
    assert_eq!(checkpoint.resource_fractions.mp, Q / 7);
    assert_eq!(checkpoint.reuse_remainders.len(), 1);
    assert_eq!(checkpoint.reuse_remainders[&SkillId(1177)], 51);
    assert_eq!(checkpoint.effect_remainders.len(), 1);
    assert_eq!(checkpoint.effect_remainders[0].remaining_ticks, 101);
    checkpoint
        .validate(&phase.rules, &progression.class_state.learned_skills, true)
        .unwrap();
    let restored = checkpoint
        .stage_restore(
            &phase.rules,
            &progression.class_state.learned_skills,
            true,
            7,
            Tick(2000),
            40,
            100,
        )
        .unwrap();
    let restored_effect = &restored.effects.entries()[&effects::EffectId(40)];
    assert_eq!(restored_effect.target_incarnation, 7);
    assert_eq!(restored_effect.expires_at, Some(Tick(2101)));
    assert_eq!(restored.flag_until, Some(Tick(2001)));
    assert_eq!(restored.reuse[&SkillId(1177)], Tick(2051));
    assert!(phase.checkpoint(npc, tick).is_err());
    // One tick later, exact equality with both thresholds excludes both saved timers.
    let expired_subset = phase.checkpoint(player, Tick(tick.0 + 1)).unwrap();
    assert!(expired_subset.effect_remainders.is_empty());
    assert!(expired_subset.reuse_remainders.is_empty());
    assert!(!expired_subset
        .modifier_install_order
        .iter()
        .any(|s| matches!(s, effects::RetainedModifierSource::Effect { .. })));
    assert_eq!(
        checkpoint.effect_remainders[0].remaining_ticks, 101,
        "frozen request never ages"
    );
}

#[test]
fn paused_load_rebases_at_admission_without_hooks_or_rng_and_same_tick_commands_see_it() {
    let (mut zone, player, _) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1177, 1)],
    );
    let completed = finish_skill(&mut zone, player, player, 1040);
    let snapshot = zone.snapshot();
    let phase = snapshot.phase3.as_ref().unwrap();
    let old_effect = *phase.entities[&player]
        .effects
        .entries()
        .keys()
        .next()
        .unwrap();
    let mut checkpoint = phase.checkpoint(player, completed.tick).unwrap();
    checkpoint.reuse_remainders.insert(SkillId(1177), 123);
    checkpoint.resource_fractions.mp = Q / 7;
    let remaining = checkpoint.effect_remainders[0].remaining_ticks;
    let c = combat(&zone, player);
    let CombatRole::Player { progression, .. } = c.role else {
        panic!("player")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    load.phase3 = Some(checkpoint.clone());
    load.hp = Some(1);
    load.mp = Some(1);
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: player })]))
        .unwrap();
    for _ in 0..75 {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let before = zone.snapshot();
    let at = zone.next_tick();
    let draft = zone.draft(vec![
        ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: player,
            name: "Caster".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load.clone())),
        }),
        ZoneInput::session(
            player,
            SessionGeneration(1),
            5,
            ZoneCommand::MoveTo {
                entity: player,
                dest: Vec2Fixed::from_tiles(11, 10),
            },
        ),
    ]);
    let mut replay = ZoneState::from_snapshot(before.clone()).unwrap();
    let admitted = zone.run_tick(draft.clone()).unwrap();
    let repeated = replay.run_tick(draft).unwrap();
    assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
    assert_eq!(admitted, repeated);
    assert_eq!(zone.snapshot().rng, before.rng);
    assert_eq!(combat(&zone, player).hp, 1, "restore does not run heal hooks");
    assert_eq!(combat(&zone, player).mp, 1, "restore charges no costs and starts regen later");
    assert_eq!(combat(&zone, player).sheet.p_def(), c.sheet.p_def());
    let after = zone.snapshot();
    let side = &after.phase3.as_ref().unwrap().entities[&player];
    assert_eq!(side.fractions.mp, Q / 7);
    assert_eq!(side.reuse[&SkillId(1177)], Tick(at.0 + 123));
    assert_eq!(side.regen.next_at, Some(Tick(at.0 + 30)));
    let effect = side.effects.entries().values().next().unwrap();
    assert!(effect.id > old_effect);
    assert_eq!(effect.expires_at, Some(Tick(at.0 + remaining)));
    assert!(!admitted
        .events
        .iter()
        .any(|event| matches!(event, ZoneEvent::EffectApplied { .. })));
    assert!(admitted.outputs[&player].iter().any(|out| matches!(out,
        ObserverOutput::Event(ZoneEvent::EffectApplied { baseline: Some(tick), .. }) if *tick == at)));
    let record = AppliedTickRecord::from_applied(ZoneId(1), &admitted);
    assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    ZoneState::from_snapshot(after).unwrap();

    // The marker is never accepted by a legacy simulation, even when the payload is empty.
    let mut legacy = ZoneState::new(before.seed, before.bounds, 0);
    let legacy_before = legacy.snapshot();
    let command = ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: player,
        name: "Caster".into(),
        pos: Vec2Fixed::from_tiles(10, 10),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(load)),
    });
    assert_eq!(
        legacy.run_tick(legacy.draft(vec![command])),
        Err(TickError::Phase3CommandInLegacy)
    );
    assert_eq!(legacy.snapshot(), legacy_before);
}

#[test]
fn invalid_paused_admission_is_a_disposition_without_partial_allocation() {
    let (mut zone, player, _) = setup();
    let CombatRole::Player { progression, .. } = combat(&zone, player).role else {
        panic!("player")
    };
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: player })]))
        .unwrap();
    let before = zone.snapshot();
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    let mut checkpoint = Phase3Checkpoint::new();
    checkpoint.reuse_remainders.insert(SkillId(1177), 50);
    load.phase3 = Some(checkpoint);
    let applied = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: player,
            name: "Caster".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })]))
        .unwrap();
    assert_eq!(applied.dispositions.len(), 1);
    assert_eq!(applied.dispositions[0].reason, RejectReason::InvalidLoad);
    let after = zone.snapshot();
    assert_eq!(after.rng, before.rng);
    assert_eq!(after.phase3, before.phase3);
    assert!(!after.entities.iter().any(|entity| entity.id == player));
}

#[test]
fn retained_player_dot_is_admitted_and_cold_replays_with_cp_reduction() {
    let (mut zone, caster, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1168, 1)],
    );
    for _ in 0..10 {
        let result = finish_skill(&mut zone, caster, npc, 1168);
        if result
            .events
            .iter()
            .any(|event| matches!(event, ZoneEvent::EffectApplied { .. }))
        {
            break;
        }
        for _ in 0..40 {
            zone.run_tick(zone.draft(vec![])).unwrap();
        }
    }
    let snapshot = zone.snapshot();
    let phase = snapshot.phase3.as_ref().unwrap();
    let effect = phase.entities[&npc]
        .effects
        .entries()
        .values()
        .next()
        .unwrap();
    let retained = effect
        .retain(Tick(zone.next_tick().0 - 1))
        .unwrap()
        .unwrap();
    let mut checkpoint = Phase3Checkpoint::new();
    checkpoint.effect_remainders.push(retained);
    checkpoint
        .modifier_install_order
        .push(effects::RetainedModifierSource::Effect { effect_ordinal: 0 });
    let CombatRole::Player { progression, .. } = combat(&zone, caster).role else {
        panic!("player")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    load.phase3 = Some(checkpoint);
    let joining = EntityId::from_uuid(Uuid::from_u128(999));
    let result = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: joining,
            name: "Joining".into(),
            pos: Vec2Fixed::from_tiles(10, 10),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })]))
        .unwrap();
    assert!(result.dispositions.is_empty());
    assert!(zone.entity(joining).is_some());
    let mut restored = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    let cp_before = match combat(&zone, joining).role {
        CombatRole::Player {
            progression: Some(p),
            ..
        } => p.class_state.cp,
        _ => panic!("player"),
    };
    for _ in 0..35 {
        let applied = zone.run_tick(zone.draft(vec![])).unwrap();
        assert_eq!(applied, restored.run_tick(restored.draft(vec![])).unwrap());
    }
    let CombatRole::Player {
        progression: Some(p),
        ..
    } = combat(&zone, joining).role
    else {
        panic!("player")
    };
    assert!(p.class_state.cp < cp_before);
    assert!(combat(&zone, joining).hp > 0);
    // Entering peace suppresses retained negative callbacks without pausing their clocks.
    let mut snapshot = zone.snapshot();
    let center = snapshot
        .phase3
        .as_ref()
        .unwrap()
        .rules
        .geometry()
        .center()
        .unwrap();
    let joined = snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == joining)
        .unwrap();
    joined.pos = center;
    let mut aoi = AoiIndex::default();
    for entity in &snapshot.entities {
        aoi.insert(entity.id, entity.pos);
    }
    snapshot.aoi = aoi.to_cells();
    let before = snapshot.phase3.as_ref().unwrap().entities[&joining]
        .effects
        .clone();
    zone = ZoneState::from_snapshot(snapshot).unwrap();
    for _ in 0..35 {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let CombatRole::Player {
        progression: Some(after),
        ..
    } = combat(&zone, joining).role
    else {
        panic!("player")
    };
    assert!(after.class_state.cp >= p.class_state.cp);
    assert_ne!(zone.snapshot().phase3.as_ref().unwrap().entities[&joining].effects, before);
}

#[test]
fn cancellation_live_reverse_order_rng_and_midcast_replay() {
    // Test-only guaranteed land/rate and max=1 expose traversal independently of chance.
    // Production source bytes are never modified.
    let mut source = Phase3DataSource::embedded();
    let cancel = source.skills.0.get_mut("skills/1177.toml").unwrap();
    *cancel = cancel
        .replace("min_chance = 10", "min_chance = 100")
        .replace("max_chance = 90", "max_chance = 100");
    cancel.truncate(cancel.find("[[effects]]").unwrap());
    cancel.push_str("[[effects]]\nsection = \"effects\"\nname = \"DispelByCategory\"\nparams = { max = \"1\", rate = \"25\", slot = \"BUFF\" }\nstat_children = []\n");
    let rules = load_phase3_rules(&source, &[]).unwrap();
    let (mut zone, player, npc) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1068, 1), (1177, 1)],
        rules,
    );
    let base = combat(&zone, npc).sheet;
    finish_skill(&mut zone, player, npc, 1040);
    finish_skill(&mut zone, player, npc, 1068);
    // Regen pays for the real source MP1+MP2 cost, avoiding a fake free cast.
    for _ in 0..300 {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let before = zone.snapshot();
    let effects: Vec<_> = before.phase3.as_ref().unwrap().entities[&npc]
        .effects
        .entries()
        .keys()
        .copied()
        .collect();
    assert_eq!(effects.len(), 2);
    let mut input = cast(player, npc);
    input.command = ZoneCommand::CastSkill {
        entity: player,
        skill_id: SkillId(1177),
        target: Some(npc),
    };
    let started = zone.run_tick(zone.draft(vec![input])).unwrap();
    assert!(started.dispositions.is_empty(), "{:?}", started.dispositions);
    let mut replay = ZoneState::from_snapshot(
        decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap(),
    )
    .unwrap();
    let mut draws = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    draws.roll_below(100);
    for _ in 0..200 {
        let draft = zone.draft(vec![]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        let reproduced = replay.run_tick(draft).unwrap();
        assert_eq!(
            AppliedTickRecord::from_applied(ZoneId(1), &applied).encode(),
            AppliedTickRecord::from_applied(ZoneId(1), &reproduced).encode()
        );
        if applied
            .events
            .iter()
            .any(|event| matches!(event, ZoneEvent::SkillCastResult { .. }))
        {
            let removed: Vec<_> = applied
                .events
                .iter()
                .filter_map(|event| match event {
                    ZoneEvent::EffectRemoved {
                        effect_id,
                        reason: effects::EffectRemoveReason::Dispelled,
                        ..
                    } => Some(*effect_id),
                    _ => None,
                })
                .collect();
            assert_eq!(removed, vec![effects[1]]);
            assert_eq!(
                zone.snapshot().rng,
                draws.snapshot().rng,
                "one eligible candidate, one draw"
            );
            assert_eq!(combat(&zone, npc).sheet.p_atk(), base.p_atk());
            assert!(combat(&zone, npc).sheet.p_def() > base.p_def());
            ZoneState::from_snapshot(zone.snapshot()).unwrap();
            return;
        }
    }
    panic!("cancellation did not complete");
}

fn ai_test_rules() -> Phase3Rules {
    let mut source = Phase3DataSource::embedded();
    for (id, handler, params) in [
        (1177, "GetAgro", "{}"),
        (1040, "AddHate", "{ power = \"500\" }"),
    ] {
        let text = source
            .skills
            .0
            .get_mut(&format!("skills/{id}.toml"))
            .unwrap();
        let rank_count: usize = if id == 1177 { 5 } else { 3 };
        let zeros = format!("[{}]", vec!["0"; rank_count].join(", "));
        *text = text
            .lines()
            .take_while(|l| *l != "[[effects]]")
            .filter(|l| !l.starts_with("abnormal ="))
            .map(|l| {
                if l.starts_with("operate_type =") {
                    "operate_type = \"A1\"".into()
                } else if l.starts_with("hit_time_ms =") {
                    "hit_time_ms = 1000".into()
                } else if l.starts_with("reuse_ms =") {
                    "reuse_ms = 0".into()
                } else if l.starts_with("activate_rate =") {
                    "activate_rate = -1".into()
                } else if let Some(key) = ["effect_point", "mp_consume_start", "mp_consume_end"]
                    .iter()
                    .find(|key| l.starts_with(&format!("{key} =")))
                {
                    format!("{key} = {zeros}")
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        write!(text, "\n[[effects]]\nsection = \"effects\"\nname = \"{handler}\"\nparams = {params}\nstat_children = []\n").unwrap();
    }
    load_phase3_rules(&source, &[]).unwrap()
}

fn setup_ai_rules(rules: Phase3Rules) -> (ZoneState, EntityId, EntityId, EntityId) {
    let (mut zone, first, bare) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1177, 1)],
        rules,
    );
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: bare })]))
        .unwrap();
    let data = parse_zone(TEST_ZONE_TOML).unwrap();
    let template = &data.npc_templates[0];
    let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
    let mut slot = SpawnSlotSpec::new(
        &SpawnSlot {
            id: "phase3-ai".into(),
            template: template.id.clone(),
            home: Vec2Fixed::from_tiles(12, 10),
            count: 1,
            respawn_delay_secs: 30,
            respawn_random_secs: 0,
        },
        template,
        NpcCombat::from_template(&stats, template).unwrap(),
    );
    slot.brain.aggressive = false;
    slot.brain.clan = None;
    zone = zone.with_spawn_slots(vec![slot]);
    let second = EntityId::from_uuid(Uuid::from_u128(900));
    let CombatRole::Player { progression, .. } = combat(&zone, first).role else {
        panic!("player")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    let applied = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: second,
            name: "Second".into(),
            pos: Vec2Fixed::from_tiles(10, 11),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })]))
        .unwrap();
    assert!(applied.dispositions.is_empty());
    let npc = zone
        .snapshot()
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Npc)
        .unwrap()
        .id;
    // Keep the NPC engaged throughout the ordering scenarios so no random-walk draw
    // contaminates the handler ledger; source skill18/28 remain deferred in real data.
    finish_skill(&mut zone, first, npc, 1040);
    (zone, first, second, npc)
}

#[test]
fn get_agro_live_forces_intention_without_hate_and_yields_on_later_think() {
    let (mut zone, first, second, npc) = setup_ai_rules(ai_test_rules());
    let before = zone.snapshot();
    let result = finish_skill(&mut zone, second, npc, 1177);
    assert_eq!(zone.entity(npc).unwrap().targeting.target, Some(second));
    assert_eq!(zone.snapshot().hate, before.hate, "GetAgro does not manufacture hate");
    assert!(!result
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::HateChanged { .. })));
    assert_eq!(
        zone.snapshot().rng.word_pos,
        before.rng.word_pos + 3,
        "one NPC auto hit during the cast; GetAgro has no intrinsic RNG"
    );
    let final_result = result
        .events
        .iter()
        .position(|e| matches!(e, ZoneEvent::SkillCastResult { .. }))
        .unwrap();
    let intention = result.events.iter().position(|e| matches!(e, ZoneEvent::TargetChanged { entity, target: Some(target), .. } if *entity == npc && *target == second)).unwrap();
    assert!(final_result < intention, "AI drains after impacts and starts");
    let mut restored = ZoneState::from_snapshot(
        decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap(),
    )
    .unwrap();
    for _ in 0..11 {
        let draft = zone.draft(vec![]);
        assert_eq!(zone.run_tick(draft.clone()).unwrap(), restored.run_tick(draft).unwrap());
    }
    assert_eq!(
        zone.entity(npc).unwrap().targeting.target,
        Some(first),
        "force has no persistent lock"
    );
}

fn edit_skill_rules(
    rules: &Phase3Rules,
    edit: impl FnOnce(&mut std::collections::BTreeMap<SkillId, skill::SkillDef>),
) -> Phase3Rules {
    let mut skills = rules.skills().skills().clone();
    edit(&mut skills);
    Phase3Rules::try_new(
        skill::SkillRegistry::try_new(skills).unwrap(),
        rules.pvp().clone(),
        rules.regen().clone(),
        rules.constants().clone(),
        rules.profiles().clone(),
        rules.geometry().clone(),
        rules.npc().clone(),
    )
    .unwrap()
}

#[test]
fn get_agro_and_hate_requests_keep_emission_order_in_the_live_tick() {
    for force_last in [false, true] {
        let rules = edit_skill_rules(&ai_test_rules(), |skills| {
            let mut hate = skills[&SkillId(1040)].effects[0].clone();
            hate.params
                .insert("power".into(), skill::RankValue::Scalar("1".into()));
            let effects = &mut skills.get_mut(&SkillId(1177)).unwrap().effects;
            if force_last {
                effects.insert(0, hate);
            } else {
                effects.push(hate);
            }
        });
        let (mut zone, first, second, npc) = setup_ai_rules(rules);
        let before = zone.snapshot().rng;
        let result = finish_skill(&mut zone, second, npc, 1177);
        let expected = if force_last { second } else { first };
        assert_eq!(zone.entity(npc).unwrap().targeting.target, Some(expected));
        assert_eq!(zone.hate_ledger(npc).unwrap().get(first).unwrap().hate, 500);
        assert_eq!(zone.hate_ledger(npc).unwrap().get(second).unwrap().hate, 1);
        assert_eq!(zone.snapshot().rng.word_pos, before.word_pos + 3);
        let targets: Vec<_> = result
            .events
            .iter()
            .filter_map(|event| match event {
                ZoneEvent::TargetChanged { entity, target, .. } if *entity == npc => *target,
                _ => None,
            })
            .collect();
        assert_eq!(
            targets,
            if force_last {
                vec![second]
            } else {
                vec![second, first]
            }
        );
        ZoneState::from_snapshot(zone.snapshot()).unwrap();
    }
}

#[test]
fn get_agro_most_hated_noop_and_add_hate_negative_source_quirk_are_live() {
    let rules = edit_skill_rules(&ai_test_rules(), |skills| {
        skills.get_mut(&SkillId(1177)).unwrap().effects = vec![{
            let mut effect = skills[&SkillId(1040)].effects[0].clone();
            effect
                .params
                .insert("power".into(), skill::RankValue::Scalar("-132".into()));
            effect
        }];
    });
    let (mut zone, first, second, npc) = setup_ai_rules(rules);
    let rng = zone.snapshot().rng;
    finish_skill(&mut zone, second, npc, 1177);
    assert!(
        zone.hate_ledger(npc).unwrap().get(second).is_none(),
        "negative branch does not create a row"
    );
    finish_skill(&mut zone, first, npc, 1177);
    assert_eq!(
        zone.hate_ledger(npc).unwrap().get(first).unwrap().hate,
        632,
        "pinned reduceHate adds positive magnitude"
    );
    assert_eq!(zone.snapshot().rng.word_pos, rng.word_pos + 3);

    let (mut zone, first, _, npc) = setup_ai_rules(ai_test_rules());
    let before = zone.snapshot();
    let result = finish_skill(&mut zone, first, npc, 1177);
    assert_eq!(zone.snapshot().hate, before.hate);
    assert!(!result
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::TargetChanged { .. } | ZoneEvent::HateChanged { .. })));
}

#[test]
fn two_get_agro_completions_use_entity_ordinals_and_force_beats_same_tick_think() {
    let (mut zone, first, second, npc) = setup_ai_rules(ai_test_rules());
    let third = EntityId::from_uuid(Uuid::from_u128(3)); // UUID order differs from admission order.
    let CombatRole::Player { progression, .. } = combat(&zone, first).role else {
        panic!("player")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: third,
        name: "Third".into(),
        pos: Vec2Fixed::from_tiles(11, 11),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(load)),
    })]))
    .unwrap();
    // Read a real cast's duration from an isolated replay branch, then align the actual
    // two completions with this NPC's scheduled think tick (no guessed cast-speed math).
    let mut probe = ZoneState::from_snapshot(zone.snapshot()).unwrap();
    let start = probe
        .run_tick(probe.draft(vec![cast(second, npc)]))
        .unwrap();
    let offset = start
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastStarted {
                start, complete_at, ..
            } => Some(complete_at.0.checked_sub(start.0).unwrap()),
            _ => None,
        })
        .unwrap();
    while zone.next_tick().0.checked_add(offset).unwrap() % 10 != NpcAi::think_phase(npc) {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let before = zone.snapshot();
    let start = zone
        .run_tick(zone.draft(vec![cast(third, npc), cast(second, npc)]))
        .unwrap();
    assert!(start.dispositions.is_empty());
    let end = start
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastStarted { complete_at, .. } => Some(*complete_at),
            _ => None,
        })
        .unwrap();
    while zone.next_tick() < end {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let applied = zone.run_tick(zone.draft(vec![])).unwrap();
    let casters: Vec<_> = applied
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::SkillCastResult { caster, .. } => Some(caster.entity),
            _ => None,
        })
        .collect();
    assert_eq!(casters, vec![second, third]);
    let targets: Vec<_> = applied
        .events
        .iter()
        .filter_map(|e| match e {
            ZoneEvent::TargetChanged { entity, target, .. } if *entity == npc => *target,
            _ => None,
        })
        .collect();
    assert_eq!(targets, vec![second, third]);
    assert_eq!(zone.entity(npc).unwrap().targeting.target, Some(third));
    assert_eq!(zone.snapshot().hate, before.hate);
    assert_eq!(zone.snapshot().rng.word_pos, before.rng.word_pos + 3);
    assert_eq!(applied.tick.0 % 10, NpcAi::think_phase(npc));
}

#[test]
fn v5_slot_death_respawn_reconstructs_new_sidecar_and_replays() {
    let rules = edit_skill_rules(&ai_test_rules(), |skills| {
        let source = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        skills.insert(SkillId(1177), source.skills().get(SkillId(1177)).unwrap().clone());
    });
    let (mut zone, caster, _, npc) = setup_ai_rules(rules);
    let original = zone.snapshot().phase3.as_ref().unwrap().entities[&npc].ordinal;
    for _ in 0..10 {
        finish_skill(&mut zone, caster, npc, 1177);
        if combat(&zone, npc).hp == 0 {
            break;
        }
        for _ in 0..30 {
            zone.run_tick(zone.draft(vec![])).unwrap();
        }
    }
    assert_eq!(combat(&zone, npc).hp, 0);
    let corpse = zone.snapshot();
    let previous_incarnation = combat(&zone, npc).incarnation;
    let mut replay =
        ZoneState::from_snapshot(decode_snapshot(&encode_snapshot(&corpse).unwrap()).unwrap())
            .unwrap();
    for _ in 0..400 {
        let draft = zone.draft(vec![]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, replay.run_tick(draft).unwrap());
        if zone
            .entity(npc)
            .and_then(|e| e.combat.as_ref())
            .is_some_and(|c| c.incarnation > previous_incarnation)
        {
            let snapshot = zone.snapshot();
            let side = &snapshot.phase3.as_ref().unwrap().entities[&npc];
            assert!(side.ordinal > original);
            assert_eq!(side.effects.entries().len(), 0);
            assert_eq!(side.regen.next_at, Some(Tick(applied.tick.0.checked_add(30).unwrap())));
            ZoneState::from_snapshot(snapshot).unwrap();
            return;
        }
    }
    panic!("slot did not respawn");
}

fn scope_rules(scope: &str, offset: u32, width: u32) -> Phase3Rules {
    let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
    edit_skill_rules(&rules, |skills| {
        let shield = skills[&SkillId(1040)].clone();
        let s = skills.get_mut(&SkillId(1177)).unwrap();
        s.effects = shield.effects;
        for handler in &mut s.effects {
            for modifier in &mut handler.stat_children {
                modifier.value = vec![modifier.value[0].clone(); 5];
            }
        }
        s.reuse_ms = skill::RankValue::Scalar(100_000);
        let mut abnormal = shield.abnormal.unwrap();
        abnormal.level = vec![1; 5];
        abnormal.time_s = vec![120; 5];
        s.abnormal = Some(abnormal);
        s.operate_type = "A2".into();
        s.target_type = skill::RankValue::Scalar(
            if scope == "POINT_BLANK" {
                "SELF"
            } else {
                "ENEMY"
            }
            .into(),
        );
        s.affect_scope = skill::RankValue::Scalar(scope.into());
        s.affect_range = Some(skill::RankValue::Scalar(64));
        s.affect_limit = skill::AffectLimit { offset, width };
        s.affect_object = "NOT_FRIEND".into();
    })
}

fn extra_npc(zone: &mut ZoneState, x: i32) -> EntityId {
    let data = parse_zone(TEST_ZONE_TOML).unwrap();
    let template = &data.npc_templates[0];
    let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
    let result = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnNpc {
            name: format!("Npc{x}"),
            pos: Vec2Fixed::from_tiles(x, 10),
            speed: template.move_speed,
            combat: Some(Box::new(NpcCombat::from_template(&stats, template).unwrap())),
        })]))
        .unwrap();
    assert!(result.dispositions.is_empty());
    result
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::EntitySpawn { entity, .. } => Some(*entity),
            _ => None,
        })
        .unwrap()
}

#[test]
fn range_captures_ordinal_targets_filters_departure_without_resampling_and_replays() {
    let (mut zone, caster, center) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1177, 1)],
        scope_rules("RANGE", 1, 0),
    );
    let inside = extra_npc(&mut zone, 13);
    let outside = extra_npc(&mut zone, 14); // Exact radius is excluded by pinned strict comparison.
    let before = zone.snapshot();
    let start = zone
        .run_tick(zone.draft(vec![cast(caster, center)]))
        .unwrap();
    assert!(start.dispositions.is_empty());
    let snapshot = zone.snapshot();
    let CastState::Casting(casting) = &snapshot.phase3.as_ref().unwrap().entities[&caster].cast
    else {
        panic!("casting")
    };
    assert_eq!(casting.affect_limit, 0, "width0 ignores offset1 and is unlimited");
    assert_eq!(
        casting.targets.iter().map(|t| t.entity).collect::<Vec<_>>(),
        vec![center, inside]
    );
    assert_eq!(snapshot.rng, before.rng);
    let complete = casting.complete_at;
    let mut replay =
        ZoneState::from_snapshot(decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap())
            .unwrap();
    let mut despawned = false;
    while zone.next_tick() <= complete {
        let inputs = if despawned {
            vec![]
        } else {
            despawned = true;
            vec![ZoneInput::system(ZoneCommand::Despawn { entity: center })]
        };
        let draft = zone.draft(inputs);
        let applied = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(applied, replay.run_tick(draft).unwrap());
        if applied.tick == complete {
            let targets = applied
                .events
                .iter()
                .find_map(|e| match e {
                    ZoneEvent::SkillCastResult { targets, .. } => Some(targets),
                    _ => None,
                })
                .unwrap();
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].target.entity, inside);
        }
    }
    let phase = zone.snapshot().phase3.unwrap();
    assert_eq!(phase.entities[&inside].effects.entries().len(), 1);
    assert!(phase.entities[&outside].effects.entries().is_empty());
    assert_eq!(zone.snapshot().rng, before.rng, "no second limit draw or handler RNG");
}

#[test]
fn point_blank_excludes_self_and_positive_limit_draws_once_after_validation() {
    let (mut zone, caster, outside) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1177, 1)],
        scope_rules("POINT_BLANK", 1, 1),
    );
    let inside = extra_npc(&mut zone, 11);
    let before = zone.snapshot();
    let mut draws = ZoneState::from_snapshot(before.clone()).unwrap();
    draws.roll_below(1);
    let applied = finish_skill(&mut zone, caster, outside, 1177);
    let targets = applied
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastResult { targets, .. } => Some(targets),
            _ => None,
        })
        .unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].target.entity, inside);
    let after = zone.snapshot();
    assert_eq!(after.rng, draws.snapshot().rng);
    assert!(after.phase3.as_ref().unwrap().entities[&caster]
        .effects
        .entries()
        .is_empty());
    assert!(after.phase3.as_ref().unwrap().entities[&outside]
        .effects
        .entries()
        .is_empty());
    // A cooldown refusal must not sample width1, charge MP1, or allocate a cast id.
    let snapshot = zone.snapshot();
    let rejected = zone
        .run_tick(zone.draft(vec![cast(caster, outside)]))
        .unwrap();
    assert_eq!(rejected.dispositions[0].reason, RejectReason::SkillCooldown);
    assert_eq!(zone.snapshot().rng, snapshot.rng);
    assert_eq!(
        zone.snapshot().phase3.as_ref().unwrap().next_cast_id,
        snapshot.phase3.as_ref().unwrap().next_cast_id
    );
}

#[test]
fn point_blank_negative_effect_range_uses_affect_fallback_at_launch() {
    let rules = edit_skill_rules(&scope_rules("POINT_BLANK", 0, 0), |skills| {
        let s = skills.get_mut(&SkillId(1177)).unwrap();
        s.cast_range = skill::RankValue::Scalar(-1);
        s.effect_range = skill::RankValue::Scalar(-1);
        s.affect_range = Some(skill::RankValue::Scalar(200));
    });
    let (mut zone, caster, npc) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1177, 1)],
        rules,
    );
    let start = zone.run_tick(zone.draft(vec![cast(caster, npc)])).unwrap();
    assert!(start.dispositions.is_empty());
    let before = zone.snapshot();
    let mut moved = false;
    for _ in 0..100 {
        let inputs = if moved {
            vec![]
        } else {
            moved = true;
            vec![ZoneInput::system(ZoneCommand::MoveTo {
                entity: npc,
                dest: Vec2Fixed::from_tiles(30, 10),
            })]
        };
        let applied = zone.run_tick(zone.draft(inputs)).unwrap();
        assert!(applied.dispositions.is_empty());
        if applied.events.iter().any(|e| {
            matches!(
                e,
                ZoneEvent::SkillCastCancelled {
                    reason: state_skill::CastCancelReason::OutOfRange,
                    ..
                }
            )
        }) {
            assert_eq!(zone.snapshot().rng, before.rng);
            assert!(zone.snapshot().phase3.as_ref().unwrap().entities[&npc]
                .effects
                .entries()
                .is_empty());
            return;
        }
    }
    panic!("escaping point-blank target must cancel before impact");
}

#[test]
#[allow(clippy::too_many_lines)] // Full source caller and replay assertions stay together.
fn physical_skill_live_cost_source_rng_damage_and_lethal_reflection_replay() {
    for (critical_chance, ranged, lethal) in
        [(0, false, false), (100, false, true), (0, true, false)]
    {
        let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let rules = edit_skill_rules(&base, |skills| {
            let mut physical = skills[&SkillId(3)].effects[0].clone();
            physical
                .params
                .insert("power".into(), skill::RankValue::Scalar("30".into()));
            let conditions = skills[&SkillId(3)].conditions.clone();
            let s = skills.get_mut(&SkillId(1177)).unwrap();
            s.effects = vec![physical];
            s.conditions = conditions;
            s.is_magic = 0;
            s.mp_consume_start.fill(0);
            s.mp_consume_end.fill(9);
            s.cast_range = skill::RankValue::Scalar(40);
            s.effects[0].params.insert(
                "criticalChance".into(),
                skill::RankValue::Scalar(critical_chance.to_string()),
            );
            if ranged {
                s.cast_range = skill::RankValue::Scalar(400);
            }
        });
        let (zone, player, npc) = setup_with_rules(
            Vec2Fixed::from_tiles(10, 10),
            Vec2Fixed::from_tiles(11, 10),
            &[(1177, 1)],
            rules,
        );
        let mut snapshot = zone.snapshot();
        if lethal {
            snapshot
                .entities
                .iter_mut()
                .find(|e| e.id == npc)
                .unwrap()
                .combat
                .as_mut()
                .unwrap()
                .hp = 1;
        }
        let mut zone = ZoneState::from_snapshot(snapshot).unwrap();
        let mut input = cast(player, npc);
        input.command = ZoneCommand::CastSkill {
            entity: player,
            skill_id: SkillId(1177),
            target: Some(npc),
        };
        let initial_rng = zone.snapshot().rng;
        let mp = combat(&zone, player).mp;
        let start = zone.run_tick(zone.draft(vec![input])).unwrap();
        assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
        assert_eq!(zone.snapshot().rng, initial_rng);
        assert_eq!(combat(&zone, player).mp, mp); // Power Strike has no MP1.
        let complete = start
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::SkillCastStarted { complete_at, .. } => Some(*complete_at),
                _ => None,
            })
            .unwrap();
        let mut replay = ZoneState::from_snapshot(
            decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap(),
        )
        .unwrap();
        while zone.draft(vec![]).tick < complete {
            let draft = zone.draft(vec![]);
            let a = zone.run_tick(draft.clone()).unwrap();
            let b = replay.run_tick(draft).unwrap();
            assert_eq!(
                AppliedTickRecord::from_applied(ZoneId(1), &a).encode(),
                AppliedTickRecord::from_applied(ZoneId(1), &b).encode()
            );
        }
        let before = combat(&zone, npc);
        let attacker = combat(&zone, player);
        let mut ledger = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        ledger.roll_below(100); // physical evasion at stat0
        if critical_chance != 0 {
            ledger.roll_below(100_000_000);
        }
        let spread = i64::from(ledger.roll_below(21)).checked_sub(10).unwrap();
        if !ranged {
            ledger.roll_below(100);
        } // after reduction, even after death
        let expected = skill_math::physical_skill_base(
            skill_math::ExactRatio::whole(30),
            skill_math::ExactRatio::new(i128::from(attacker.sheet.p_atk().raw()), i128::from(Q))
                .unwrap(),
            skill_math::ExactRatio::new(i128::from(before.sheet.p_def().raw()), i128::from(Q))
                .unwrap(),
            skill_math::ExactRatio::whole(1),
            false,
        )
        .unwrap()
        .checked_mul(
            skill_math::ExactRatio::new(i128::from(100_i64.checked_add(spread).unwrap()), 100)
                .unwrap(),
        )
        .unwrap();
        let draft = zone.draft(vec![]);
        let applied = zone.run_tick(draft.clone()).unwrap();
        let reproduced = replay.run_tick(draft).unwrap();
        assert_eq!(zone.snapshot().rng, ledger.snapshot().rng);
        assert_eq!(
            AppliedTickRecord::from_applied(ZoneId(1), &applied).encode(),
            AppliedTickRecord::from_applied(ZoneId(1), &reproduced).encode()
        );
        let row = applied
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::SkillCastResult { targets, .. } => targets.first(),
                _ => None,
            })
            .unwrap();
        let multiplier = if row.outcome == SkillOutcome::Crit {
            2
        } else {
            1
        };
        assert_eq!(row.resolution_kind, SkillResolutionKind::Physical);
        assert_eq!(
            row.rolled,
            u32::try_from(
                expected
                    .checked_mul(skill_math::ExactRatio::whole(multiplier))
                    .unwrap()
                    .trunc()
                    .unwrap()
            )
            .unwrap()
        );
        assert_eq!(row.hp_removed, row.rolled.min(before.hp));
        assert_eq!(combat(&zone, player).mp, attacker.mp.checked_sub(9).unwrap());
        if lethal {
            assert!(applied
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::SpGained { amount: 2, .. })));
        }
        ZoneState::from_snapshot(zone.snapshot()).unwrap();
    }
}

#[test]
fn physical_weapon_condition_refuses_before_cost_reuse_or_rng() {
    let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
    let rules = edit_skill_rules(&base, |skills| {
        let conditions = skills[&SkillId(24)].conditions.clone();
        skills.get_mut(&SkillId(1177)).unwrap().conditions = conditions;
    });
    let (mut zone, player, npc) = setup_with_rules(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(11, 10),
        &[(1177, 1)],
        rules,
    );
    let before = zone.snapshot().rng;
    let mp = combat(&zone, player).mp;
    let mut input = cast(player, npc);
    input.command = ZoneCommand::CastSkill {
        entity: player,
        skill_id: SkillId(1177),
        target: Some(npc),
    };
    let applied = zone.run_tick(zone.draft(vec![input])).unwrap();
    assert!(applied
        .dispositions
        .iter()
        .any(|d| d.reason == RejectReason::WeaponCondition));
    assert_eq!(zone.snapshot().rng, before);
    assert_eq!(combat(&zone, player).mp, mp);
    assert!(zone.snapshot().phase3.unwrap().entities[&player]
        .reuse
        .is_empty());
}

#[test]
fn mixed_magic_continuous_success_precedes_instant_and_stronger_stack_keeps_damage() {
    for (lands, blocked) in [(false, false), (true, false), (true, true)] {
        let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let rules = edit_skill_rules(&base, |skills| {
            let mut debuff = skills[&SkillId(1040)].effects[0].clone();
            debuff.name = "Debuff".into();
            debuff.stat_children[0].value = vec!["0.9".into(); 5];
            let mut abnormal = skills[&SkillId(1040)].abnormal.clone().unwrap();
            abnormal.level = vec![1; 5];
            abnormal.time_s = vec![60; 5];
            abnormal.is_debuff = true;
            skills
                .get_mut(&SkillId(1040))
                .unwrap()
                .abnormal
                .as_mut()
                .unwrap()
                .level
                .fill(2);
            let s = skills.get_mut(&SkillId(1177)).unwrap();
            s.effects[0].name = "MagicalAttack".into();
            s.effects[0].params.remove("shieldDefensePercent");
            s.effects[0]
                .params
                .insert("power".into(), skill::RankValue::Scalar("1".into()));
            s.effects.push(debuff);
            s.abnormal = Some(abnormal);
            s.operate_type = "A2".into();
            s.activate_rate = 60;
            s.basic_property = "MEN".into();
            s.min_chance = 50;
            s.max_chance = 50;
        });
        let (mut zone, player, npc) = setup_with_rules(
            Vec2Fixed::from_tiles(10, 10),
            Vec2Fixed::from_tiles(12, 10),
            &[(1040, 1), (1177, 1)],
            rules,
        );
        if blocked {
            finish_skill(&mut zone, player, npc, 1040);
        }
        let start = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
        assert!(start.dispositions.is_empty());
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
        let original = zone.snapshot();
        // Choose a deterministic recorded RNG input covering each container branch.
        // The injected math tests separately cover exact boundary rolls.
        let mut selected = None;
        for cursor in 0..100 {
            let mut snapshot = original.clone();
            snapshot.rng.word_pos = cursor;
            let mut preview = ZoneState::from_snapshot(snapshot.clone()).unwrap();
            preview.roll_below(100); // outer debuff reflection, even at zero
            if (preview.roll_below(100) < 50) == lands {
                selected = Some(snapshot);
                break;
            }
        }
        zone = ZoneState::from_snapshot(selected.unwrap()).unwrap();
        let mut ledger = ZoneState::from_snapshot(zone.snapshot()).unwrap();
        ledger.roll_below(100);
        ledger.roll_below(100); // success then duration (no RNG)
        ledger.roll_below(1000);
        if ledger.roll_below(100) >= 99 {
            ledger.roll_below(100);
        }
        ledger.roll_below(21);
        ledger.roll_below(100);
        let hp = combat(&zone, npc).hp;
        let applied = zone.run_tick(zone.draft(vec![])).unwrap();
        assert_eq!(zone.snapshot().rng, ledger.snapshot().rng);
        let rows = applied
            .events
            .iter()
            .find_map(|e| match e {
                ZoneEvent::SkillCastResult { targets, .. } => Some(targets),
                _ => None,
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].resolution_kind, SkillResolutionKind::Magical);
        assert_eq!(rows[1].resolution_kind, SkillResolutionKind::Effect);
        assert!(rows[0].hp_removed > 0);
        assert_eq!(combat(&zone, npc).hp, hp.checked_sub(rows[0].hp_removed).unwrap());
        assert_eq!(!rows[1].applied_effect_ids.is_empty(), lands && !blocked);
        if !lands {
            assert_eq!(rows[1].outcome, SkillOutcome::Resist);
        }
        let snapshot = zone.snapshot();
        let effects = snapshot.phase3.as_ref().unwrap().entities[&npc]
            .effects
            .entries();
        if blocked {
            assert!(effects.values().all(|e| e.skill.id == SkillId(1040)));
        }
        ZoneState::from_snapshot(snapshot).unwrap();
    }
}

#[test]
fn hashed_high_level_npc_damage_policy_is_admitted_at_epoch_boundary() {
    let stats = load_rules(&RulesSource::embedded()).unwrap().rules;
    let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
    let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
    let mut profiles = base.profiles().clone();
    profiles.npcs.get_mut("keltir").unwrap().physical.level = 78;
    let rules = Phase3Rules::try_new(
        base.skills().clone(),
        base.pvp().clone(),
        base.regen().clone(),
        base.constants().clone(),
        profiles,
        base.geometry().clone(),
        base.npc().clone(),
    )
    .unwrap();
    let result = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(1),
            epoch: 45,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stats,
        classes,
        rules,
    );
    let zone = result.unwrap();
    ZoneState::from_snapshot(decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap())
        .unwrap();
}

#[test]
fn invalid_v5_intents_are_recorded_without_holding_supported_commands() {
    let (mut zone, player, npc) = setup();
    let mut replay = ZoneState::from_snapshot(
        decode_snapshot(&encode_snapshot(&zone.snapshot()).unwrap()).unwrap(),
    )
    .unwrap();
    let source = CommandSource::Session {
        entity: player,
        generation: SessionGeneration(1),
    };
    let commands = [
        ZoneCommand::Attack { entity: player },
        ZoneCommand::Respawn { entity: player },
        ZoneCommand::AcceptResurrect {
            entity: player,
            offer_id: OfferId(1),
            accept: true,
        },
        ZoneCommand::ChangeClass {
            entity: player,
            account: AccountId::from_uuid(Uuid::from_u128(2)),
            request_key: Uuid::from_u128(400),
            target: ClassId(11),
        },
    ];
    let mut inputs: Vec<_> = commands
        .into_iter()
        .enumerate()
        .map(|(index, command)| ZoneInput {
            source,
            seq: Some(u32::try_from(index).unwrap()),
            command,
        })
        .collect();
    inputs.push(ZoneInput::system(ZoneCommand::AddAggro {
        npc,
        target: player,
    }));
    inputs.push(ZoneInput {
        source: CommandSource::Session {
            entity: player,
            generation: SessionGeneration(0),
        },
        seq: Some(10),
        command: ZoneCommand::Attack { entity: player },
    });
    inputs.push(ZoneInput::system(ZoneCommand::AcceptResurrect {
        entity: player,
        offer_id: OfferId(1),
        accept: true,
    }));
    inputs.push(cast(player, npc));
    let before_rng = zone.snapshot().rng;
    let draft = zone.draft(inputs);
    let applied = zone.run_tick(draft.clone()).unwrap();
    assert_eq!(applied.dispositions.len(), 7);
    assert_eq!(zone.snapshot().rng, before_rng);
    assert_eq!(
        applied.outputs[&player]
            .iter()
            .filter(|output| matches!(output, ObserverOutput::Rejected(_)))
            .count(),
        4
    );
    for (index, disposition) in applied.dispositions[..5].iter().enumerate() {
        assert_eq!(
            disposition.reason,
            match index {
                0 => RejectReason::InvalidTargetType,
                1 => RejectReason::NotDead,
                2 => RejectReason::NoResurrectOffer,
                3 => RejectReason::InCombat,
                _ => RejectReason::NotYetImplemented,
            }
        );
    }
    assert_eq!(applied.dispositions[5].reason, RejectReason::StaleSession);
    assert_eq!(applied.dispositions[6].reason, RejectReason::NotPermitted);
    assert!(applied.events.iter().any(|e| matches!(e,
        ZoneEvent::SkillCastStarted { caster, .. } if caster.entity == player)));
    assert_eq!(
        AppliedTickRecord::from_applied(ZoneId(1), &applied).encode(),
        AppliedTickRecord::from_applied(ZoneId(1), &replay.run_tick(draft).unwrap()).encode()
    );
    // A refused command does not remain at the head of a replayed or live command stream.
    let next = zone.run_tick(zone.draft(vec![])).unwrap();
    assert!(next.dispositions.is_empty());
    assert_eq!(next.tick, applied.tick.next());
}

#[tokio::test]
async fn experimental_actor_does_not_hold_unsupported_intents_at_fifo_head() {
    use nightfall_api::application::zone_actor::{manual_ticks, OpenGate, ZoneActor};
    let (state, player, npc) = setup();
    let (ticks, driver) = manual_ticks();
    let actor = ZoneActor::spawn_gated(state, ticks, OpenGate);
    let mut output = actor.subscribe();
    actor
        .send(ZoneInput::session(
            player,
            SessionGeneration(1),
            1,
            ZoneCommand::AcceptResurrect {
                entity: player,
                offer_id: OfferId(1),
                accept: true,
            },
        ))
        .unwrap();
    actor.send(cast(player, npc)).unwrap();
    driver.step().await.unwrap();
    let tick = tokio::time::timeout(std::time::Duration::from_secs(3), output.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tick.commands.len(), 2);
    assert_eq!(tick.dispositions.len(), 1);
    assert_eq!(tick.dispositions[0].reason, RejectReason::NoResurrectOffer);
    assert!(tick.events.iter().any(|event| matches!(event,
        ZoneEvent::SkillCastStarted { caster, .. } if caster.entity == player)));
    // Idle ticks deliberately suppress broadcasts; the driver acknowledgement proves
    // progress without requiring an output that the actor never promises to publish.
    assert_eq!(
        driver.step().await.unwrap(),
        nightfall_api::application::zone_actor::TickOutcome::Ran(tick.tick.next())
    );
}

#[test]
fn high_level_npc_skill_penalty_is_live_and_midcast_snapshot_keeps_the_rules() {
    fn run(penalties: bool, physical: bool, boundary: bool) -> (u32, SkillOutcome) {
        let base = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
        let mut profiles = base.profiles().clone();
        let profile = profiles.npcs.get_mut("keltir").unwrap();
        profile.physical.level = 78;
        profile.physical.max_hp = 1_000_000;
        let mut npc = base.npc().clone();
        if !penalties {
            let minimum = npc
                .penalty
                .config
                .get_mut("MinNPCLevelForDmgPenalty")
                .unwrap();
            minimum.values = vec!["99".into()];
            minimum.values_q = vec![99_000_000];
        }
        let base = Phase3Rules::try_new(
            base.skills().clone(),
            base.pvp().clone(),
            base.regen().clone(),
            base.constants().clone(),
            profiles,
            base.geometry().clone(),
            npc,
        )
        .unwrap();
        let rules = edit_skill_rules(&base, |skills| {
            let skill = skills.get_mut(&SkillId(1177)).unwrap();
            skill.magic_level[0] = 78;
            if physical {
                skill.is_magic = 0;
                skill.effects[0].name = "PhysicalAttack".into();
                skill.effects[0].params.clear();
            }
            skill.effects[0]
                .params
                .insert("power".into(), skill::RankValue::Scalar("100".into()));
        });
        let (mut zone, player, npc) = setup_with_rules(
            Vec2Fixed::from_tiles(10, 10),
            Vec2Fixed::from_tiles(12, 10),
            &[(1177, 1)],
            rules,
        );
        if boundary {
            let mut ledger = zone.clone();
            let mut found = false;
            for _ in 0..1000 {
                let mut probe = ledger.clone();
                probe.roll_below(1000);
                if (96..99).contains(&probe.roll_below(100)) {
                    let mut snapshot = zone.snapshot();
                    snapshot.rng = ledger.snapshot().rng;
                    zone = ZoneState::from_snapshot(snapshot).unwrap();
                    found = true;
                    break;
                }
                ledger.roll_below(100);
            }
            assert!(found, "source chance boundary seed");
        }
        let applied = zone.run_tick(zone.draft(vec![cast(player, npc)])).unwrap();
        assert!(applied.dispositions.is_empty());
        let snapshot = zone.snapshot();
        let mut replay = ZoneState::from_snapshot(
            decode_snapshot(&encode_snapshot(&snapshot).unwrap()).unwrap(),
        )
        .unwrap();
        for _ in 0..100 {
            let draft = zone.draft(vec![]);
            let applied = zone.run_tick(draft.clone()).unwrap();
            assert_eq!(applied, replay.run_tick(draft).unwrap());
            if let Some(row) = applied.events.iter().find_map(|e| match e {
                ZoneEvent::SkillCastResult { targets, .. } => targets.first(),
                _ => None,
            }) {
                assert_eq!(row.hp_removed, row.rolled);
                return (row.rolled, row.outcome);
            }
        }
        panic!("no skill completion");
    }
    for (physical, numerator, denominator) in [(false, 62_u64, 100_u64), (true, 341, 1000)] {
        let (penalized, outcome) = run(true, physical, false);
        let (control, control_outcome) = run(false, physical, false);
        assert_eq!(outcome, control_outcome);
        assert!(matches!(outcome, SkillOutcome::Hit | SkillOutcome::Crit));
        assert!(penalized > 0 && penalized < control);
        // Saturated magic factor0.62; physical skill additionally takes normal factor0.55.
        // Whole intervals prove the factor without multiplying an already truncated value.
        let lower = u64::from(control).checked_mul(numerator).unwrap() / denominator;
        let upper = u64::from(control)
            .checked_add(1)
            .unwrap()
            .checked_mul(numerator)
            .unwrap()
            / denominator;
        assert!((lower..=upper).contains(&u64::from(penalized)));
    }
    // magicLvl78 isolates failure scaling from the acting-player gap77. The same source
    // roll96..98 is accepted at99 without the NPC penalty and rejected at96 with it.
    let (penalized, outcome) = run(true, false, true);
    let (control, control_outcome) = run(false, false, true);
    assert_eq!(outcome, SkillOutcome::Resist);
    assert!(matches!(control_outcome, SkillOutcome::Hit | SkillOutcome::Crit));
    assert_eq!(penalized, 0);
    assert!(control > 0);
}

#[test]
#[allow(clippy::too_many_lines)] // One live recording checks full, digest-only, and baseline suffix coverage.
fn schema_three_coverage_counts_live_lifecycles_once_and_verifies_the_suffix() {
    use nightfall_api::application::replay_log::{Watermark, WatermarkReason};
    use nightfall_api::infrastructure::eventlog::Recording;

    fn skill_cast(player: EntityId, target: EntityId, skill_id: u32) -> ZoneInput {
        let mut input = cast(player, target);
        input.command = ZoneCommand::CastSkill {
            entity: player,
            target: Some(target),
            skill_id: SkillId(skill_id),
        };
        input
    }
    fn cancel(player: EntityId, target: EntityId) -> ZoneInput {
        let mut input = cast(player, target);
        input.command = ZoneCommand::CancelCast {
            entity: player,
            cast_id: None,
        };
        input
    }
    fn record(zone: &mut ZoneState, rec: &mut Recording, inputs: Vec<ZoneInput>) -> AppliedTick {
        let applied = zone.run_tick(zone.draft(inputs)).unwrap();
        rec.records
            .push(AppliedTickRecord::from_applied(rec.snapshot.seed.zone, &applied));
        rec.watermark.last_tick = Some(applied.tick);
        rec.watermark.records = u64::try_from(rec.records.len()).unwrap();
        applied
    }
    fn cover(rec: &Recording) -> Result<serde_json::Value, String> {
        let prefix = std::env::temp_dir().join(format!("phase3-coverage-{}", Uuid::new_v4()));
        let file = prefix.with_extension("nfr");
        let out = prefix.with_extension("json");
        rec.write(&file).unwrap();
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_nightfall-replay"))
            .arg("coverage")
            .arg("--file")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap();
        std::fs::remove_file(file).unwrap();
        if !result.status.success() {
            assert!(!out.exists(), "diverged coverage must not publish a report");
            return Err(String::from_utf8_lossy(&result.stderr).into_owned());
        }
        let json = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        std::fs::remove_file(out).unwrap();
        Ok(json)
    }
    fn count(report: &serde_json::Value, table: &str, from: &str, to: &str) -> u64 {
        report[table]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["from"] == from && row["to"] == to)
            .unwrap()["count"]
            .as_u64()
            .unwrap()
    }
    let (mut zone, player, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1), (1177, 1)],
    );
    let mut rec = Recording {
        snapshot: zone.snapshot(),
        watermark: Watermark {
            zone: ZoneId(1),
            epoch: zone.snapshot().seed.epoch,
            last_tick: None,
            records: 0,
            reason: WatermarkReason::Capture,
        },
        records: Vec::new(),
    };
    let mut suffix_start = None;
    for number in 0..2 {
        let start = record(&mut zone, &mut rec, vec![skill_cast(player, npc, 1040)]);
        assert!(start
            .events
            .iter()
            .any(|e| matches!(e, ZoneEvent::SkillCastStarted { .. })));
        if number == 1 {
            suffix_start = Some((zone.snapshot(), rec.records.len()));
        }
        loop {
            let applied = record(&mut zone, &mut rec, vec![]);
            if applied
                .events
                .iter()
                .any(|e| matches!(e, ZoneEvent::SkillCastResult { .. }))
            {
                break;
            }
        }
        for _ in 0..25 {
            record(&mut zone, &mut rec, vec![]);
        }
    }
    // AOI entry hydrates the existing effect but must not count another installation.
    let load = match combat(&zone, player).role {
        CombatRole::Player { progression, .. } => {
            let mut load = PlayerLoad::fresh("human_mystic");
            load.progression = progression;
            load
        },
        CombatRole::Npc { .. } => panic!("player"),
    };
    let observer = EntityId::from_uuid(Uuid::from_u128(900));
    let joined = record(
        &mut zone,
        &mut rec,
        vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: observer,
            name: "Observer".into(),
            pos: Vec2Fixed::from_tiles(11, 10),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(1),
            load: Some(Box::new(load)),
        })],
    );
    assert!(joined.dispositions.is_empty());
    assert!(joined.outputs[&observer].iter().any(|output| matches!(
        output,
        ObserverOutput::Event(ZoneEvent::EffectApplied {
            baseline: Some(_),
            ..
        })
    )));
    let start = record(&mut zone, &mut rec, vec![cast(player, npc)]);
    assert!(start.dispositions.is_empty(), "{:?}", start.dispositions);
    record(&mut zone, &mut rec, vec![cancel(player, npc)]);
    // Same-tick staged cancellation has no accepted start or terminal cast fact.
    record(&mut zone, &mut rec, vec![cast(player, npc), cancel(player, npc)]);
    record(&mut zone, &mut rec, vec![skill_cast(player, npc, u32::MAX)]);
    let report = cover(&rec).unwrap();
    assert_eq!(report["schema_version"], 3);
    assert_eq!(count(&report, "cast_states", "Idle", "Casting"), 3);
    assert_eq!(count(&report, "cast_states", "Casting", "Idle"), 3);
    assert_eq!(report["cast_end_reasons"], serde_json::json!({"Completed": 2, "Stopped": 1}));
    assert_eq!(count(&report, "effect_states", "Absent", "Active"), 2);
    assert_eq!(count(&report, "effect_states", "Active", "Removed"), 1);
    assert_eq!(report["effect_remove_reasons"], serde_json::json!({"Replaced": 1}));
    assert_eq!(report["intent_rejected"]["SkillUnknown"], 1);
    assert_eq!(report["learning_committed"], 0);
    assert_eq!(report["cp_absorption"], serde_json::json!({}));
    for table in ["cast_states", "effect_states"] {
        for row in report[table].as_array().unwrap() {
            if row["reachable"] == false {
                assert_eq!(row["count"], 0);
            }
        }
    }
    let mut digests = rec.clone();
    digests.records = digests
        .records
        .iter()
        .map(AppliedTickRecord::with_output_digests)
        .collect();
    assert_eq!(cover(&digests).unwrap(), report);
    let (snapshot, index) = suffix_start.unwrap();
    let mut suffix = rec.clone();
    suffix.snapshot = snapshot;
    suffix.records.drain(..index);
    suffix.watermark.records = u64::try_from(suffix.records.len()).unwrap();
    let report = cover(&suffix).unwrap();
    assert_eq!(count(&report, "cast_states", "Idle", "Casting"), 1);
    assert_eq!(count(&report, "cast_states", "Casting", "Idle"), 2);
    assert_eq!(count(&report, "effect_states", "Absent", "Active"), 1);
    assert_eq!(count(&report, "effect_states", "Active", "Removed"), 1);
    // A validly encoded event mutation is still rejected before coverage is emitted.
    let mut bad = rec.clone();
    let first = bad.records.first_mut().unwrap();
    let mut facts = nightfall_api::application::replay_log::decode_events(&first.events).unwrap();
    let started = facts
        .iter_mut()
        .find(|e| matches!(e, ZoneEvent::SkillCastStarted { .. }))
        .unwrap();
    if let ZoneEvent::SkillCastStarted { complete_at, .. } = started {
        *complete_at = complete_at.next();
    }
    first.events = nightfall_api::application::replay_log::encode_events(&facts);
    assert!(cover(&bad).unwrap_err().contains("divergence"));
    let first = digests.records.first_mut().unwrap();
    let mut bytes = first.events.to_vec();
    bytes[0] ^= 1;
    first.events = bytes.into();
    assert!(cover(&digests).unwrap_err().contains("divergence"));
    // Explicit offline generator only; ordinary tests never rewrite a committed fixture.
    if let Some(path) = std::env::var_os("NIGHTFALL_V5_RECORDING_OUT") {
        rec.write(std::path::Path::new(&path)).unwrap();
    }
}

fn spawn_observer_v5(
    zone: &mut ZoneState,
    source: EntityId,
    observer: EntityId,
    x: i32,
) -> AppliedTick {
    let CombatRole::Player { progression, .. } = combat(zone, source).role else {
        panic!("player fixture")
    };
    let mut load = PlayerLoad::fresh("human_mystic");
    load.progression = progression;
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
        entity: observer,
        name: "Viewer".into(),
        pos: Vec2Fixed::from_tiles(x, 10),
        speed: Speed::DEFAULT,
        generation: SessionGeneration(1),
        load: Some(Box::new(load)),
    })]))
    .unwrap()
}

fn wire_events_for(tick: &AppliedTick, observer: EntityId) -> Vec<pb::world_event::Event> {
    use prost::Message as _;
    nightfall_api::interface::zone_mapping::encode_observer_outputs(
        tick.outputs.get(&observer).map_or(&[], Vec::as_slice),
        tick.server_time_ms,
    )
    .iter()
    .filter_map(|frame| {
        let decoded = pb::ServerMessage::decode(frame.as_ref()).unwrap();
        match decoded.payload {
            Some(pb::server_message::Payload::Event(world)) => world.event,
            _ => None,
        }
    })
    .collect()
}

#[test]
fn aoe_wire_rows_are_filtered_per_observer_without_reordering_or_private_totals() {
    use pb::world_event::Event;
    let mut rules = scope_rules("RANGE", 0, 0);
    rules = edit_skill_rules(&rules, |skills| {
        skills.get_mut(&SkillId(1177)).unwrap().affect_range = Some(skill::RankValue::Scalar(128));
    });
    let (mut zone, caster, center) = setup_with_rules(
        Vec2Fixed::from_tiles(60, 10),
        Vec2Fixed::from_tiles(62, 10),
        &[(1177, 1)],
        rules,
    );
    let visible = extra_npc(&mut zone, 63);
    let hidden = extra_npc(&mut zone, 64);
    let observer = EntityId::from_uuid(Uuid::from_u128(333));
    let admission = spawn_observer_v5(&mut zone, caster, observer, 0);
    assert!(admission.dispositions.is_empty());
    let identities = wire_events_for(&admission, observer);
    assert!(identities
        .iter()
        .any(|e| matches!(e, Event::Spawn(s) if s.entity_id == visible.to_string())));
    assert!(!identities
        .iter()
        .any(|e| matches!(e, Event::Spawn(s) if s.entity_id == hidden.to_string())));
    let result = finish_skill(&mut zone, caster, center, 1177);
    let owner = wire_events_for(&result, caster);
    let public = wire_events_for(&result, observer);
    let rows = |events: Vec<Event>| {
        events
            .into_iter()
            .find_map(|e| match e {
                Event::SkillCastResult(r) => Some(r.targets),
                _ => None,
            })
            .unwrap()
    };
    let owner_rows = rows(owner);
    let observer_rows = rows(public.clone());
    assert_eq!(
        owner_rows.iter().map(|r| &r.target).collect::<Vec<_>>(),
        vec![
            &center.to_string(),
            &visible.to_string(),
            &hidden.to_string()
        ]
    );
    assert_eq!(
        observer_rows,
        owner_rows
            .into_iter()
            .filter(|r| r.target != hidden.to_string())
            .collect::<Vec<_>>()
    );
    assert!(!public.iter().any(|e| matches!(e,
        Event::StatsChanged(s) if s.entity == caster.to_string())));
    assert!(!public.iter().any(|e| matches!(e,
        Event::SkillReuse(s) if s.entity == caster.to_string())));
    assert!(!public.iter().any(|e| matches!(e,
        Event::SkillStateSync(s) if s.entity == caster.to_string())));
}

#[test]
fn mutual_buff_cast_hydration_wire_follows_all_identities_without_replaying_mutations() {
    use pb::world_event::Event;
    let (mut zone, first, _) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1)],
    );
    let second = EntityId::from_uuid(Uuid::from_u128(444));
    assert!(spawn_observer_v5(&mut zone, first, second, 11)
        .dispositions
        .is_empty());
    let started = zone
        .run_tick(
            zone.draft(
                vec![first, second]
                    .into_iter()
                    .enumerate()
                    .map(|(i, entity)| {
                        ZoneInput::session(
                            entity,
                            SessionGeneration(1),
                            u32::try_from(i).unwrap() + 1,
                            ZoneCommand::CastSkill {
                                entity,
                                skill_id: SkillId(1040),
                                target: Some(if entity == first { second } else { first }),
                            },
                        )
                    })
                    .collect(),
            ),
        )
        .unwrap();
    assert!(started.dispositions.is_empty());
    let before = zone.snapshot();
    let viewer = EntityId::from_uuid(Uuid::from_u128(555));
    let admission = spawn_observer_v5(&mut zone, first, viewer, 10);
    let wire = wire_events_for(&admission, viewer);
    let last_identity = wire
        .iter()
        .rposition(|e| matches!(e, Event::Spawn(_)))
        .unwrap();
    let casts: Vec<_> = wire
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::SkillCastStarted(c) if c.caster != viewer.to_string() => Some((i, c)),
            _ => None,
        })
        .collect();
    assert_eq!(casts.len(), 2);
    let after = zone.snapshot();
    for (index, cast) in casts {
        assert!(index > last_identity);
        assert!(cast.baseline);
        assert_eq!(cast.baseline_tick, admission.tick.0);
        let entity = if cast.caster == first.to_string() {
            first
        } else {
            second
        };
        let CastState::Casting(original) = &before.phase3.as_ref().unwrap().entities[&entity].cast
        else {
            panic!("cast fixture")
        };
        assert_eq!(
            (cast.cast_id, cast.start_tick, cast.launch_tick, cast.complete_tick),
            (original.id.0, original.start_at.0, original.launch_at.0, original.complete_at.0)
        );
        assert_eq!(cast.target, original.target.map(|t| t.entity.to_string()));
        assert_eq!(cast.target_incarnation, original.target.map(|t| t.incarnation));
        assert_eq!(
            after.phase3.as_ref().unwrap().entities[&entity].cast,
            before.phase3.as_ref().unwrap().entities[&entity].cast
        );
        assert!(!wire
            .iter()
            .any(|e| matches!(e, Event::StatsChanged(s) if s.entity == entity.to_string())));
        assert!(!wire
            .iter()
            .any(|e| matches!(e, Event::SkillReuse(s) if s.entity == entity.to_string())));
        assert!(!wire
            .iter()
            .any(|e| matches!(e, Event::SkillStateSync(s) if s.entity == entity.to_string())));
    }
    assert!(wire
        .iter()
        .filter_map(|e| match e {
            Event::Spawn(s) => Some(s),
            _ => None,
        })
        .all(|s| s.pending_cast.is_none()
            && s.pending_swing.is_none()
            && s.active_effects.is_empty()));
}

#[test]
fn npc_effect_hydration_wire_uses_identity_tick_without_changing_legacy_spawn_bytes(
) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use nightfall_api::interface::zone_mapping::{spawn_to_pb, world_event_to_pb};
    use pb::world_event::Event;
    use prost::Message as _;
    let (mut zone, caster, npc) = setup_with_skills(
        Vec2Fixed::from_tiles(10, 10),
        Vec2Fixed::from_tiles(12, 10),
        &[(1040, 1)],
    );
    finish_skill(&mut zone, caster, npc, 1040);
    let viewer = EntityId::from_uuid(Uuid::from_u128(556));
    let admission = spawn_observer_v5(&mut zone, caster, viewer, 11);
    let events = wire_events_for(&admission, viewer);
    let identity = events
        .iter()
        .find_map(|e| match e {
            Event::Spawn(s) if s.entity_id == npc.to_string() => Some(s),
            _ => None,
        })
        .context("NPC identity frame")?;
    assert_eq!(identity.state_tick, admission.tick.0);
    let effect = events
        .iter()
        .find_map(|e| match e {
            Event::EffectApplied(e) if e.target == npc.to_string() && e.skill_id == 1040 => Some(e),
            _ => None,
        })
        .context("existing NPC effect hydration frame")?;
    assert!(effect.baseline);
    assert_eq!(effect.target_incarnation, identity.life_incarnation);
    assert_eq!((effect.tick, effect.baseline_tick), (identity.state_tick, identity.state_tick));
    // Client fence contract (architecture-decisions C12): hydration upserts at the identity
    // tick; ordinary deltas need a strictly newer tick. The client owner tests its projection.

    let last_identity = events
        .iter()
        .rposition(|e| matches!(e, Event::Spawn(_)))
        .context("identity batch")?;
    let effect_index = events
        .iter()
        .position(|e| matches!(e, Event::EffectApplied(e) if e.effect_id == effect.effect_id))
        .context("hydration index")?;
    assert!(effect_index > last_identity);
    let frames = nightfall_api::interface::zone_mapping::encode_observer_outputs(
        admission.outputs.get(&viewer).context("viewer outputs")?,
        admission.server_time_ms,
    )
    .iter()
    .map(|bytes| pb::ServerMessage::decode(bytes.as_ref()))
    .collect::<Result<Vec<_>, _>>()?;
    hydration_fixture::write("npc-effects", &frames)?;

    // Reproject the actual NPC producer with its legacy combat policy. The previous spawn
    // mapper's exact bytes are preserved; absent player metadata cannot imply a V5 baseline.
    let mut legacy = admission
        .outputs
        .get(&viewer)
        .context("viewer outputs")?
        .iter()
        .find_map(|o| match o {
            ObserverOutput::Event(e @ ZoneEvent::EntitySpawn { entity, .. }) if *entity == npc => {
                Some(e.clone())
            },
            _ => None,
        })
        .context("NPC domain identity")?;
    if let ZoneEvent::EntitySpawn {
        combat: Some(c), ..
    } = &mut legacy
    {
        c.phase3 = None;
    }
    if let ZoneEvent::EntitySpawn {
        entity,
        kind,
        name,
        pos,
        generation,
        combat,
        ..
    } = &legacy
    {
        let previous = spawn_to_pb(*entity, *kind, name, *pos, *generation, combat.as_ref());
        let mapped = world_event_to_pb(&legacy, admission.server_time_ms)
            .into_iter()
            .next()
            .context("legacy spawn mapping")?;
        assert_eq!(mapped.encode_to_vec(), previous.encode_to_vec());
        let decoded = pb::WorldEvent::decode(mapped.encode_to_vec().as_slice())?;
        assert!(matches!(decoded.event, Some(Event::Spawn(s)) if s.state_tick == 0));
    }
    Ok(())
}
