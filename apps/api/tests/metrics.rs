#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use common::{TestApp, ACCOUNT, KEY_A};
use nightfall_api::infrastructure::telemetry::{FrameDirection, OutboxStats, OutboxStatsSource};
use nightfall_api::interface::grpc::pb::{
    CreateCharacterRequest, GetCharacterRequest, IssuePlayTicketRequest, ListMyCharactersRequest,
    PingRequest,
};
use std::sync::Arc;

async fn scrape(app: &TestApp) -> String {
    reqwest::get(format!("{}/metrics", app.http_base))
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
}

/// A create request. `account_id` is deprecated on the wire but still required until Story 1.6
/// moves the account into the token; scoped here rather than silencing the whole crate.
#[allow(deprecated)]
fn create_request(name: &str) -> CreateCharacterRequest {
    CreateCharacterRequest {
        idempotency_key: KEY_A.into(),
        account_id: ACCOUNT.into(),
        name: name.into(),
        race: 0,
    }
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
        .create_character(create_request("x"))
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

#[tokio::test]
async fn arbitrary_grpc_paths_share_one_bucketed_series() {
    let mut app = TestApp::spawn().await;
    let mut client = tonic::client::Grpc::new(
        tonic::transport::Channel::from_shared(app.grpc_base.clone())
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    let junk = [
        "/evil-1f2e3d/Method",
        "/nightfall.v1.GameService/NotARealMethod-9a8b",
        "/random.Service/Ping",
        "/only-one-segment-77",
        "/a/b/c/d-5e6f",
    ];
    for path in junk {
        client.ready().await.unwrap();
        let res: Result<tonic::Response<PingRequest>, _> = client
            .unary(
                tonic::Request::new(PingRequest::default()),
                path.parse().unwrap(),
                tonic_prost::ProstCodec::<PingRequest, PingRequest>::default(),
            )
            .await;
        assert_eq!(res.unwrap_err().code(), tonic::Code::Unimplemented, "{path}");
    }
    // A real call still gets its real labels.
    app.grpc
        .ping(PingRequest {
            client_version: "t".into(),
        })
        .await
        .unwrap();

    let text = scrape(&app).await;
    for needle in [
        "evil-1f2e3d",
        "NotARealMethod",
        "random.Service",
        "only-one-segment",
        "5e6f",
    ] {
        assert!(!text.contains(needle), "{needle} became a label:\n{text}");
    }
    let unknown: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.starts_with("nightfall_grpc_requests_total") && l.contains("service=\"unknown\"")
        })
        .collect();
    assert_eq!(unknown.len(), 1, "{unknown:?}");
    assert!(
        unknown[0].contains("method=\"unknown\"") && unknown[0].ends_with(" 5"),
        "{unknown:?}"
    );
    assert!(text.contains("method=\"Ping\""), "{text}");
}

/// Guards the label table in `layers.rs` against drifting from the protos: every implemented
/// RPC must land on its own labels, never in the `unknown` bucket.
#[tokio::test]
async fn every_implemented_rpc_has_a_known_label() {
    let mut app = TestApp::spawn().await;
    app.grpc.ping(PingRequest::default()).await.unwrap();
    app.grpc
        .get_character(GetCharacterRequest {
            character_id: uuid::Uuid::nil().to_string(),
        })
        .await
        .unwrap_err();
    app.grpc
        .create_character(create_request("x"))
        .await
        .unwrap_err();
    app.grpc
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap_err();
    app.session
        .issue_play_ticket(IssuePlayTicketRequest::default())
        .await
        .unwrap_err();

    let text = scrape(&app).await;
    assert!(!text.contains("service=\"unknown\""), "{text}");
    for (service, method) in [
        ("nightfall.v1.GameService", "Ping"),
        ("nightfall.v1.GameService", "GetCharacter"),
        ("nightfall.v1.GameService", "CreateCharacter"),
        ("nightfall.v1.GameService", "ListMyCharacters"),
        ("nightfall.v1.SessionService", "IssuePlayTicket"),
    ] {
        assert!(
            text.lines()
                .any(|l| l.contains(&format!("service=\"{service}\""))
                    && l.contains(&format!("method=\"{method}\""))),
            "missing {service}/{method} in:\n{text}"
        );
    }
}
