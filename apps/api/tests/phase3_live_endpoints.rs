//! Real gRPC/WebSocket endpoints backed by the experimental durable actor.
#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
use api::application::checkpoint::CheckpointService;
use api::application::ports::{
    learn_skill_fingerprint, CharacterRepository, LearnReceiptLookup, LearnReceiptRepository,
};
use api::application::replay_log::{start_epoch, DurableTickGate, GateConfig, NoReplayMetrics};
use api::application::zone_actor::{IntervalTicks, ZoneActor, ZoneTelemetry};
use api::domain::character_progression::ClassState;
use api::domain::class::ClassId;
use api::domain::zone::*;
use api::domain::{AccountId, Character, CharacterName, DomainEvent, Position, Race};
use api::infrastructure::class_data::{load_classes, ClassSource};
use api::infrastructure::eventlog::InMemoryEventLog;
use api::infrastructure::memory::{InMemoryCharacterRepository, InMemorySessionAudit};
use api::infrastructure::rules_data::{load_rules, RulesSource};
use api::infrastructure::skill_data::{load_phase3_rules, Phase3DataSource};
use api::infrastructure::telemetry::Metrics;
use api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event,
};
use common::ws::{self, Player};
use common::TestApp;
use nightfall_api as api;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tonic::Code;
use uuid::Uuid;

async fn fixture() -> (TestApp, Arc<InMemoryCharacterRepository>, Character) {
    let classes = load_classes(&ClassSource::embedded()).unwrap().registry;
    let rules = load_phase3_rules(&Phase3DataSource::embedded(), &[]).unwrap();
    let repo =
        Arc::new(InMemoryCharacterRepository::default().with_phase3_rules(Arc::new(rules.clone())));
    let stat_rules = load_rules(&RulesSource::embedded()).unwrap().rules;
    let mut character = Character::create(
        AccountId::from_uuid(common::account()),
        CharacterName::new("WireMage").unwrap(),
        Race::Human,
    );
    character.class_state = ClassState::new(ClassId(10));
    character.stats = classes.get(ClassId(10)).unwrap().base_stats;
    character.level = 10;
    character.xp = stat_rules.xp_to_level(10).unwrap();
    character.class_state.sp = 5000;
    character.position = Position { x: 126.0, y: 128.0 };
    repo.insert_for_test(character.clone());
    let state = ZoneState::new_phase3(
        ZoneSeed {
            zone: ZoneId(89),
            epoch: 1,
        },
        ZoneBounds::new(Vec2Fixed::from_tiles(0, 0), Vec2Fixed::from_tiles(256, 256)).unwrap(),
        0,
        stat_rules,
        classes,
        rules,
    )
    .unwrap();
    let log = Arc::new(InMemoryEventLog::default());
    let started = start_epoch(log.as_ref(), &state.snapshot()).await.unwrap();
    let gate = DurableTickGate::new(
        &started,
        log,
        Arc::new(NoReplayMetrics),
        GateConfig::default(),
        CancellationToken::new(),
    );
    let metrics = Metrics::detached();
    let consumer = metrics.consumer(&state.snapshot());
    let zone =
        ZoneActor::spawn_gated_with_telemetry(state, IntervalTicks::new(), gate, Some(consumer));
    zone.checkpoints.install(
        CheckpointService::new(
            repo.clone(),
            Arc::new(InMemorySessionAudit::default()),
            Arc::new(metrics.clone()),
        )
        .with_learning(repo.clone()),
    );
    let app = TestApp::spawn_experimental_phase3(repo.clone(), zone, metrics).await;
    (app, repo, character)
}
fn player(c: &Character) -> Player {
    Player {
        account: c.account_id.as_uuid(),
        character: c.id,
        name: c.name.to_string(),
    }
}
fn request(c: &Character, id: u32, rank: u32) -> pb::LearnSkillRequest {
    pb::LearnSkillRequest {
        character_id: c.id.to_string(),
        skill_id: id,
        skill_level: rank,
        idempotency_key: Uuid::now_v7().to_string(),
    }
}
async fn joined(app: &TestApp, c: &Character) -> ws::Ws {
    let mut socket = ws::join(app, &player(c)).await;
    socket.until(|m| matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::SkillStateSync(sync)) if sync.entity==c.id.to_string()))).await;
    socket
}
fn message(seq: u32, intent: Intent) -> pb::ClientMessage {
    pb::ClientMessage {
        seq,
        intent: Some(intent),
    }
}
async fn response(socket: &mut ws::Ws, seq: u32) -> pb::ServerMessage {
    socket
        .until(|m| match &m.payload {
            Some(Payload::Ack(a)) => a.seq == seq,
            Some(Payload::Rejected(r)) => r.seq == seq,
            _ => false,
        })
        .await
        .pop()
        .unwrap()
}

