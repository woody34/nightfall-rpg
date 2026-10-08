//! Connection URLs carry credentials. Whatever the adapters log while connecting, the password
//! must not appear in any emitted log line or span (they are exported to Loki).
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::capture::Capture;
use nightfall_api::infrastructure::nats::NatsEventBus;
use nightfall_api::infrastructure::postgres;

const SENTINEL: &str = "SENTINEL-PASSWORD-9c3e71";

#[tokio::test]
async fn nats_url_password_never_reaches_logs() {
    let capture = Capture::default();
    let _guard = capture.install();

    // Reachable server (a credential-less server ignores the user info) and an unreachable one,
    // so both the success and failure paths are exercised.
    if let Ok(url) = std::env::var("NATS_URL") {
        let authed = url.replacen("://", &format!("://user:{SENTINEL}@"), 1);
        let connected = NatsEventBus::connect(&authed).await;
        let logged = capture.text();
        assert!(
            connected.is_ok() && logged.contains("connected to NATS"),
            "success path not captured, test is vacuous:\n{logged}"
        );
    }
    let failed = NatsEventBus::connect(&format!("nats://user:{SENTINEL}@127.0.0.1:1")).await;
    assert!(failed.is_err());

    let logged = capture.text();
    assert!(!logged.contains(SENTINEL), "NATS password leaked:\n{logged}");
}

#[tokio::test]
async fn database_url_password_never_reaches_logs() {
    let Some(db) = common::pg::fresh_database(Some(SENTINEL)).await else {
        return;
    };
    let capture = Capture::default();
    let _guard = capture.install();

    postgres::connect(&db.url).await.unwrap();
    let logged = capture.text();
    assert!(
        logged.contains("postgres connected"),
        "success path not captured, test is vacuous:\n{logged}"
    );
    // Wrong password: the failure path must not echo the URL either.
    let bad = db.url.replace(SENTINEL, &format!("{SENTINEL}-wrong"));
    assert!(postgres::connect(&bad).await.is_err());

    let logged = capture.text();
    assert!(!logged.contains(SENTINEL), "database password leaked:\n{logged}");
    db.drop_db().await;
}
