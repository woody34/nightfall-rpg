//! Real domain purchases and exact replay; actor success remains fenced until receipts.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm
)]

use nightfall_api::application::replay_log::{decode_snapshot, encode_snapshot, AppliedTickRecord};
use nightfall_api::domain::character_progression::{
    CharacterAppearance, CharacterIdentity, ClassState,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::zone::skill::{SkillId, SkillRef};
use nightfall_api::domain::zone::*;
use nightfall_api::domain::{AccountId, Race};
use nightfall_api::infrastructure::{
    class_data::{load_classes, ClassSource},
    rules_data::{load_rules, RulesSource},
    skill_data::{load_phase3_rules, Phase3DataSource},
};
use nightfall_api::interface::{grpc::pb, zone_mapping::observer_output_to_pb};
use uuid::Uuid;

fn player() -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(1))
}
fn account() -> AccountId {
    AccountId::from_uuid(Uuid::from_u128(2))
}
fn observer() -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(3))
}
fn skill(id: u32, rank: u32) -> SkillRef {
    SkillRef {
        id: SkillId(id),
        rank,
    }
}

fn setup(class: u32, level: u32, sp: u64) -> ZoneState {
    setup_position(class, level, sp, Vec2Fixed::from_tiles(126, 128))
}
fn setup_position(class: u32, level: u32, sp: u64, pos: Vec2Fixed) -> ZoneState {
    let stat_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let mut load = PlayerLoad::fresh(if class == 0 {
        "human_fighter"
    } else {
        "human_mystic"
    });
    load.level = level;
    load.xp = stat_rules.xp_to_level(level).unwrap();
    load.hp = Some(1);
    load.mp = Some(1);
    let mut ledger = ClassState::new(ClassId(class));
    ledger.sp = sp;
    load.progression = Some(Box::new(PlayerProgression {
        identity: CharacterIdentity {
            account_id: account(),
            race: Race::Human,
            base_class_id: ClassId(class),
            appearance: CharacterAppearance::default(),
        },
        class_state: ledger,
        max_cp: 0,
    }));
    let mut state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(99),
            epoch: 7,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stat_rules,
        load_classes(&ClassSource::embedded()).unwrap().registry,
        load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap(),
    )
    .unwrap();
    let applied = state
        .run_tick(
            state.draft(
                vec![player(), observer()]
                    .into_iter()
                    .map(|entity| {
                        ZoneInput::system(ZoneCommand::SpawnPlayer {
                            entity,
                            name: if entity == player() {
                                "Learner"
                            } else {
                                "Observer"
                            }
                            .into(),
                            pos,
                            speed: Speed::DEFAULT,
                            generation: SessionGeneration(1),
                            load: Some(Box::new(load.clone())),
                        })
                    })
                    .collect(),
            ),
        )
        .unwrap();
    assert!(applied.dispositions.is_empty(), "{:?}", applied.dispositions);
    state
}
fn learn(id: u32, rank: u32, key: u128) -> ZoneInput {
    ZoneInput::session(
        player(),
        SessionGeneration(1),
        1,
        ZoneCommand::LearnSkill {
            entity: player(),
            account: account(),
            request_key: Uuid::from_u128(key),
            skill_id: SkillId(id),
            skill_level: rank,
        },
    )
}
fn combat(state: &ZoneState) -> CombatState {
    state
        .snapshot()
        .entities
        .iter()
        .find(|e| e.id == player())
        .unwrap()
        .combat
        .clone()
        .unwrap()
}
fn success(tick: &AppliedTick) -> &FrozenLearnResult {
    tick.events
        .iter()
        .find_map(|event| match event {
            ZoneEvent::SkillLearned {
                entity,
                account: owner,
                request_key,
                result,
                ..
            } => {
                assert_eq!(*entity, player());
                assert_eq!(*owner, account());
                assert!(!request_key.is_nil());
                Some(result.as_ref())
            },
            _ => None,
        })
        .unwrap()
}
fn restored(state: &ZoneState) -> ZoneState {
    ZoneState::from_snapshot(decode_snapshot(&encode_snapshot(&state.snapshot()).unwrap()).unwrap())
        .unwrap()
}

