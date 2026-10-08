//! Contract stubs (Story 0.1): the RPCs exist on the wire but are not implemented yet.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::{TestApp, KEY_A};
use nightfall_api::interface::grpc::pb::{IssuePlayTicketRequest, ListMyCharactersRequest};
use tonic::Code;

#[tokio::test]
async fn issue_play_ticket_is_unimplemented() {
    let mut app = TestApp::spawn().await;
    let err = app
        .session
        .issue_play_ticket(IssuePlayTicketRequest {
            idempotency_key: KEY_A.into(),
            character_id: uuid::Uuid::nil().to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn list_my_characters_is_unimplemented() {
    let mut app = TestApp::spawn().await;
    let err = app
        .grpc
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}
