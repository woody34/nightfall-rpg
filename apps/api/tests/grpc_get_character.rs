#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::{account, other_account, TestApp};
use nightfall_api::domain::{AccountId, Character, CharacterName, Race};
use nightfall_api::interface::grpc::pb::{self, GetCharacterRequest};
use tonic::Code;
use uuid::Uuid;

fn seed(app: &TestApp, owner: Uuid) -> Character {
    let c = Character::create(
        AccountId::from_uuid(owner),
        CharacterName::new("Samwise").unwrap(),
        Race::Human,
    );
    app.characters.insert_for_test(c.clone());
    c
}

#[tokio::test]
async fn returns_own_character() {
    let mut app = TestApp::spawn().await;
    let c = seed(&app, account());

    let got = app
        .grpc
        .get_character(GetCharacterRequest {
            character_id: c.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(got.id, c.id.to_string());
    assert_eq!(got.name, "Samwise");
    assert_eq!(got.race, pb::Race::Human as i32);
    assert_eq!(got.level, 1);
    assert_eq!(got.stats.unwrap().str, Race::Human.starting_stats().str);
    assert_eq!(got.position, Some(pb::Position { x: 0.0, y: 0.0 }));
}

#[tokio::test]
async fn another_accounts_character_is_permission_denied() {
    let app = TestApp::spawn().await;
    let c = seed(&app, account());
    let err = app
        .game_as(other_account())
        .get_character(GetCharacterRequest {
            character_id: c.id.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn unknown_id_is_not_found() {
    let mut app = TestApp::spawn().await;
    let err = app
        .grpc
        .get_character(GetCharacterRequest {
            character_id: Uuid::nil().to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn malformed_id_is_invalid_argument() {
    let mut app = TestApp::spawn().await;
    let err = app
        .grpc
        .get_character(GetCharacterRequest {
            character_id: "not-a-uuid".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn without_a_token_is_unauthenticated() {
    let app = TestApp::spawn().await;
    let c = seed(&app, account());
    let err = app
        .anon_grpc()
        .get_character(GetCharacterRequest {
            character_id: c.id.to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}
