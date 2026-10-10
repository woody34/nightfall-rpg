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