#[test]
fn live_purchase_query_passive_upgrade_owner_projection_and_exact_replay() {
    let mut zone = setup(0, 10, 2000);
    let before = zone.snapshot();
    let digest = zone.state_digest();
    assert_eq!(
        zone.validate_learn_skill(player(), account(), SessionGeneration(1), skill(3, 1)),
        Ok(())
    );
    assert_eq!(
        zone.validate_learn_skill(
            player(),
            account(),
            SessionGeneration(1_u64 << 32 | 1),
            skill(3, 1)
        ),
        Err(RejectReason::StaleSession)
    );
    assert_eq!(
        zone.validate_learn_skill(
            player(),
            AccountId::from_uuid(Uuid::from_u128(44)),
            SessionGeneration(1),
            skill(3, 1)
        ),
        Err(RejectReason::NotPermitted)
    );
    assert_eq!(zone.state_digest(), digest);
    assert_eq!(zone.snapshot(), before);
    let mut replay = restored(&zone);
    let initial_attack = combat(&zone).sheet.p_atk();
    for (id, rank, key) in [(3, 1, 10), (141, 1, 11), (141, 2, 12)] {
        let draft = zone.draft(vec![learn(id, rank, key)]);
        let tick = zone.run_tick(draft.clone()).unwrap();
        assert!(tick.dispositions.is_empty());
        assert_eq!(tick, replay.run_tick(draft).unwrap());
        let record = AppliedTickRecord::from_applied(ZoneId(99), &tick);
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
        let result = success(&tick);
        if id == 3 {
            assert_eq!((result.sp_spent, result.sp_remaining), (50, 1950));
        }
        assert_eq!(result.hp, combat(&zone).hp);
        assert_eq!(result.mp, combat(&zone).mp);
        assert_eq!(result.effective_stats.p_atk_q, combat(&zone).sheet.p_atk().raw());
        let wire: Vec<_> = tick.outputs[&player()]
            .iter()
            .flat_map(|o| observer_output_to_pb(o, tick.server_time_ms))
            .collect();
        assert!(wire.iter().any(|m| matches!(&m.payload, Some(pb::server_message::Payload::Event(e)) if matches!(&e.event, Some(pb::world_event::Event::SkillLearned(s)) if s.skill_id == id && s.skill_level == rank && s.idempotency_key == Uuid::from_u128(key).to_string()))));
        assert!(!tick
            .outputs
            .get(&observer())
            .into_iter()
            .flatten()
            .any(|e| matches!(
                e,
                ObserverOutput::Event(
                    ZoneEvent::SkillLearned { .. } | ZoneEvent::StatsChanged { .. }
                )
            )));
        assert_eq!(zone.snapshot().rng, before.rng);
        replay = restored(&zone);
        assert_eq!(zone.state_digest(), replay.state_digest());
    }
    assert!(combat(&zone).sheet.p_atk() > initial_attack);
    let snapshot = zone.snapshot();
    let phase = snapshot.phase3.as_ref().unwrap();
    let installs = &phase.entities[&player()].modifier_install_order;
    assert_eq!(
        installs
            .values()
            .filter(|s| matches!(s,RestoredModifierSource::Passive(s) if s.id == SkillId(141)))
            .count(),
        1
    );
    assert_eq!(
        phase
            .checkpoint(player(), Tick(zone.next_tick().0 - 1))
            .unwrap()
            .modifier_install_order
            .len(),
        installs.len()
    );
    let refusal = zone.run_tick(zone.draft(vec![learn(141, 2, 13)])).unwrap();
    assert_eq!(refusal.dispositions[0].reason, RejectReason::SkillWrongRank);
    assert!(!refusal
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::SkillLearned { .. })));
}

