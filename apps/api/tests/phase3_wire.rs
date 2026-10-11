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
    assert_eq!(entry.operate_type, pb::SkillOperateType::Unspecified as i32);
    assert!(entry.target_type.is_empty());
    assert!(entry.affect_scope.is_empty());
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

#[test]
fn catalogue_classification_fields_use_additive_tags() -> Result<(), prost::DecodeError> {
    let entry = pb::SkillCatalogEntry {
        operate_type: pb::SkillOperateType::Toggle as i32,
        target_type: vec!["SELF".into()],
        affect_scope: vec!["unset".into()],
        ..Default::default()
    };
    // Varint enum tag 13; repeated strings tags 14 and 15.
    let bytes = [
        104, 5, 114, 4, b'S', b'E', b'L', b'F', 122, 5, b'u', b'n', b's', b'e', b't',
    ];
    assert_eq!(entry.encode_to_vec(), bytes);
    assert_eq!(pb::SkillCatalogEntry::decode(bytes.as_slice())?, entry);
    // Future enum discriminants survive transport without becoming an active type.
    let unknown = pb::SkillCatalogEntry::decode([104, 127].as_slice())?;
    assert!(
        unknown.operate_type == 127
            && pb::SkillOperateType::try_from(unknown.operate_type).is_err()
    );
    assert_eq!(unknown.encode_to_vec(), [104, 127]);
    Ok(())
}

#[test]
fn swing_hydration_preserves_start_times_and_omits_hidden_endpoint_pair(
) -> Result<(), prost::DecodeError> {
    use nightfall_api::domain::zone::{effects::TargetBinding, EntityId, Swing, Tick};
    use nightfall_api::interface::zone_mapping::{phase3_swing_to_pb, swing_to_pb};
    let attacker = TargetBinding {
        entity: EntityId::from_uuid(uuid::Uuid::from_u128(1)),
        incarnation: 7,
    };
    let swing = Swing {
        target: EntityId::from_uuid(uuid::Uuid::from_u128(2)),
        target_incarnation: 9,
        start: Tick(81),
        impact: Tick(105),
        ready: Tick(u64::MAX),
    };
    for visible in [false, true] {
        for baseline in [None, Some(Tick(100))] {
            let projected = phase3_swing_to_pb(attacker, &swing, visible, baseline);
            let decoded = pb::AttackStarted::decode(projected.encode_to_vec().as_slice())?;
            assert_eq!(
                (decoded.tick, decoded.impact_tick, decoded.ready_tick),
                (81, 105, u64::MAX)
            );
            assert_eq!(decoded.attacker_incarnation, Some(7));
            assert_eq!(decoded.baseline, baseline.is_some());
            assert_eq!(decoded.baseline_tick, baseline.map_or(0, |t| t.0));
            assert_eq!(
                decoded.target,
                if visible {
                    swing.target.to_string()
                } else {
                    String::new()
                }
            );
            assert_eq!(decoded.target_incarnation, if visible { 9 } else { 0 });
        }
    }
    let legacy = swing_to_pb(attacker.entity, &swing);
    assert_eq!(legacy.attacker_incarnation, None);
    assert!(!legacy.baseline);
    assert_eq!(legacy.target, swing.target.to_string());
    Ok(())
}

#[test]
fn public_identity_is_reference_free_and_selection_bootstrap_can_explicitly_clear(
) -> Result<(), prost::DecodeError> {
    use nightfall_api::domain::zone::{effects::TargetBinding, EntityId, PvpState, Tick};
    use nightfall_api::interface::zone_mapping::{
        phase3_identity_to_pb, phase3_selection_to_pb, pvp_state_to_pb,
    };
    let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
    let mut identity = pb::EntitySpawn {
        entity_id: entity.to_string(),
        pending_swing: Some(pb::AttackStarted {
            target: "hidden".into(),
            ..Default::default()
        }),
        pending_cast: Some(pb::SkillCastStarted {
            target: Some("hidden".into()),
            ..Default::default()
        }),
        active_effects: vec![pb::ActiveEffect::default()],
        ..Default::default()
    };
    phase3_identity_to_pb(&mut identity, Some(Tick(u64::MAX)), 123);
    let identity = pb::EntitySpawn::decode(identity.encode_to_vec().as_slice())?;
    assert!(identity.pending_swing.is_none() && identity.pending_cast.is_none());
    assert!(identity.active_effects.is_empty());
    assert_eq!((identity.flag_until_tick, identity.karma), (u64::MAX, 123));
    let pvp = pvp_state_to_pb(
        TargetBinding {
            entity,
            incarnation: 7,
        },
        Tick(100),
        &PvpState {
            flag_until: Some(Tick(u64::MAX)),
            karma: 123,
            pk_count: u32::MAX,
            pvp_count: 456,
        },
        Some(Tick(100)),
    );
    let pvp = pb::PvpStateChanged::decode(pvp.encode_to_vec().as_slice())?;
    assert_eq!((pvp.karma, pvp.pk_count, pvp.pvp_count), (123, u32::MAX, 456));
    assert_eq!((pvp.incarnation, pvp.tick, pvp.baseline_tick), (7, 100, 100));
    assert!(pvp.baseline);
    for target in [
        None,
        Some(TargetBinding {
            entity,
            incarnation: 7,
        }),
    ] {
        let selection = phase3_selection_to_pb(entity, Tick(100), target, Some(Tick(100)));
        let selection = pb::TargetChanged::decode(selection.encode_to_vec().as_slice())?;
        assert!(selection.baseline);
        assert_eq!((selection.tick, selection.baseline_tick), (100, 100));
        assert_eq!(selection.target_incarnation, target.map(|t| t.incarnation));
        assert_eq!(selection.target.is_empty(), target.is_none());
    }
    Ok(())
}

