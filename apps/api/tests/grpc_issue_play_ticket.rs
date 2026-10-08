//! `SessionService.IssuePlayTicket` through the wire (Story 1.4, api-guidelines.md section 4),
//! and the consume side the `/ws` handshake will use.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::{account, other_account, TestApp, KEY_A, KEY_B};
use nightfall_api::application::use_cases::{ConsumePlayTicket, TicketRejection};
use nightfall_api::application::Clock;
use nightfall_api::domain::{AccountId, Character, CharacterId, CharacterName, Race};
use nightfall_api::infrastructure::memory::ManualClock;
use nightfall_api::interface::grpc::pb::{IssuePlayTicketRequest, IssuePlayTicketResponse};
use tonic::Code;
use uuid::Uuid;

fn seed(app: &TestApp, owner: Uuid, name: &str) -> CharacterId {
    let c = Character::create(
        AccountId::from_uuid(owner),
        CharacterName::new(name).unwrap(),
        Race::Elf,
    );
    app.characters.insert_for_test(c.clone());
    c.id
}

fn req(key: &str, character: CharacterId) -> IssuePlayTicketRequest {
    IssuePlayTicketRequest {
        idempotency_key: key.into(),
        character_id: character.to_string(),
    }
}

/// Spawns with a manual clock so expiry is testable and timestamps are exact.
async fn spawn() -> (TestApp, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::default());
    let c = clock.clone();
    let app = TestApp::spawn_with(move |deps| deps.clock = c).await;
    (app, clock)
}

fn consumer(app: &TestApp, clock: Arc<ManualClock>) -> ConsumePlayTicket {
    ConsumePlayTicket::new(app.sessions.clone(), clock)
}

async fn issue(app: &mut TestApp, key: &str, c: CharacterId) -> IssuePlayTicketResponse {
    app.session
        .issue_play_ticket(req(key, c))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn issues_a_ticket_that_admits_the_caller_as_the_character() {
    let (mut app, clock) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let res = issue(&mut app, KEY_A, c).await;

    assert_eq!(res.ticket.len(), 43, "32 bytes, base64url, unpadded");
    assert!(res
        .ticket
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    assert_eq!(res.expires_at_ms, clock.now().timestamp_millis() + 60_000);
    assert_eq!(res.ws_url, "ws://localhost:3000/ws");
    assert!(!res.ws_url.contains(&res.ticket), "ticket never in the URL");

    let admission = consumer(&app, clock).execute(&res.ticket).await.unwrap();
    assert_eq!(admission.account_id, AccountId::from_uuid(account()));
    assert_eq!(admission.character_id, c);
    assert_eq!(admission.generation.get(), 1);
}

#[tokio::test]
async fn ws_url_comes_from_configuration() {
    let mut app =
        TestApp::spawn_with(|deps| deps.ws_public_url = "wss://play.test/ws".into()).await;
    let c = seed(&app, account(), "Aria");
    assert_eq!(issue(&mut app, KEY_A, c).await.ws_url, "wss://play.test/ws");
}

#[tokio::test]
async fn retry_with_same_key_returns_the_identical_ticket_even_after_consumption() {
    let (mut app, clock) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let first = issue(&mut app, KEY_A, c).await;
    consumer(&app, clock.clone())
        .execute(&first.ticket)
        .await
        .unwrap();

    clock.advance(chrono::Duration::seconds(5));
    let retry = issue(&mut app, KEY_A, c).await;
    assert_eq!(retry, first);
    assert_eq!(app.sessions.ticket_count(), 1, "no second ticket");
    assert_eq!(
        app.sessions
            .generation(AccountId::from_uuid(account()))
            .unwrap()
            .get(),
        1,
        "no second generation bump"
    );
    // The replayed ticket is spent.
    let err = consumer(&app, clock)
        .execute(&retry.ticket)
        .await
        .unwrap_err();
    assert!(matches!(err, TicketRejection::Consumed), "{err:?}");
}

#[tokio::test]
async fn same_key_with_a_different_character_is_failed_precondition() {
    let (mut app, _) = spawn().await;
    let c1 = seed(&app, account(), "Aria");
    let c2 = seed(&app, account(), "Brea");
    issue(&mut app, KEY_A, c1).await;
    let err = app
        .session
        .issue_play_ticket(req(KEY_A, c2))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(app.sessions.ticket_count(), 1);
}

#[tokio::test]
async fn another_accounts_character_is_permission_denied() {
    let (app, _) = spawn().await;
    let theirs = seed(&app, other_account(), "Aria");
    let err = app
        .session_as(account())
        .issue_play_ticket(req(KEY_A, theirs))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(app.sessions.ticket_count(), 0);
}

#[tokio::test]
async fn unknown_character_is_not_found() {
    let (mut app, _) = spawn().await;
    let err = app
        .session
        .issue_play_ticket(req(KEY_A, CharacterId::new()))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn malformed_key_or_character_id_is_invalid_argument() {
    let (mut app, _) = spawn().await;
    let c = seed(&app, account(), "Aria");
    for bad in [
        req("", c),
        req("nope", c),
        IssuePlayTicketRequest {
            idempotency_key: KEY_A.into(),
            character_id: "not-a-uuid".into(),
        },
    ] {
        let err = app.session.issue_play_ticket(bad).await.unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }
    assert_eq!(app.sessions.ticket_count(), 0);
}

#[tokio::test]
async fn without_a_token_is_unauthenticated() {
    let (app, _) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let err = app
        .anon_session()
        .issue_play_ticket(req(KEY_A, c))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
    assert_eq!(app.sessions.ticket_count(), 0);
}

#[tokio::test]
async fn consuming_twice_fails() {
    let (mut app, clock) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let t = issue(&mut app, KEY_A, c).await.ticket;
    let consume = consumer(&app, clock);
    consume.execute(&t).await.unwrap();
    assert!(matches!(consume.execute(&t).await, Err(TicketRejection::Consumed)));
}

#[tokio::test]
async fn consuming_after_expiry_fails() {
    let (mut app, clock) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let t = issue(&mut app, KEY_A, c).await.ticket;
    clock.advance(chrono::Duration::seconds(60));
    let err = consumer(&app, clock).execute(&t).await.unwrap_err();
    assert!(matches!(err, TicketRejection::Expired), "{err:?}");
}

#[tokio::test]
async fn back_to_back_tickets_increase_the_generation_and_supersede_the_older() {
    let (mut app, clock) = spawn().await;
    let c = seed(&app, account(), "Aria");
    let older = issue(&mut app, KEY_A, c).await.ticket;
    let newer = issue(&mut app, KEY_B, c).await.ticket;
    assert_ne!(older, newer);

    let consume = consumer(&app, clock);
    let admission = consume.execute(&newer).await.unwrap();
    assert_eq!(admission.generation.get(), 2);
    let err = consume.execute(&older).await.unwrap_err();
    assert!(matches!(err, TicketRejection::Superseded), "{err:?}");
}
