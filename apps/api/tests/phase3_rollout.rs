//! Real endpoint coverage for the additive contract's fail-closed rollout stubs.
#![allow(missing_docs, clippy::unwrap_used)]

mod common;

use common::ws::{join, rejected, seed_player};
use common::TestApp;
use nightfall_api::interface::grpc::pb::{
    self, client_message::Intent, server_message::Payload, world_event::Event,
};
use std::time::Duration;
use tonic::Code;
use uuid::Uuid;

#[tokio::test]
async fn new_rpcs_require_authentication_and_refuse_unavailable_runtime_without_writes() {
    let app = TestApp::spawn().await;
    let player = seed_player(&app, "Learner", 10.0, 10.0);
    let mut character = app.characters.get_for_test(player.character).unwrap();
    character.class_state.sp = 123;
    app.characters.insert_for_test(character.clone());
    let request = pb::LearnSkillRequest {
        character_id: player.character.to_string(),
        skill_id: 3,
        skill_level: 1,
        idempotency_key: Uuid::now_v7().to_string(),
    };
    let before = app.characters.staged_events();
    let mut anonymous = app.anon_grpc();
    assert_eq!(
        anonymous
            .learn_skill(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        anonymous
            .list_learnable_skills(pb::ListLearnableSkillsRequest {
                character_id: request.character_id.clone()
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        anonymous
            .get_skill_catalog(pb::GetSkillCatalogRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    let mut authenticated = app.game_as(player.account);
    assert_eq!(
        authenticated
            .learn_skill(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(
        authenticated
            .learn_skill(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(
        authenticated
            .list_learnable_skills(pb::ListLearnableSkillsRequest {
                character_id: request.character_id
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(
        authenticated
            .get_skill_catalog(pb::GetSkillCatalogRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(app.characters.get_for_test(player.character).unwrap(), character);
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
}

#[tokio::test]
async fn new_ws_intents_reject_once_with_original_sequence_and_never_reach_actor() {
    let app = TestApp::spawn().await;
    let player = seed_player(&app, "Caster", 10.0, 10.0);
    let mut character = app.characters.get_for_test(player.character).unwrap();
    character.class_state.sp = 123;
    app.characters.insert_for_test(character.clone());
    let mut socket = join(&app, &player).await;
    socket
        .until(|message| {
            matches!(
                &message.payload,
                Some(Payload::Event(pb::WorldEvent {
                    event: Some(Event::StatsChanged(_))
                }))
            )
        })
        .await;
    let before = app.zone.snapshot().await.unwrap();
    let before_events = app.characters.staged_events();
    let mut applied = app.zone.subscribe();
    for (seq, intent) in [
        (
            41,
            Intent::CastSkill(pb::CastSkillRequest {
                skill_id: 3,
                target: None,
            }),
        ),
        (42, Intent::CancelCast(pb::CancelCastRequest { cast_id: Some(7) })),
        (
            43,
            Intent::AcceptResurrect(pb::AcceptResurrectRequest {
                offer_id: 8,
                accept: true,
            }),
        ),
    ] {
        socket
            .send(&pb::ClientMessage {
                seq,
                intent: Some(intent),
            })
            .await;
        let mut messages = socket
            .until(|message| rejected(message).is_some_and(|r| r.seq == seq))
            .await;
        messages.extend(socket.drain(Duration::from_millis(120)).await);
        assert_eq!(messages.len(), 1);
        let rejection = messages.first().and_then(rejected).unwrap();
        assert_eq!(rejection.seq, seq);
        assert_eq!(rejection.reason, i32::from(pb::RejectReason::NotYetImplemented));
        assert!(!rejection.detail.is_empty());
    }
    let after = app.zone.snapshot().await.unwrap();
    assert_eq!(after.entities, before.entities);
    assert_eq!(after.rng, before.rng);
    assert_eq!(app.characters.get_for_test(player.character).unwrap(), character);
    assert_eq!(app.characters.staged_events(), before_events);
    while let Ok(tick) = applied.try_recv() {
        assert!(tick.commands.is_empty());
        assert!(tick.events.is_empty());
    }
    assert!(app.bus.published().is_empty());
}