#[test]
fn offer_baseline_uses_additive_tags_and_preserves_prior_bytes() -> Result<(), prost::DecodeError> {
    let prior = [10, 1, b'p', 16, 7, 24, 9, 64, 100, 72, 1, 80, 50];
    let mut offer = pb::ResurrectOffer::decode(prior.as_slice())?;
    assert!(!offer.baseline);
    assert_eq!(offer.baseline_tick, 0);
    assert_eq!(offer.encode_to_vec(), prior);
    offer.baseline = true;
    offer.baseline_tick = u64::MAX;
    let mut expected = prior.to_vec();
    expected.extend([88, 1, 96, 255, 255, 255, 255, 255, 255, 255, 255, 255, 1]);
    assert_eq!(offer.encode_to_vec(), expected);
    assert_eq!(pb::ResurrectOffer::decode(expected.as_slice())?, offer);
    Ok(())
}

#[test]
fn offer_hydration_requires_matching_v5_identity_and_only_marks_offered() -> anyhow::Result<()> {
    use anyhow::Context as _;
    use nightfall_api::domain::zone::{
        effects::{StatusFlags, TargetBinding},
        CombatView, CombatViewPhase3, EntityId, EntityKind, ObserverOutput, OfferId,
        ResurrectOfferState, SessionGeneration, Speed, Tick, Vec2Fixed, ZoneEvent,
    };
    use nightfall_api::interface::zone_mapping::encode_observer_outputs;
    let entity = EntityId::from_uuid(uuid::Uuid::from_u128(1));
    for (v5, same_entity, life, identity_tick, offer_tick, state, expected) in [
        (true, true, 7, 100, 100, ResurrectOfferState::Offered, true),
        (false, true, 7, 100, 100, ResurrectOfferState::Offered, false),
        (true, false, 7, 100, 100, ResurrectOfferState::Offered, false),
        (true, true, 8, 100, 100, ResurrectOfferState::Offered, false),
        (true, true, 7, 100, 101, ResurrectOfferState::Offered, false),
        (true, true, 7, 100, 99, ResurrectOfferState::Offered, false),
        (true, true, 7, 100, 100, ResurrectOfferState::Accepted, false),
        (true, true, 7, 100, 100, ResurrectOfferState::Declined, false),
        (true, true, 7, 100, 100, ResurrectOfferState::Expired, false),
    ] {
        let identity = ZoneEvent::EntitySpawn {
            identity: None,
            tick: Tick(identity_tick),
            entity: if same_entity {
                entity
            } else {
                EntityId::from_uuid(uuid::Uuid::from_u128(2))
            },
            kind: EntityKind::Player,
            name: "owner".into(),
            pos: Vec2Fixed::from_tiles(0, 0),
            dest: None,
            speed: Speed::DEFAULT,
            generation: SessionGeneration(2),
            combat: Some(CombatView {
                phase3: v5.then_some(CombatViewPhase3 {
                    status_flags: StatusFlags::default(),
                    flag_until: None,
                    karma: 0,
                }),
                template: None,
                incarnation: life,
                dead: true,
                attackable: true,
                hp: 0,
                max_hp: 100,
                level: 1,
                swing: None,
            }),
        };
        let offer = ZoneEvent::ResurrectOffer {
            tick: Tick(offer_tick),
            target: TargetBinding {
                entity,
                incarnation: 7,
            },
            offer_id: OfferId(9),
            caster: None,
            restore_percent_q: 0,
            recoverable_xp: 0,
            expires_at: Tick(u64::MAX),
            state,
        };
        let outputs = [
            ObserverOutput::Event(identity),
            ObserverOutput::Event(offer),
        ];
        for (batch, baseline) in [(&outputs[..], expected), (&outputs[1..], false)] {
            let decoded = encode_observer_outputs(batch, 0)
                .iter()
                .map(|bytes| pb::ServerMessage::decode(bytes.as_ref()))
                .collect::<Result<Vec<_>, _>>()?;
            let offer = decoded
                .iter()
                .find_map(|m| match &m.payload {
                    Some(pb::server_message::Payload::Event(pb::WorldEvent {
                        event: Some(pb::world_event::Event::ResurrectOffer(o)),
                    })) => Some(o),
                    _ => None,
                })
                .context("mapped offer frame")?;
            assert_eq!(offer.baseline, baseline);
            assert_eq!(offer.baseline_tick, if baseline { identity_tick } else { 0 });
            assert_eq!(offer.tick, offer_tick);
            assert_eq!(offer.expiry_tick, u64::MAX);
            assert!(offer.caster.is_none() && offer.caster_incarnation.is_none());
        }
    }
    Ok(())
}
