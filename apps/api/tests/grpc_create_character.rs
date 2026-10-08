#![allow(deprecated)] // account_id is deprecated on the wire but still honoured until Story 1.6
#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::{TestApp, ACCOUNT, KEY_A, KEY_B};
use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use nightfall_api::domain::DomainEvent;
use nightfall_api::domain::{Character, CharacterId};
use nightfall_api::interface::grpc::pb::{self, CreateCharacterRequest, GetCharacterRequest};
use tonic::Code;

fn req(key: &str, name: &str) -> CreateCharacterRequest {
    CreateCharacterRequest {
        idempotency_key: key.into(),
        account_id: ACCOUNT.into(),
        name: name.into(),
        race: pb::Race::Orc as i32,
    }
}

#[tokio::test]
async fn creates_character_readable_afterwards_and_publishes_event() {
    let mut app = TestApp::spawn().await;
    let created = app
        .grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap()
        .into_inner();
    // Asserted against literals, not against get_character: a mapping bug shared by both
    // paths must not cancel out. Orc starting stats are the domain's table.
    assert!(uuid::Uuid::parse_str(&created.id).is_ok(), "id {}", created.id);
    assert_eq!(created.name, "Thrall");
    assert_eq!(created.race, pb::Race::Orc as i32);
    assert_eq!(created.level, 1);
    assert_eq!(
        created.stats,
        Some(pb::BaseStats {
            str: 40,
            dex: 26,
            con: 47,
            int: 18,
            wit: 12,
            men: 27,
        })
    );
    assert_eq!(created.position, Some(pb::Position { x: 0.0, y: 0.0 }));

    let fetched = app
        .grpc
        .get_character(GetCharacterRequest {
            character_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fetched, created);

    let events = app.characters.staged_events();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], DomainEvent::CharacterCreated { .. }));
}

#[tokio::test]
async fn retry_with_same_key_returns_same_character_and_no_second_event() {
    let mut app = TestApp::spawn().await;
    let a = app
        .grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap()
        .into_inner();
    let b = app
        .grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(a, b);
    assert_eq!(app.characters.len(), 1);
    assert_eq!(app.characters.staged_events().len(), 1);
}

#[tokio::test]
async fn same_key_different_body_is_failed_precondition() {
    let mut app = TestApp::spawn().await;
    app.grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap();
    let err = app
        .grpc
        .create_character(req(KEY_A, "Garrosh"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
}

#[tokio::test]
async fn duplicate_name_is_already_exists() {
    let mut app = TestApp::spawn().await;
    app.grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap();
    let err = app
        .grpc
        .create_character(req(KEY_B, "thrall"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::AlreadyExists);
}

#[tokio::test]
async fn missing_idempotency_key_is_invalid_argument() {
    let mut app = TestApp::spawn().await;
    let err = app
        .grpc
        .create_character(req("", "Thrall"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert!(err.message().contains("idempotency_key"));
}

#[tokio::test]
async fn unspecified_race_is_invalid_argument() {
    let mut app = TestApp::spawn().await;
    let mut r = req(KEY_A, "Thrall");
    r.race = pb::Race::Unspecified as i32;
    let err = app.grpc.create_character(r).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn bad_name_is_invalid_argument_and_writes_nothing() {
    let mut app = TestApp::spawn().await;
    let err = app
        .grpc
        .create_character(req(KEY_A, "x1"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
    assert_eq!(app.characters.len(), 0);
    assert!(app.characters.staged_events().is_empty());
}

/// A repository whose backing store is down. The error text carries connection details that
/// must never reach the client.
struct BrokenRepository;

const LEAK: &str = "postgres://nightfall:hunter2@db.internal:5432/nightfall";

#[async_trait]
impl CharacterRepository for BrokenRepository {
    async fn get(&self, _id: CharacterId) -> anyhow::Result<Option<Character>> {
        anyhow::bail!("connection to {LEAK} refused")
    }

    async fn create_idempotent(
        &self,
        _key: &IdempotencyKey,
        _fingerprint: &str,
        _character: &Character,
    ) -> Result<CreateOutcome, RepositoryError> {
        Err(RepositoryError::Other(anyhow::anyhow!("connection to {LEAK} refused")))
    }
}

#[tokio::test]
async fn infrastructure_failure_is_a_sanitised_internal_error() {
    let mut app = TestApp::spawn_with_repo(Arc::new(BrokenRepository)).await;
    let err = app
        .grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Internal);
    assert_eq!(err.message(), "internal error");
    for leaked in ["hunter2", "db.internal", "postgres://", "refused"] {
        assert!(!format!("{err:?}").contains(leaked), "{leaked} leaked: {err:?}");
    }
}