#[test]
fn purchase_guards_are_authoritative_and_never_debit_or_allocate() {
    let base = setup(0, 5, 49);
    let before = base.snapshot();
    for (id, rank, reason) in [
        (3, 1, RejectReason::InsufficientSp),
        (3, 2, RejectReason::SkillWrongRank),
        (u32::MAX, 1, RejectReason::SkillUnknown),
        (1177, 1, RejectReason::SkillNotLearnable),
        (142, 1, RejectReason::SkillDeferred),
    ] {
        let mut zone = restored(&base);
        assert_eq!(
            zone.validate_learn_skill(player(), account(), SessionGeneration(1), skill(id, rank)),
            Err(reason)
        );
        let tick = zone
            .run_tick(zone.draft(vec![learn(id, rank, 20)]))
            .unwrap();
        assert_eq!(tick.dispositions[0].reason, reason);
        assert_eq!(combat(&zone), combat(&base));
        assert_eq!(zone.snapshot().phase3, before.phase3);
        assert_eq!(zone.snapshot().rng, before.rng);
        let record = AppliedTickRecord::from_applied(ZoneId(99), &tick);
        assert_eq!(AppliedTickRecord::decode(&record.encode()).unwrap(), record);
    }
    let mut zone = setup(0, 1, 2000);
    assert_eq!(
        zone.validate_learn_skill(player(), account(), SessionGeneration(1), skill(3, 1)),
        Err(RejectReason::SkillLevelRequired)
    );
    let mut input = learn(3, 1, 21);
    input.source = CommandSource::System;
    let tick = zone.run_tick(zone.draft(vec![input])).unwrap();
    assert_eq!(tick.dispositions[0].reason, RejectReason::NotPermitted);
    let mut input = learn(3, 1, 22);
    if let ZoneCommand::LearnSkill { account: owner, .. } = &mut input.command {
        *owner = AccountId::from_uuid(Uuid::from_u128(88));
    }
    let tick = zone.run_tick(zone.draft(vec![input])).unwrap();
    assert_eq!(tick.dispositions[0].reason, RejectReason::NotPermitted);
}

#[test]
fn final_result_includes_same_tick_regen_and_receipt_key_is_not_canonical_state() {
    let mut zone = setup(0, 10, 2000);
    let due = zone.snapshot().phase3.as_ref().unwrap().entities[&player()]
        .regen
        .next_at
        .unwrap();
    while zone.next_tick() < due {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let mut alternate = restored(&zone);
    let before = combat(&zone);
    let tick = zone.run_tick(zone.draft(vec![learn(141, 1, 31)])).unwrap();
    let other = alternate
        .run_tick(alternate.draft(vec![learn(141, 1, 32)]))
        .unwrap();
    let result = success(&tick);
    assert!(result.hp > before.hp);
    assert!(result.mp > before.mp);
    assert_eq!(result.hp, combat(&zone).hp);
    assert_eq!(result.mp, combat(&zone).mp);
    assert_eq!(zone.state_digest(), alternate.state_digest());
    assert_eq!(zone.snapshot(), alternate.snapshot());
    assert_ne!(
        AppliedTickRecord::from_applied(ZoneId(99), &tick).encode(),
        AppliedTickRecord::from_applied(ZoneId(99), &other).encode()
    );
}

#[test]
fn one_purchase_per_tick_and_lifecycle_do_not_erase_the_success_fact() {
    let mut zone = setup(0, 10, 2000);
    let tick = zone
        .run_tick(zone.draft(vec![
            learn(3, 1, 40),
            learn(3, 2, 41),
            ZoneInput::system(ZoneCommand::Despawn { entity: player() }),
        ]))
        .unwrap();
    assert_eq!(
        tick.dispositions
            .iter()
            .map(|d| d.reason)
            .collect::<Vec<_>>(),
        vec![RejectReason::InCombat, RejectReason::InCombat]
    );
    assert_eq!(success(&tick).sp_remaining, 1950);
    assert_eq!(
        tick.events
            .iter()
            .filter(|e| matches!(e, ZoneEvent::SkillLearned { .. }))
            .count(),
        1
    );
    restored(&zone);
}

#[test]
fn learned_passive_survives_real_departure_and_paused_readmission() {
    let mut zone = setup(0, 10, 2000);
    let learned = zone.run_tick(zone.draft(vec![learn(141, 1, 70)])).unwrap();
    let before = combat(&zone);
    let CombatRole::Player {
        class,
        xp,
        progression,
    } = before.role.clone()
    else {
        panic!("player");
    };
    let mut load = PlayerLoad::fresh(class);
    load.level = before.sheet.level();
    load.xp = xp;
    load.hp = Some(before.hp);
    load.mp = Some(before.mp);
    load.progression = progression;
    load.phase3 = Some(
        zone.snapshot()
            .phase3
            .as_ref()
            .unwrap()
            .checkpoint(player(), learned.tick)
            .unwrap(),
    );
    zone.run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::Despawn { entity: player() })]))
        .unwrap();
    for _ in 0..5 {
        zone.run_tick(zone.draft(vec![])).unwrap();
    }
    let rng = zone.snapshot().rng;
    let admitted = zone
        .run_tick(zone.draft(vec![ZoneInput::system(ZoneCommand::SpawnPlayer {
            entity: player(),
            name: "Learner".into(),
            pos: Vec2Fixed::from_tiles(126, 128),
            speed: Speed::DEFAULT,
            generation: SessionGeneration(2),
            load: Some(Box::new(load)),
        })]))
        .unwrap();
    assert!(admitted.dispositions.is_empty(), "{:?}", admitted.dispositions);
    assert_eq!(combat(&zone).sheet, before.sheet);
    assert_eq!(combat(&zone).hp, before.hp);
    assert_eq!(combat(&zone).mp, before.mp);
    assert_eq!(zone.snapshot().rng, rng);
    assert_eq!(
        zone.validate_learn_skill(player(), account(), SessionGeneration(2), skill(141, 2)),
        Ok(())
    );
    assert!(!admitted
        .events
        .iter()
        .any(|e| matches!(e, ZoneEvent::SkillLearned { .. })));
    restored(&zone);
}

