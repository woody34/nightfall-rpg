#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]
mod common;
use common::{account, other_account, TestApp};
use nightfall_api::application::{CharacterCheckpoint, CharacterRepository, IdempotencyKey};
use nightfall_api::domain::character_progression::{
    FrozenTransferResult, SuccessfulTransferReceipt,
};
use nightfall_api::domain::class::ClassId;
use nightfall_api::domain::{AccountId, Character, CharacterName, Position, Race};
use nightfall_api::interface::grpc::pb;
use tonic::Code;
use uuid::Uuid;

fn fixture() -> Character {
    Character::create(
        AccountId::from_uuid(account()),
        CharacterName::new("Hero").unwrap(),
        Race::Human,
    )
}
fn frozen(c: &Character) -> FrozenTransferResult {
    FrozenTransferResult {
        character_id: c.id,
        identity: c.identity(),
        name: c.name.clone(),
        current_class_id: ClassId(1),
        level: 20,
        xp: 100,
        sp: 55,
        stats: c.stats,
        position_millitiles: [126_000, 126_000],
        hp: 100,
        mp: 40,
        cp: 20,
        max_hp: 100,
        max_mp: 40,
        max_cp: 20,
        token_tier_1_count: 0,
        token_tier_2_count: 1,
        granted_skill_keys: Vec::new(),
    }
}
fn change(id: String, key: Uuid, target: u32) -> pb::ChangeClassRequest {
    pb::ChangeClassRequest {
        character_id: id,
        target_class_id: target,
        idempotency_key: key.to_string(),
    }
}

