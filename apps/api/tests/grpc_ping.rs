#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::TestApp;
use nightfall_api::interface::grpc::pb::PingRequest;

#[tokio::test]
async fn ping_returns_version_and_recent_time() {
    let mut app = TestApp::spawn().await;
    let before = chrono::Utc::now().timestamp_millis();
    let res = app
        .grpc
        .ping(PingRequest {
            client_version: "test".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let after = chrono::Utc::now().timestamp_millis();
    assert_eq!(res.server_version, env!("CARGO_PKG_VERSION"));
    assert!((before..=after).contains(&res.server_time_ms));
}

#[tokio::test]
async fn ping_is_idempotent() {
    let mut app = TestApp::spawn().await;
    let a = app
        .grpc
        .ping(PingRequest::default())
        .await
        .unwrap()
        .into_inner();
    let b = app
        .grpc
        .ping(PingRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(a.server_version, b.server_version);
}