#[test]
fn trainer_boundary_is_inclusive_and_read_query_does_not_move_or_mutate() {
    let base = setup(0, 5, 100);
    for (raw_x, expected) in [
        (129_000, Ok(())),
        (129_001, Err(RejectReason::ClassMasterTooFar)),
    ] {
        let mut zone = setup_position(
            0,
            5,
            100,
            Vec2Fixed::new(Fixed::from_raw(raw_x), Fixed::from_tiles(128)),
        );
        let before = zone.snapshot();
        assert_eq!(
            zone.validate_learn_skill(player(), account(), SessionGeneration(1), skill(3, 1)),
            expected
        );
        assert_eq!(zone.snapshot(), before);
        let tick = zone.run_tick(zone.draft(vec![learn(3, 1, 80)])).unwrap();
        match expected {
            Ok(()) => {
                assert!(tick.dispositions.is_empty());
                assert_eq!(success(&tick).sp_remaining, 50);
            },
            Err(reason) => {
                assert_eq!(tick.dispositions[0].reason, reason);
                assert_eq!(combat(&zone), combat(&base));
            },
        }
    }
}

#[test]
fn staged_cast_blocks_learning_but_prior_cancel_removes_the_engagement() {
    let mut zone = setup(10, 10, 2000);
    let cast = ZoneInput::session(
        player(),
        SessionGeneration(1),
        2,
        ZoneCommand::CastSkill {
            entity: player(),
            skill_id: SkillId(1177),
            target: Some(observer()),
        },
    );
    let cancel = ZoneInput::session(
        player(),
        SessionGeneration(1),
        3,
        ZoneCommand::CancelCast {
            entity: player(),
            cast_id: None,
        },
    );
    let tick = zone
        .run_tick(zone.draft(vec![cast.clone(), learn(1040, 1, 50), cancel.clone()]))
        .unwrap();
    assert_eq!(tick.dispositions[0].reason, RejectReason::InCombat);
    let tick = zone
        .run_tick(zone.draft(vec![cast, cancel, learn(1040, 1, 51)]))
        .unwrap();
    assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
    assert_eq!(success(&tick).skill, skill(1040, 1));
}