#[tokio::test]
async fn transfer_endpoints_authenticate_validate_ownership_and_require_a_live_session() {
    let mut app = TestApp::spawn().await;
    let c = fixture();
    app.characters.insert_for_test(c.clone());
    let options = pb::TransferOptionsRequest {
        character_id: c.id.to_string(),
    };
    let key = Uuid::now_v7();
    let mutation = change(c.id.to_string(), key, 1);
    assert_eq!(
        app.anon_grpc()
            .transfer_options(options.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.anon_grpc()
            .change_class(mutation.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    assert_eq!(
        app.game_as(other_account())
            .transfer_options(options.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        app.game_as(other_account())
            .change_class(mutation.clone())
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    for id in ["invalid".to_owned(), Uuid::now_v7().to_string()] {
        let code = if id == "invalid" {
            Code::InvalidArgument
        } else {
            Code::NotFound
        };
        assert_eq!(
            app.grpc
                .transfer_options(pb::TransferOptionsRequest {
                    character_id: id.clone()
                })
                .await
                .unwrap_err()
                .code(),
            code
        );
        assert_eq!(
            app.grpc
                .change_class(change(id, key, 1))
                .await
                .unwrap_err()
                .code(),
            code
        );
    }
    let mut bad = mutation.clone();
    bad.idempotency_key = "invalid".into();
    assert_eq!(app.grpc.change_class(bad).await.unwrap_err().code(), Code::InvalidArgument);
    assert_eq!(
        app.grpc
            .change_class(change(c.id.to_string(), key, 123))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        app.grpc.transfer_options(options).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    assert_eq!(
        app.grpc.change_class(mutation).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    assert!(app.characters.staged_events().is_empty());
}

#[tokio::test]
async fn offline_retry_returns_original_frozen_wire_response_after_later_checkpoints() {
    let mut app = TestApp::spawn().await;
    let c = fixture();
    let result = frozen(&c);
    let key = IdempotencyKey::new();
    app.characters.insert_for_test(c.clone());
    let mut ledger = c.class_state.clone();
    ledger.current_class_id = ClassId(1);
    ledger.sp = 55;
    ledger.token_tier_2_count = 1;
    ledger
        .record_success(SuccessfulTransferReceipt {
            key: key.as_uuid(),
            target_class_id: ClassId(1),
            result: result.clone(),
        })
        .unwrap();
    let cp = CharacterCheckpoint {
        character_id: c.id,
        revision_seen: 0,
        level: 20,
        xp: 100,
        hp: 100,
        mp: 40,
        alive: true,
        position: Position { x: 126.0, y: 126.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: Some(ledger),
    };
    app.characters.checkpoint(&cp, &[]).await.unwrap();
    let request = change(c.id.to_string(), key.as_uuid(), 1);
    let first = app
        .grpc
        .change_class(request.clone())
        .await
        .unwrap()
        .into_inner();
    let character = first.character.as_ref().unwrap();
    assert_eq!(
        (
            character.class_id,
            character.base_class_id,
            character.level,
            character.active_class_slot
        ),
        (1, 0, 20, 0)
    );
    assert_eq!(
        character.classes,
        vec![pb::ClassProgress {
            slot: 0,
            class_id: 1,
            level: 20,
            exp: 100,
            sp: 55
        }]
    );
    assert_eq!(character.position, Some(pb::Position { x: 126.0, y: 126.0 }));
    assert_eq!((first.token_tier_1_count, first.token_tier_2_count), (0, 1));
    assert!(first.granted_skill_keys.is_empty());
    let later = CharacterCheckpoint {
        revision_seen: 1,
        level: 40,
        xp: 500,
        position: Position { x: 10.0, y: 20.0 },
        idempotency: ("save_checkpoint".into(), IdempotencyKey::new()),
        class_state: None,
        ..cp
    };
    app.characters.checkpoint(&later, &[]).await.unwrap();
    assert_eq!(app.grpc.change_class(request).await.unwrap().into_inner(), first);
    let current = app
        .grpc
        .get_character(pb::GetCharacterRequest {
            character_id: c.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(current.level, 40);
    assert_eq!(current.position, Some(pb::Position { x: 10.0, y: 20.0 }));
    assert_eq!(
        app.grpc
            .change_class(change(c.id.to_string(), key.as_uuid(), 4))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    assert!(app.characters.staged_events().is_empty());
}

fn phase2_registry() -> std::sync::Arc<nightfall_api::domain::class::ClassRegistry> {
    nightfall_api::infrastructure::class_data::load_classes(
        &nightfall_api::infrastructure::class_data::ClassSource::embedded(),
    )
    .unwrap()
    .registry
}

fn seed_transfer_player(
    app: &TestApp,
    name: &str,
    level: u32,
    position: Position,
    tokens: u32,
) -> common::ws::Player {
    let account = Uuid::now_v7();
    let mut character = Character::create(
        AccountId::from_uuid(account),
        CharacterName::new(name).unwrap(),
        Race::Human,
    );
    character.level = level;
    character.xp = common::rules().xp_to_level(level).unwrap();
    character.position = position;
    character.class_state.token_tier_1_count = tokens;
    character.class_state.token_tier_2_count = tokens;
    let player = common::ws::Player {
        account,
        character: character.id,
        name: name.into(),
    };
    app.characters.insert_for_test(character);
    player
}

fn spawned(message: &pb::ServerMessage, player: &common::ws::Player) -> bool {
    matches!(&message.payload, Some(pb::server_message::Payload::Event(event))
        if matches!(&event.event, Some(pb::world_event::Event::Spawn(spawn)) if spawn.entity_id == player.entity_id()))
}

fn changed(message: &pb::ServerMessage, player: &common::ws::Player, target: u32) -> bool {
    matches!(&message.payload, Some(pb::server_message::Payload::Event(event))
        if matches!(&event.event, Some(pb::world_event::Event::ClassChanged(change)) if change.entity == player.entity_id() && change.class_id == target))
}

#[tokio::test]
async fn live_transfers_checkpoint_before_response_notify_observer_and_replay_both_frozen_results()
{
    let app = TestApp::spawn_with_zone(
        |_| {},
        common::fixture_zone()
            .with_classes(phase2_registry())
            .unwrap(),
    )
    .await;
    let owner = seed_transfer_player(&app, "TransferHero", 40, Position { x: 126.0, y: 126.0 }, 1);
    let observer = common::ws::seed_player(&app, "Observer", 126.0, 126.0);
    let mut owner_ws = common::ws::join(&app, &owner).await;
    owner_ws.until(|m| spawned(m, &owner)).await;
    let mut observer_ws = common::ws::join(&app, &observer).await;
    observer_ws.until(|m| spawned(m, &owner)).await;
    let mut client = app.game_as(owner.account);
    let options = pb::TransferOptionsRequest {
        character_id: owner.character.to_string(),
    };
    let initial = client
        .transfer_options(options.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(initial.current_class_id, 0);
    assert_eq!((initial.token_tier_1_count, initial.token_tier_2_count), (1, 1));
    assert_eq!(
        initial
            .options
            .iter()
            .map(|o| o.class_id)
            .collect::<Vec<_>>(),
        vec![1, 4, 7]
    );
    assert!(initial
        .options
        .iter()
        .all(|o| o.eligible && o.unmet.is_empty()));
    // Failed attempts are not retained; this key may subsequently request a valid branch.
    let key = Uuid::now_v7();
    assert_eq!(
        client
            .change_class(change(owner.character.to_string(), key, 88))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let request = change(owner.character.to_string(), key, 1);
    let mut concurrent = client.clone();
    let (first, retry) = tokio::join!(
        client.change_class(request.clone()),
        concurrent.change_class(request.clone())
    );
    let first = first.unwrap().into_inner();
    assert_eq!(retry.unwrap().into_inner(), first);
    assert_eq!(first.character.as_ref().unwrap().class_id, 1);
    assert_eq!((first.token_tier_1_count, first.token_tier_2_count), (0, 1));
    assert!(!first.granted_skill_keys.is_empty());
    let stored = app.characters.get(owner.character).await.unwrap().unwrap();
    assert_eq!(stored.class_state.current_class_id, ClassId(1));
    assert_eq!(stored.class_state.successful_transfer_receipts.len(), 1);
    assert!(first.granted_skill_keys.iter().all(|key| stored
        .class_state
        .learned_skills
        .iter()
        .any(|skill| &skill.key == key)));
    observer_ws.until(|m| changed(m, &owner, 1)).await;
    owner_ws.until(|m| changed(m, &owner, 1)).await;
    assert_eq!(
        client
            .change_class(change(owner.character.to_string(), key, 4))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let after_first = client
        .transfer_options(options.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(after_first.current_class_id, 1);
    assert_eq!(
        after_first
            .options
            .iter()
            .map(|o| o.class_id)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(after_first.options.iter().all(|o| o.eligible));
    let second_request = change(owner.character.to_string(), Uuid::now_v7(), 2);
    let second = client
        .change_class(second_request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.character.as_ref().unwrap().class_id, 2);
    assert_eq!((second.token_tier_1_count, second.token_tier_2_count), (0, 0));
    assert_eq!(
        client
            .get_character(pb::GetCharacterRequest {
                character_id: owner.character.to_string()
            })
            .await
            .unwrap()
            .into_inner(),
        second.character.clone().unwrap()
    );
    let third_options = client.transfer_options(options).await.unwrap().into_inner();
    assert_eq!(third_options.options.len(), 1);
    assert_eq!(third_options.options[0].class_id, 88);
    assert!(!third_options.options[0].eligible);
    assert!(!third_options.options[0].unmet.is_empty());
    let stored = app.characters.get(owner.character).await.unwrap().unwrap();
    assert_eq!(stored.class_state.successful_transfer_receipts.len(), 2);
    assert_eq!(client.change_class(request).await.unwrap().into_inner(), first);
    assert_eq!(
        client
            .change_class(second_request)
            .await
            .unwrap()
            .into_inner(),
        second
    );
    assert_eq!(
        app.characters
            .staged_events()
            .iter()
            .filter(|e| matches!(
                e,
                nightfall_api::domain::DomainEvent::CharacterClassChanged { .. }
            ))
            .count(),
        2
    );
    owner_ws.close().await;
    observer_ws.close().await;
}

#[tokio::test]
async fn live_ineligible_options_and_transfers_leave_tokens_and_receipts_unchanged() {
    let app = TestApp::spawn_with_zone(
        |_| {},
        common::fixture_zone()
            .with_classes(phase2_registry())
            .unwrap(),
    )
    .await;
    for (name, level, position, tokens) in [
        ("LowLevel", 1, Position { x: 126.0, y: 126.0 }, 1),
        ("NoTokens", 40, Position { x: 126.0, y: 126.0 }, 0),
        ("TooFar", 40, Position { x: 0.0, y: 0.0 }, 1),
    ] {
        let player = seed_transfer_player(&app, name, level, position, tokens);
        let before = app.characters.get(player.character).await.unwrap().unwrap();
        let mut ws = common::ws::join(&app, &player).await;
        ws.until(|m| spawned(m, &player)).await;
        let mut client = app.game_as(player.account);
        let options = client
            .transfer_options(pb::TransferOptionsRequest {
                character_id: player.character.to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(options
            .options
            .iter()
            .all(|o| !o.eligible && !o.unmet.is_empty()));
        for target in [1, 18, 88] {
            assert_eq!(
                client
                    .change_class(change(player.character.to_string(), Uuid::now_v7(), target))
                    .await
                    .unwrap_err()
                    .code(),
                Code::FailedPrecondition
            );
        }
        let after = app.characters.get(player.character).await.unwrap().unwrap();
        assert_eq!(after.class_state, before.class_state);
        ws.close().await;
    }
    assert!(app
        .characters
        .staged_events()
        .iter()
        .all(|e| !matches!(e, nightfall_api::domain::DomainEvent::CharacterClassChanged { .. })));
}

struct FailingRuntime(tonic::Code);
impl FailingRuntime {
    fn error(&self) -> nightfall_api::application::AppError {
        use nightfall_api::application::AppError;
        match self.0 {
            Code::FailedPrecondition => AppError::FailedPrecondition("requirements changed".into()),
            Code::ResourceExhausted => AppError::ResourceExhausted("command queue full".into()),
            Code::Unavailable => AppError::Unavailable("zone recovering".into()),
            _ => AppError::Infrastructure(anyhow::anyhow!("private database diagnostic")),
        }
    }
}
#[async_trait::async_trait]
impl nightfall_api::application::ports::ClassTransferRuntime for FailingRuntime {
    async fn transfer_options(
        &self,
        _: AccountId,
        _: nightfall_api::domain::CharacterId,
    ) -> Result<
        nightfall_api::application::ports::TransferOptionsState,
        nightfall_api::application::AppError,
    > {
        Err(self.error())
    }
    async fn change_class(
        &self,
        _: AccountId,
        _: nightfall_api::domain::CharacterId,
        _: ClassId,
        _: IdempotencyKey,
    ) -> Result<FrozenTransferResult, nightfall_api::application::AppError> {
        Err(self.error())
    }
}

#[tokio::test]
async fn runtime_failures_map_through_real_grpc_without_claiming_success_or_exposing_diagnostics() {
    for code in [
        Code::FailedPrecondition,
        Code::ResourceExhausted,
        Code::Unavailable,
        Code::Internal,
    ] {
        let mut app = TestApp::spawn_with(|deps| {
            deps.class_transfers = Some(std::sync::Arc::new(FailingRuntime(code)))
        })
        .await;
        let c = fixture();
        app.characters.insert_for_test(c.clone());
        let request = change(c.id.to_string(), Uuid::now_v7(), 1);
        for _ in 0..2 {
            let error = app.grpc.change_class(request.clone()).await.unwrap_err();
            assert_eq!(error.code(), code);
            assert!(!error.message().contains("private database diagnostic"));
        }
        let error = app
            .grpc
            .transfer_options(pb::TransferOptionsRequest {
                character_id: c.id.to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        assert!(!error.message().contains("private database diagnostic"));
        assert_eq!(app.characters.get(c.id).await.unwrap().unwrap(), c);
        assert!(app.characters.staged_events().is_empty());
    }
}
