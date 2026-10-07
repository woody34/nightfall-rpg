#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::TestApp;

#[tokio::test]
async fn health_returns_ok_and_version() {
    let app = TestApp::spawn().await;
    let res = reqwest::get(format!("{}/health", app.http_base))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn unknown_route_is_404() {
    let app = TestApp::spawn().await;
    let res = reqwest::get(format!("{}/nope", app.http_base))
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}