#[test]
fn command_order_learn_then_cast_uses_new_rank_and_freezes_paid_mp_and_reuse() {
    let mut fixture = setup(10, 10, 2000).snapshot();
    let combatant = fixture
        .entities
        .iter_mut()
        .find(|e| e.id == player())
        .unwrap()
        .combat
        .as_mut()
        .unwrap();
    combatant.mp = combatant.sheet.max_mp();
    let mut zone = ZoneState::from_snapshot(fixture).unwrap();
    let before = combat(&zone).mp;
    let mut replay = restored(&zone);
    let cast = ZoneInput::session(
        player(),
        SessionGeneration(1),
        2,
        ZoneCommand::CastSkill {
            entity: player(),
            skill_id: SkillId(1040),
            target: Some(player()),
        },
    );
    let draft = zone.draft(vec![learn(1040, 1, 90), cast]);
    let tick = zone.run_tick(draft.clone()).unwrap();
    assert_eq!(tick, replay.run_tick(draft).unwrap());
    assert!(tick.dispositions.is_empty(), "{:?}", tick.dispositions);
    let receipt = success(&tick).clone();
    assert_eq!(receipt.sp_spent, 470); // Pinned classSkillTree: Shield1 Human Mystic7.
    assert_eq!(receipt.mp, combat(&zone).mp);
    assert!(receipt.mp < before);
    let complete = tick
        .events
        .iter()
        .find_map(|e| match e {
            ZoneEvent::SkillCastStarted {
                skill: learned,
                complete_at,
                ..
            } => {
                assert_eq!(*learned, skill(1040, 1));
                Some(*complete_at)
            },
            _ => None,
        })
        .unwrap();
    assert!(zone.snapshot().phase3.as_ref().unwrap().entities[&player()]
        .reuse
        .contains_key(&SkillId(1040)));
    while zone.next_tick() <= complete {
        let draft = zone.draft(vec![]);
        let tick = zone.run_tick(draft.clone()).unwrap();
        assert_eq!(tick, replay.run_tick(draft).unwrap());
    }
    assert!(zone.snapshot().phase3.as_ref().unwrap().entities[&player()]
        .effects
        .entries()
        .values()
        .any(|effect| effect.skill == skill(1040, 1)));
    assert!(
        combat(&zone).mp < receipt.mp,
        "MP2 is paid at completion, after the frozen learning tick"
    );
    restored(&zone);
}

#[tokio::test]
async fn actor_cannot_release_raw_purchase_before_atomic_receipt_integration() {
    use nightfall_api::application::zone_actor::{manual_ticks, OpenGate, TickOutcome, ZoneActor};
    let state = setup(0, 10, 2000);
    let expected = state.next_tick();
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, OpenGate);
    let mut output = zone.subscribe();
    zone.send(learn(3, 1, 60)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(expected));
    zone.stopped().await;
    assert!(zone.persistence_failed());
    assert!(zone.snapshot().await.is_err());
    assert!(matches!(
        output.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn durable_gate_alone_cannot_log_or_release_purchase_without_receipt_integration() {
    use nightfall_api::application::replay_log::{
        start_epoch, DurableTickGate, GateConfig, NoReplayMetrics,
    };
    use nightfall_api::application::zone_actor::{manual_ticks, TickOutcome, ZoneActor};
    use nightfall_api::infrastructure::eventlog::InMemoryEventLog;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    let state = setup(0, 10, 2000);
    let expected = state.next_tick();
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let before = log.subjects();
    let gate = DurableTickGate::new(
        &started,
        log.clone(),
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let (ticks, driver) = manual_ticks();
    let zone = ZoneActor::spawn_gated(state, ticks, gate);
    let mut output = zone.subscribe();
    zone.send(learn(3, 1, 61)).unwrap();
    assert_eq!(driver.step().await.unwrap(), TickOutcome::Held(expected));
    zone.stopped().await;
    assert!(zone.persistence_failed());
    assert!(zone.snapshot().await.is_err());
    assert!(matches!(
        output.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(log.subjects(), before, "no durable purchase without its receipt path");
}
