//! The dev gate is authenticated and precedes every durable input or actor mutation.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use common::TestApp;
use nightfall_api::interface::grpc::pb;
use tonic::Code;

#[tokio::test]
async fn grant_requires_explicit_dev_gate_for_every_authenticated_account() {
    let app = TestApp::spawn().await;
    let before = app.characters.staged_events();
    let request = pb::GrantItemRequest::default();
    for mut client in [app.anon_grpc(), app.game_with_token("invalid")] {
        assert_eq!(
            client.grant_item(request.clone()).await.unwrap_err().code(),
            Code::Unauthenticated
        );
    }
    for account in [common::account(), uuid::Uuid::from_u128(77)] {
        let status = app
            .game_as(account)
            .grant_item(request.clone())
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert!(status.message().contains("AUTH_DEV_TOKENS=1"));
    }
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
    assert_eq!(app.characters.len(), 0);
}

#[tokio::test]
async fn enabled_dev_gate_requires_the_item_runtime_and_authentication() {
    let app = TestApp::spawn_with(|deps| deps.dev_item_grants = true).await;
    assert_eq!(
        app.anon_grpc()
            .grant_item(pb::GrantItemRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    for _ in 0..2 {
        assert_eq!(
            app.game_as(common::account())
                .grant_item(pb::GrantItemRequest::default())
                .await
                .unwrap_err()
                .code(),
            Code::Unavailable
        );
    }
    assert_eq!(app.characters.len(), 0);
    assert!(app.characters.staged_events().is_empty());
    assert!(app.bus.published().is_empty());
}
