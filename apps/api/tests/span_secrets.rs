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
use common::{account, TestApp, KEY_A};
use nightfall_api::application::use_cases::ConsumePlayTicket;
use nightfall_api::domain::{AccountId, Character, CharacterName, PlayTicket, Race};
use nightfall_api::infrastructure::memory::FixedSecretGenerator;
use nightfall_api::infrastructure::SystemClock;
use nightfall_api::interface::grpc::pb::IssuePlayTicketRequest;

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

#[tokio::test]
async fn play_ticket_never_appears_in_spans_or_logs() {
    const SENTINEL: [u8; 32] = *b"SENTINEL-TICKET-never-log-me-42!";
    let secret = PlayTicket::from_bytes(SENTINEL).encode();

    let capture = Capture::default();
    let _guard = capture.install();

    let mut app = TestApp::spawn_with(|deps| {
        deps.secrets = std::sync::Arc::new(FixedSecretGenerator(SENTINEL));
    })
    .await;
    let c = Character::create(
        AccountId::from_uuid(account()),
        CharacterName::new("Aria").unwrap(),
        Race::Elf,
    );
    app.characters.insert_for_test(c.clone());
    let req = || IssuePlayTicketRequest {
        idempotency_key: KEY_A.into(),
        character_id: c.id.to_string(),
    };

    // Issue, replay, consume, and consume again: every path that touches the ticket.
    let issued = app
        .session
        .issue_play_ticket(req())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(issued.ticket, secret, "the sentinel really was issued");
    app.session.issue_play_ticket(req()).await.unwrap();
    let consume = ConsumePlayTicket::new(app.sessions.clone(), std::sync::Arc::new(SystemClock));
    consume.execute(&issued.ticket).await.unwrap();
    consume.execute(&issued.ticket).await.unwrap_err();

    let logged = capture.text();
    assert!(
        logged.contains("IssuePlayTicket") && logged.contains("play ticket issued"),
        "issue spans and logs not captured, test is vacuous:\n{logged}"
    );
    assert!(!logged.contains(&secret), "play ticket leaked:\n{logged}");
    assert!(!logged.contains("SENTINEL-TICKET"), "raw ticket bytes leaked:\n{logged}");
}
