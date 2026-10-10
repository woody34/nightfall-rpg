//! Additive Phase 3 tag, timer-width and hydration contracts, independent of actor rollout.

use nightfall_api::interface::grpc::pb;
use prost::Message;

#[test]
fn new_intents_keep_reserved_numbers() -> Result<(), prost::DecodeError> {
    use pb::client_message::Intent;
    for (intent, tag) in [
        (Intent::CastSkill(pb::CastSkillRequest::default()), 130_u8),
        (Intent::CancelCast(pb::CancelCastRequest::default()), 138),
        (Intent::AcceptResurrect(pb::AcceptResurrectRequest::default()), 146),
    ] {
        let message = pb::ClientMessage {
            seq: 0,
            intent: Some(intent),
        };
        let bytes = message.encode_to_vec();
        assert!(bytes.starts_with(&[tag, 1]));
        assert_eq!(pb::ClientMessage::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn new_events_keep_reserved_numbers() -> Result<(), prost::DecodeError> {
    use pb::world_event::Event;
    for (event, key) in [
        (Event::SkillCastStarted(pb::SkillCastStarted::default()), vec![114]),
        (Event::SkillCastResult(pb::SkillCastResult::default()), vec![122]),
        (Event::SkillCastCancelled(pb::SkillCastCancelled::default()), vec![130, 1]),
        (Event::EffectApplied(pb::EffectApplied::default()), vec![138, 1]),
        (Event::EffectRemoved(pb::EffectRemoved::default()), vec![146, 1]),
        (Event::SkillReuse(pb::SkillReuse::default()), vec![154, 1]),
        (Event::SkillLearned(pb::SkillLearned::default()), vec![162, 1]),
        (Event::PvpStateChanged(pb::PvpStateChanged::default()), vec![170, 1]),
        (Event::ResurrectOffer(pb::ResurrectOffer::default()), vec![178, 1]),
        (Event::SpGained(pb::SpGained::default()), vec![186, 1]),
        (Event::SkillStateSync(pb::SkillStateSync::default()), vec![194, 1]),
        (Event::VitalsChanged(pb::VitalsChanged::default()), vec![202, 1]),
    ] {
        let message = pb::WorldEvent { event: Some(event) };
        let bytes = message.encode_to_vec();
        assert!(bytes.starts_with(&key));
        assert_eq!(pb::WorldEvent::decode(bytes.as_slice())?, message);
    }
    Ok(())
}

#[test]
fn baseline_timers_and_optional_hidden_references_survive_wire() -> Result<(), prost::DecodeError> {
    let started = pb::SkillCastStarted {
        caster: "known".into(),
        caster_incarnation: 7,
        cast_id: u64::MAX,
        tick: u64::MAX,
        complete_tick: u64::MAX,
        launch_tick: u64::MAX,
        start_tick: u64::MAX,
        baseline: true,
        baseline_tick: u64::MAX,
        ..Default::default()
    };
    assert!(started.target.is_none() && started.target_incarnation.is_none());
    assert_eq!(pb::SkillCastStarted::decode(started.encode_to_vec().as_slice())?, started);
    let vitals = pb::VitalsChanged {
        entity: "known".into(),
        hp: 65,
        max_hp: 100,
        tick: u64::MAX,
        incarnation: 8,
        baseline: true,
        baseline_tick: u64::MAX,
    };
    assert_eq!(pb::VitalsChanged::decode(vitals.encode_to_vec().as_slice())?, vitals);
    let sync = pb::SkillStateSync {
        flag_until_tick: u64::MAX,
        reuse: vec![pb::SkillReuseState {
            skill_id: 1,
            ready_tick: u64::MAX,
        }],
        baseline: true,
        baseline_tick: u64::MAX,
        ..Default::default()
    };
    assert_eq!(pb::SkillStateSync::decode(sync.encode_to_vec().as_slice())?, sync);
    Ok(())
}

#[test]
fn default_additions_preserve_legacy_bytes() -> Result<(), prost::DecodeError> {
    let legacy_catalogue_entry = [8, 3, 18, 1, b'k', 26, 1, b'n', 32, 9, 40, 1, 58, 1, b'r'];
    let entry = pb::SkillCatalogEntry::decode(legacy_catalogue_entry.as_slice())?;
    assert!(entry.mp_consume_start.is_empty());
    assert!(entry.mp_consume_end.is_empty());
    assert!(entry.hp_consume.is_empty());
    assert!(entry.reuse_ms.is_empty());
    assert!(entry.hit_time_ms.is_empty());
    assert_eq!(entry.encode_to_vec(), legacy_catalogue_entry);
    let legacy_attack = [10, 1, b'a', 18, 1, b't', 24, 3, 32, 2, 40, 5, 48, 8, 56, 1];
    let attack = pb::AttackResult::decode(legacy_attack.as_slice())?;
    assert_eq!(attack.cp_damage, 0);
    assert_eq!(attack.encode_to_vec(), legacy_attack);
    let legacy_stats = [10, 1, b'p', 16, 3, 24, 5, 32, 7];
    let stats = pb::StatsChanged::decode(legacy_stats.as_slice())?;
    assert!(!stats.baseline);
    assert_eq!(stats.encode_to_vec(), legacy_stats);
    let legacy_selection = [10, 1, b'p', 18, 1, b't'];
    let selection = pb::TargetChanged::decode(legacy_selection.as_slice())?;
    assert_eq!(selection.tick, 0);
    assert_eq!(selection.encode_to_vec(), legacy_selection);
    Ok(())
}

#[test]
fn catalogue_rank_fields_use_additive_tags() -> Result<(), prost::DecodeError> {
    let entry = pb::SkillCatalogEntry {
        mp_consume_start: vec![0, u32::MAX],
        mp_consume_end: vec![0, u32::MAX],
        hp_consume: vec![0, u32::MAX],
        reuse_ms: vec![0, u32::MAX],
        hit_time_ms: vec![0, u32::MAX],
        ..Default::default()
    };
    // Packed uint32: new field keys 8..12, length 6, explicit zero then uint32::MAX.
    let bytes: Vec<u8> = [66, 74, 82, 90, 98]
        .into_iter()
        .flat_map(|key| [key, 6, 0, 255, 255, 255, 255, 15])
        .collect();
    assert_eq!(entry.encode_to_vec(), bytes);
    assert_eq!(pb::SkillCatalogEntry::decode(bytes.as_slice())?, entry);
    Ok(())
}

#[test]
fn selection_tick_distinguishes_hydration_stale_delta_and_later_selection(
) -> Result<(), prost::DecodeError> {
    // A consumer oracle for the shared baseline protocol, exercised AFTER wire decoding.
    // Client owner must apply these vectors to the actual client projection on integration.
    let identity_baseline = 100;
    let mut selected = String::new();
    for (tick, baseline, target, accepted) in [
        (100, true, "hydrated", true),
        (99, false, "stale", false),
        (100, false, "same-tick-stale", false),
        (101, false, "later", true),
    ] {
        let selection = pb::TargetChanged {
            entity: "owner".into(),
            target: target.into(),
            tick,
            baseline,
            baseline_tick: if baseline { identity_baseline } else { 0 },
            ..Default::default()
        };
        let selection = pb::TargetChanged::decode(selection.encode_to_vec().as_slice())?;
        let accepts = if selection.baseline {
            selection.tick == identity_baseline && selection.baseline_tick == identity_baseline
        } else {
            selection.tick > identity_baseline
        };
        assert_eq!(accepts, accepted);
        if accepts {
            selected = selection.target;
        }
        assert_eq!(selected, if tick == 101 { "later" } else { "hydrated" });
    }
    Ok(())
}

#[test]
fn skill_resolution_keeps_roll_absorption_and_removal_separate() -> Result<(), prost::DecodeError> {
    let target = pb::SkillTargetResult {
        target: "known".into(),
        target_incarnation: 4,
        tick: u64::MAX,
        resolution_kind: pb::SkillResolutionKind::Magical.into(),
        rolled: 100,
        cp_absorbed: 70,
        hp_removed: 20,
        heal: 0,
        outcome: pb::SkillOutcome::Half.into(),
        target_hp_after: 0,
        applied_effect_ids: vec![42],
    };
    assert_eq!(pb::SkillTargetResult::decode(target.encode_to_vec().as_slice())?, target);
    let reasons = [
        pb::RejectReason::SkillUnknown,
        pb::RejectReason::SkillNotLearned,
        pb::RejectReason::SkillCooldown,
        pb::RejectReason::InsufficientMp,
        pb::RejectReason::InsufficientHp,
        pb::RejectReason::AlreadyCasting,
        pb::RejectReason::Incapacitated,
        pb::RejectReason::Silenced,
        pb::RejectReason::InvalidTargetType,
        pb::RejectReason::PeaceZone,
        pb::RejectReason::WeaponCondition,
        pb::RejectReason::NoResurrectOffer,
    ];
    for (reason, number) in reasons.into_iter().zip(13..=24) {
        assert_eq!(i32::from(reason), number);
    }
    Ok(())
}
