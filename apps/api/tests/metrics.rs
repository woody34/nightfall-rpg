#![allow(
    deprecated,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::{TestApp, ACCOUNT, KEY_A};
use nightfall_api::infrastructure::telemetry::{FrameDirection, OutboxStats, OutboxStatsSource};
use nightfall_api::interface::grpc::pb::{CreateCharacterRequest, PingRequest};
use std::sync::Arc;

async fn scrape(app: &TestApp) -> String {
    reqwest::get(format!("{}/metrics", app.http_base))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

struct FixedOutbox;

impl OutboxStatsSource for FixedOutbox {
    fn snapshot(&self) -> OutboxStats {
        OutboxStats {
            pending: 7,
            lag_seconds: 12.5,
        }
    }
}

#[tokio::test]
async fn metrics_endpoint_exposes_the_whole_catalogue() {
    let mut app = TestApp::spawn().await;

    // Real producers.
    reqwest::get(format!("{}/health", app.http_base))
        .await
        .unwrap();
    app.grpc
        .ping(PingRequest {
            client_version: "t".into(),
        })
        .await
        .unwrap();
    // Instruments whose producers (tick loop, websocket, event log, DB) are not built yet:
    // one sample each, as a producer would record it.
    app.metrics.tick_duration_seconds.record(0.004, &[]);
    app.metrics.sessions_active.add(1, &[]);
    app.metrics.record_ws_frames(FrameDirection::In, 3);
    app.metrics.ws_dropped_frames_total.add(1, &[]);
    app.metrics.eventlog_publish_seconds.record(0.002, &[]);
    app.metrics.time_db("character", "get", async {}).await;
    app.metrics.set_outbox_source(Arc::new(FixedOutbox));

    let text = scrape(&app).await;
    for name in [
        "nightfall_tick_duration_seconds_bucket",
        "nightfall_sessions_active",
        "nightfall_ws_frames_total{direction=\"in\"",
        "nightfall_ws_dropped_frames_total",
        "nightfall_outbox_pending",
        "nightfall_outbox_lag_seconds",
        "nightfall_db_query_seconds_bucket",
        "nightfall_eventlog_publish_seconds_bucket",
        "nightfall_grpc_requests_total",
        "nightfall_http_requests_total",
    ] {
        assert!(text.contains(name), "missing {name} in:\n{text}");
    }
    assert!(
        text.contains("nightfall_outbox_pending{otel_scope_name=\"nightfall-api\"} 7"),
        "{text}"
    );
    assert!(
        text.contains("nightfall_outbox_lag_seconds{otel_scope_name=\"nightfall-api\"} 12.5"),
        "{text}"
    );
    assert!(
        text.contains("service=\"nightfall.v1.GameService\"")
            && text.contains("method=\"Ping\"")
            && text.contains("code=\"Ok\""),
        "{text}"
    );
    assert!(text.contains("route=\"/health\""), "{text}");
}

#[tokio::test]
async fn grpc_error_codes_are_labelled_and_unknown_routes_are_bucketed() {
    let mut app = TestApp::spawn().await;
    app.grpc
        .create_character(CreateCharacterRequest {
            idempotency_key: KEY_A.into(),
            account_id: ACCOUNT.into(),
            name: "x".into(),
            race: 0,
        })
        .await
        .unwrap_err();
    reqwest::get(format!("{}/nope", app.http_base))
        .await
        .unwrap();

    let text = scrape(&app).await;
    assert!(text.contains("method=\"CreateCharacter\""), "{text}");
    assert!(text.contains("code=\"InvalidArgument\""), "{text}");
    assert!(text.contains("route=\"unmatched\""), "{text}");
}

#[tokio::test]
async fn responses_carry_a_uuid_v7_request_id() {
    let app = TestApp::spawn().await;
    let res = reqwest::get(format!("{}/health", app.http_base))
        .await
        .unwrap();
    let id = res.headers()["x-request-id"].to_str().unwrap();
    let parsed = uuid::Uuid::parse_str(id).unwrap();
    assert_eq!(parsed.get_version_num(), 7);
}
