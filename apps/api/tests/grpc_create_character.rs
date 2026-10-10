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
use common::{account, other_account, TestApp, KEY_A, KEY_B, OTHER_ACCOUNT};
use nightfall_api::application::ports::RepositoryError;
use nightfall_api::application::{CharacterRepository, CreateOutcome, IdempotencyKey};
use nightfall_api::domain::{AccountId, Character, CharacterId, DomainEvent};
use nightfall_api::interface::grpc::pb::{self, CreateCharacterRequest, GetCharacterRequest};
use tonic::Code;

fn req(key: &str, name: &str) -> CreateCharacterRequest {
    CreateCharacterRequest {
        idempotency_key: key.into(),
        name: name.into(),
        race: pb::Race::Orc as i32,
        ..Default::default()
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
    assert!(matches!(
        events[0],
        DomainEvent::CharacterCreated { account_id, .. } if account_id == AccountId::from_uuid(account())
    ));
}

#[tokio::test]
async fn owner_is_the_caller_and_a_forged_account_id_is_ignored() {
    let mut app = TestApp::spawn().await;
    #[allow(deprecated)] // the point of the test: the deprecated field must have no effect
    let forged = CreateCharacterRequest {
        account_id: OTHER_ACCOUNT.into(),
        ..req(KEY_A, "Thrall")
    };
    let created = app
        .grpc
        .create_character(forged)
        .await
        .unwrap()
        .into_inner();

    let id: CharacterId = created.id.parse().unwrap();
    let stored = app.characters.get_for_test(id).unwrap();
    assert_eq!(stored.account_id, AccountId::from_uuid(account()));

    // The account named in the forged field cannot see or read it.
    let mut other = app.game_as(other_account());
    let err = other
        .get_character(GetCharacterRequest {
            character_id: created.id,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
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
async fn same_key_from_another_account_is_a_different_request() {
    let mut app = TestApp::spawn().await;
    app.grpc
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap();
    let theirs = app
        .game_as(other_account())
        .create_character(req(KEY_A, "Garrosh"))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(theirs.name, "Garrosh");
    assert_eq!(app.characters.len(), 2);
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

    async fn list_by_account(&self, _account: AccountId) -> anyhow::Result<Vec<Character>> {
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

    async fn load_for_admission(
        &self,
        _id: nightfall_api::domain::CharacterId,
    ) -> anyhow::Result<Option<nightfall_api::application::ProgressionState>> {
        anyhow::bail!("down")
    }

    async fn checkpoint(
        &self,
        _checkpoint: &nightfall_api::application::CharacterCheckpoint,
        _events: &[nightfall_api::domain::DomainEvent],
    ) -> Result<
        nightfall_api::application::CheckpointOutcome,
        nightfall_api::application::CheckpointError,
    > {
        Err(nightfall_api::application::CheckpointError::Other(anyhow::anyhow!("down")))
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

#[tokio::test]
async fn without_a_token_is_unauthenticated_and_writes_nothing() {
    let app = TestApp::spawn().await;
    let err = app
        .anon_grpc()
        .create_character(req(KEY_A, "Thrall"))
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
    assert_eq!(app.characters.len(), 0);
    assert!(app.accounts.is_empty());
}

#[tokio::test]
async fn optional_base_class_presence_preserves_legacy_defaults_and_explicit_zero() {
    let mut app = TestApp::spawn().await;
    let mut elf = req(KEY_A, "Elvenhero");
    elf.race = pb::Race::Elf as i32;
    let created = app
        .grpc
        .create_character(elf.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (created.class_id, created.base_class_id, created.sex),
        (18, 18, pb::Sex::Male as i32)
    );
    assert_eq!(
        created.classes,
        vec![pb::ClassProgress {
            slot: 0,
            class_id: 18,
            level: 1,
            exp: 0,
            sp: 0
        }]
    );
    elf.base_class_id = Some(0);
    elf.idempotency_key = KEY_B.into();
    assert_eq!(app.grpc.create_character(elf).await.unwrap_err().code(), Code::InvalidArgument);
    let mut human = req(KEY_B, "Humanhero");
    human.race = pb::Race::Human as i32;
    let omitted = app
        .grpc
        .create_character(human.clone())
        .await
        .unwrap()
        .into_inner();
    human.base_class_id = Some(0);
    assert_eq!(
        app.grpc.create_character(human).await.unwrap().into_inner(),
        omitted,
        "same effective legacy defaults replay"
    );
    assert_eq!(omitted.class_id, 0);
    assert_eq!(app.characters.len(), 2);
}

#[tokio::test]
async fn mystic_appearance_and_creation_failures_are_checked_through_the_wire() {
    let mut app = TestApp::spawn().await;
    let mut request = req(KEY_A, "Mystichero");
    request.race = pb::Race::Human as i32;
    request.base_class_id = Some(10);
    request.sex = pb::Sex::Female as i32;
    let created = app
        .grpc
        .create_character(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (created.class_id, created.base_class_id, created.sex),
        (10, 10, pb::Sex::Female as i32)
    );
    assert_eq!(
        created.stats,
        Some(pb::BaseStats {
            str: 22,
            dex: 21,
            con: 27,
            int: 41,
            wit: 20,
            men: 39
        })
    );
    assert_eq!((created.hair_style, created.hair_color, created.face), (0, 0, 0));
    request.sex = pb::Sex::Male as i32;
    assert_eq!(
        app.grpc
            .create_character(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    for (sex, style, color, face) in [(99, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)] {
        request.idempotency_key = KEY_B.into();
        request.sex = sex;
        request.hair_style = style;
        request.hair_color = color;
        request.face = face;
        assert_eq!(
            app.grpc
                .create_character(request.clone())
                .await
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
    }
    assert_eq!(app.characters.len(), 1);
    assert_eq!(app.characters.staged_events().len(), 1);
}

#[tokio::test]
async fn seven_character_slots_refuse_new_keys_and_replay_old_keys() {
    let mut app = TestApp::spawn().await;
    let first = app
        .grpc
        .create_character(req(KEY_A, "Firsthero"))
        .await
        .unwrap()
        .into_inner();
    for suffix in ['a', 'b', 'c', 'd', 'e', 'f'] {
        app.grpc
            .create_character(req(&uuid::Uuid::now_v7().to_string(), &format!("Hero{suffix}")))
            .await
            .unwrap();
    }
    assert_eq!(
        app.grpc
            .create_character(req(KEY_B, "Eighthhero"))
            .await
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    assert_eq!(
        app.grpc
            .create_character(req(KEY_A, "Firsthero"))
            .await
            .unwrap()
            .into_inner(),
        first
    );
    assert_eq!(app.characters.len(), 7);
    assert_eq!(app.characters.staged_events().len(), 7);
}

#[tokio::test]
async fn configured_curated_blocklist_rejects_a_name_without_writes() {
    let mut app = TestApp::spawn_with(|deps| {
        deps.blocked_character_names =
            Arc::new(std::collections::BTreeSet::from(["blockedhero".into()]))
    })
    .await;
    assert_eq!(
        app.grpc
            .create_character(req(KEY_A, "Blockedhero"))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert!(app.characters.is_empty());
    assert!(app.characters.staged_events().is_empty());
}
