#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::{other_account, TestApp, KEY_A, KEY_B};
use nightfall_api::interface::grpc::pb::{self, CreateCharacterRequest, ListMyCharactersRequest};
use tonic::Code;

fn req(key: &str, name: &str) -> CreateCharacterRequest {
    CreateCharacterRequest {
        idempotency_key: key.into(),
        name: name.into(),
        race: pb::Race::Elf as i32,
        ..Default::default()
    }
}

#[tokio::test]
async fn lists_only_the_callers_characters_in_creation_order() {
    let mut app = TestApp::spawn().await;
    let first = app
        .grpc
        .create_character(req(KEY_A, "Arwen"))
        .await
        .unwrap()
        .into_inner();
    app.game_as(other_account())
        .create_character(req(KEY_A, "Galadriel"))
        .await
        .unwrap();
    let second = app
        .grpc
        .create_character(req(KEY_B, "Elrond"))
        .await
        .unwrap()
        .into_inner();

    let mine = app
        .grpc
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap()
        .into_inner()
        .characters;
    assert_eq!(mine, vec![first, second], "every field, in creation order");
}

#[tokio::test]
async fn account_without_characters_gets_an_empty_list() {
    let mut app = TestApp::spawn().await;
    let res = app
        .grpc
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(res.characters.is_empty());
}

#[tokio::test]
async fn without_a_token_is_unauthenticated() {
    let app = TestApp::spawn().await;
    let err = app
        .anon_grpc()
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn invalid_token_is_unauthenticated() {
    let app = TestApp::spawn().await;
    let err = app
        .game_with_token("not-a-test-token")
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}
