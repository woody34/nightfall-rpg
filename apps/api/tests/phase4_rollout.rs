//! Real sockets prove the additive item contract fails closed before runtime installation.
#![allow(missing_docs, clippy::unwrap_used)]
mod common;
use common::TestApp;
use nightfall_api::interface::grpc::pb;
use tonic::Code;

#[tokio::test]
async fn absent_item_ports_require_authentication_and_write_nothing() {
    let app = TestApp::spawn().await;
    let owner = common::ws::seed_player(&app, "InventoryOwner", 10.0, 10.0);
    let inventory_request = pb::GetInventoryRequest {
        character_id: owner.character.as_uuid().as_bytes().to_vec(),
    };
    let before = app.characters.staged_events();
    for (mut client, expected) in [
        (app.anon_grpc(), Code::Unauthenticated),
        (app.game_as(owner.account), Code::Unavailable),
    ] {
        assert_eq!(
            client
                .get_item_catalog(pb::GetItemCatalogRequest::default())
                .await
                .unwrap_err()
                .code(),
            expected
        );
        assert_eq!(
            client
                .get_inventory(inventory_request.clone())
                .await
                .unwrap_err()
                .code(),
            expected
        );
        assert_eq!(
            client
                .grant_item(pb::GrantItemRequest::default())
                .await
                .unwrap_err()
                .code(),
            expected
        );
    }
    assert_eq!(app.characters.staged_events(), before);
    assert!(app.bus.published().is_empty());
}
