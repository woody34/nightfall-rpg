//! Secrets must never reach spans or logs (plan Revision 1, item 7): the play ticket travels in
//! an `Authorization` header and possibly a query string, and neither may be recorded.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::capture::Capture;
use common::TestApp;

#[tokio::test]
async fn query_string_and_authorization_header_never_appear_in_spans_or_logs() {
    const QUERY_SECRET: &str = "SENTINEL-QUERY-4f1c9a";
    const HEADER_SECRET: &str = "SENTINEL-BEARER-77d2e0";

    let capture = Capture::default();
    let _guard = capture.install();

    let app = TestApp::spawn().await;
    let res = reqwest::Client::new()
        .get(format!("{}/health?ticket={QUERY_SECRET}", app.http_base))
        .header("authorization", format!("Bearer {HEADER_SECRET}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let logged = capture.text();
    assert!(
        logged.contains("\"http.path\":\"/health\""),
        "request span not captured, test is vacuous:\n{logged}"
    );
    assert!(!logged.contains(QUERY_SECRET), "query string leaked:\n{logged}");
    assert!(!logged.contains(HEADER_SECRET), "header leaked:\n{logged}");
}