fn assert_private_owner_events_absent(messages: &[pb::ServerMessage], owner: &str) {
    for message in messages {
        if let Some(Payload::Event(world)) = &message.payload {
            match &world.event {
                Some(Event::SkillLearned(e)) => assert_ne!(e.entity, owner),
                Some(Event::SkillReuse(e)) => assert_ne!(e.entity, owner),
                Some(Event::SkillStateSync(e)) => assert_ne!(e.entity, owner),
                Some(Event::StatsChanged(e)) => assert_ne!(e.entity, owner),
                Some(Event::SpGained(e)) => assert_ne!(e.entity, owner),
                Some(Event::TargetChanged(e)) => assert_ne!(e.entity, owner),
                Some(Event::ResurrectOffer(e)) => assert_ne!(e.target, owner),
                _ => {},
            }
        }
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One socket lifecycle verifies purchase through offline receipt replay.
async fn real_learning_catalogue_cast_cancel_hydration_and_offline_frozen_reply() {
    let (mut app, repo, c) = fixture().await;
    let mut socket = joined(&app, &c).await;
    let catalogue = app
        .grpc
        .get_skill_catalog(pb::GetSkillCatalogRequest {})
        .await
        .unwrap()
        .into_inner();
    let entry = catalogue
        .skills
        .iter()
        .find(|s| s.skill_id == 1040)
        .unwrap();
    assert!(entry.effect_implemented);
    assert!(!entry.mp_consume_start.is_empty());
    let next = app
        .grpc
        .list_learnable_skills(pb::ListLearnableSkillsRequest {
            character_id: c.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    let option = next
        .skills
        .iter()
        .find(|s| {
            s.skill
                .as_ref()
                .is_some_and(|s| s.skill_id == 1040 && s.skill_level == 1)
        })
        .unwrap();
    assert!(option.eligible);
    assert!(option.affordable);
    assert!(option.trainer_required);
    let req = request(&c, 1040, 1);
    let learned = app
        .grpc
        .learn_skill(req.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(learned.sp_spent, option.skill.as_ref().unwrap().sp_cost);
    assert_eq!(learned.sp_remaining, 5000u64.checked_sub(learned.sp_spent).unwrap());
    assert_eq!(
        repo.load_for_admission(c.id)
            .await
            .unwrap()
            .unwrap()
            .class_state
            .sp,
        learned.sp_remaining
    );
    let owner=socket.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::SkillLearned(s)) if s.skill_id==1040))).await;
    assert!(!owner.is_empty());
    assert_eq!(
        app.grpc
            .learn_skill(req.clone())
            .await
            .unwrap()
            .into_inner(),
        learned
    );
    socket
        .send(&message(
            1,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 1040,
                target: Some(c.id.to_string()),
            }),
        ))
        .await;
    assert!(matches!(response(&mut socket, 1).await.payload, Some(Payload::Ack(_))));
    let start=socket.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::SkillCastStarted(_))))).await;
    let cast = start
        .iter()
        .find_map(|m| match &m.payload {
            Some(Payload::Event(w)) => match &w.event {
                Some(Event::SkillCastStarted(s)) => Some(s.cast_id),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    socket
        .send(&message(
            2,
            Intent::CancelCast(pb::CancelCastRequest {
                cast_id: Some(cast),
            }),
        ))
        .await;
    assert!(matches!(response(&mut socket, 2).await.payload, Some(Payload::Ack(_))));
    socket.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::SkillCastCancelled(_))))).await;
    socket
        .send(&message(
            3,
            Intent::AcceptResurrect(pb::AcceptResurrectRequest {
                offer_id: 1,
                accept: true,
            }),
        ))
        .await;
    assert!(
        matches!(response(&mut socket,3).await.payload,Some(Payload::Rejected(r)) if r.reason==i32::from(pb::RejectReason::NoResurrectOffer))
    );
    // Replacement bootstraps learned ranks/reuse without charging or another purchase.
    let mut replacement = joined(&app, &c).await;
    assert_eq!(socket.closed(ws::WAIT).await, Some(4409));
    replacement.socket.close(None).await.unwrap();
    tokio::time::timeout(ws::WAIT, async {
        loop {
            if app.zone.snapshot().await.unwrap().entities.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let saved = repo.load_for_admission(c.id).await.unwrap().unwrap();
    let mut ledger = saved.class_state.clone();
    ledger.sp = 777;
    repo.checkpoint(
        &api::application::CharacterCheckpoint {
            character_id: c.id,
            revision_seen: saved.revision,
            level: saved.level,
            xp: saved.xp,
            hp: saved.hp.unwrap(),
            mp: saved.mp.unwrap(),
            alive: true,
            position: saved.position,
            idempotency: ("save_checkpoint".into(), api::application::IdempotencyKey::new()),
            class_state: Some(ledger),
            phase3: saved.phase3,
        },
        &[],
    )
    .await
    .unwrap();
    assert_eq!(app.grpc.learn_skill(req).await.unwrap().into_inner(), learned);
    assert_eq!(
        repo.staged_events()
            .iter()
            .filter(|e| matches!(e, DomainEvent::CharacterSkillLearned { .. }))
            .count(),
        1
    );
    let metrics = app.metrics.render().unwrap();
    let purchases: Vec<_> = metrics
        .lines()
        .filter(|line| line.starts_with("nightfall_skills_learned_total"))
        .collect();
    assert_eq!(purchases.len(), 1);
    assert!(purchases[0].ends_with(" 1"), "{}", purchases[0]);
    assert!(metrics
        .lines()
        .any(|l| l.starts_with("nightfall_skill_casts_total{")
            && l.contains("outcome=\"started\"")
            && l.ends_with(" 1")));
    assert!(!metrics.contains(&c.id.to_string()));
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One live session checks all ingress refusals and remains usable.
async fn real_endpoint_refusals_spend_nothing_and_preserve_ws_seq() {
    let (mut app, repo, c) = fixture().await;
    let req = request(&c, 1040, 1);
    assert_eq!(
        app.anon_grpc()
            .learn_skill(req.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.grpc.learn_skill(req.clone()).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        app.game_as(Uuid::now_v7())
            .learn_skill(req.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    let mut socket = joined(&app, &c).await;
    let mut invalid = req.clone();
    invalid.skill_id = 0;
    assert_eq!(app.grpc.learn_skill(invalid).await.unwrap_err().code(), Code::InvalidArgument);
    let wrong = request(&c, 1040, 2);
    assert_eq!(app.grpc.learn_skill(wrong).await.unwrap_err().code(), Code::FailedPrecondition);
    let before = repo.load_for_admission(c.id).await.unwrap().unwrap();
    for (seq, intent) in [
        (
            1,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 0,
                target: None,
            }),
        ),
        (
            2,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 1040,
                target: Some("bad".into()),
            }),
        ),
        (3, Intent::CancelCast(pb::CancelCastRequest { cast_id: Some(0) })),
        (
            4,
            Intent::AcceptResurrect(pb::AcceptResurrectRequest {
                offer_id: 0,
                accept: true,
            }),
        ),
    ] {
        socket.send(&message(seq, intent)).await;
        assert!(
            matches!(response(&mut socket,seq).await.payload,Some(Payload::Rejected(r)) if r.reason==i32::from(pb::RejectReason::Invalid))
        );
    }
    socket
        .send(&message(5, Intent::CancelCast(pb::CancelCastRequest { cast_id: None })))
        .await;
    assert!(matches!(response(&mut socket, 5).await.payload, Some(Payload::Ack(_))));
    for (seq, intent) in [
        (
            6,
            Intent::AcceptResurrect(pb::AcceptResurrectRequest {
                offer_id: u64::MAX,
                accept: true,
            }),
        ),
        (7, Intent::Attack(pb::AttackRequest {})),
        (8, Intent::Respawn(pb::RespawnRequest {})),
    ] {
        socket.send(&message(seq, intent)).await;
        let reason = match seq {
            6 => pb::RejectReason::NoResurrectOffer,
            7 => pb::RejectReason::InvalidTargetType,
            // Existing wire mapping groups domain NotDead with Invalid.
            _ => pb::RejectReason::Invalid,
        };
        assert!(matches!(response(&mut socket, seq).await.payload, Some(Payload::Rejected(r))
            if r.reason == i32::from(reason)));
    }
    socket
        .send(&message(
            9,
            Intent::CancelCast(pb::CancelCastRequest {
                cast_id: Some(u64::MAX),
            }),
        ))
        .await;
    assert!(matches!(response(&mut socket, 9).await.payload, Some(Payload::Ack(_))));
    let after = repo.load_for_admission(c.id).await.unwrap().unwrap();
    assert_eq!(after.class_state, before.class_state);
    assert_eq!(
        repo.learn_receipt_lookup(
            c.account_id,
            &api::application::IdempotencyKey::parse(&req.idempotency_key).unwrap(),
            &learn_skill_fingerprint(
                c.id,
                api::domain::zone::skill::SkillRef {
                    id: api::domain::zone::skill::SkillId(1040),
                    rank: 1
                }
            )
        )
        .await
        .unwrap(),
        LearnReceiptLookup::Unknown
    );
    assert!(!repo
        .staged_events()
        .iter()
        .any(|e| matches!(e, DomainEvent::CharacterSkillLearned { .. })));
}

#[tokio::test]
async fn effect_reconnect_hydration_is_after_identity_and_owner_state_stays_private() {
    let (mut app, repo, c) = fixture().await;
    let mut owner = joined(&app, &c).await;
    let mut other = Character::create(
        AccountId::from_uuid(Uuid::now_v7()),
        CharacterName::new("WireObserver").unwrap(),
        Race::Human,
    );
    other.position = c.position;
    repo.insert_for_test(other.clone());
    let mut observer = joined(&app, &other).await;
    app.grpc.learn_skill(request(&c, 1040, 1)).await.unwrap();
    owner
        .send(&message(
            1,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 1040,
                target: Some(c.id.to_string()),
            }),
        ))
        .await;
    assert!(matches!(response(&mut owner, 1).await.payload, Some(Payload::Ack(_))));
    owner.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::EffectApplied(e)) if e.skill_id==1040&&!e.baseline))).await;
    let public_messages=observer.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::EffectApplied(e)) if e.skill_id==1040))).await;
    assert_private_owner_events_absent(&public_messages, &c.id.to_string());
    let applied_count = app
        .metrics
        .render()
        .unwrap()
        .lines()
        .filter(|line| {
            line.starts_with("nightfall_skill_effects_total{")
                && line.contains("change=\"applied\"")
                && line.ends_with(" 1")
        })
        .count();
    assert!(applied_count > 0);
    let mut replacement = ws::join(&app, &player(&c)).await;
    let baseline=replacement.until(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::EffectApplied(e)) if e.skill_id==1040&&e.baseline))).await;
    let last_identity=baseline.iter().rposition(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::Spawn(_))))).unwrap();
    let effect=baseline.iter().position(|m|matches!(&m.payload,Some(Payload::Event(w)) if matches!(&w.event,Some(Event::EffectApplied(e)) if e.skill_id==1040&&e.baseline))).unwrap();
    assert!(effect > last_identity);
    let sync = baseline
        .iter()
        .find_map(|m| match &m.payload {
            Some(Payload::Event(w)) => match &w.event {
                Some(Event::SkillStateSync(s)) => Some(s),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert!(sync.baseline);
    assert!(sync.learned.iter().any(|s| s.skill_id == 1040));
    for message in &baseline {
        if let Some(Payload::Event(w)) = &message.payload {
            if let Some(Event::Spawn(s)) = &w.event {
                assert!(s.active_effects.is_empty());
                assert!(s.pending_cast.is_none());
                assert!(s.pending_swing.is_none());
            }
        }
    }
    observer
        .send(&message(1, Intent::CancelCast(pb::CancelCastRequest { cast_id: None })))
        .await;
    let after_replacement = observer
        .until(|m| matches!(&m.payload,Some(Payload::Ack(ack)) if ack.seq == 1))
        .await;
    assert_private_owner_events_absent(&after_replacement, &c.id.to_string());
    let after = app
        .metrics
        .render()
        .unwrap()
        .lines()
        .filter(|line| {
            line.starts_with("nightfall_skill_effects_total{")
                && line.contains("change=\"applied\"")
                && line.ends_with(" 1")
        })
        .count();
    assert_eq!(after, applied_count);
    assert_eq!(owner.closed(ws::WAIT).await, Some(4409));
}
