//! The authentication layer (Stories 1.2, 1.3): which RPCs need a token, what happens to bad
//! ones, and that a verified caller gets an account. Uses the real `KeycloakVerifier` against
//! a local JWKS endpoint serving the test keys in `tests/fixtures/jwt`.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use axum::routing::get;
use common::{account, TestApp};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use nightfall_api::domain::AccountId;
use nightfall_api::infrastructure::auth::{KeycloakVerifier, OidcConfig};
use nightfall_api::interface::grpc::pb::{ListMyCharactersRequest, PingRequest};
use serde_json::json;
use tonic::Code;
use uuid::Uuid;

const KEY_A: &str = include_str!("fixtures/jwt/key-a.pem");
const JWKS_A: &str = include_str!("fixtures/jwt/jwks-a.json");

#[tokio::test]
async fn ping_is_public_with_or_without_a_token() {
    let app = TestApp::spawn().await;
    app.anon_grpc().ping(PingRequest::default()).await.unwrap();
    app.game_with_token("garbage")
        .ping(PingRequest::default())
        .await
        .unwrap();
    assert!(app.accounts.is_empty(), "public RPCs do not touch accounts");
}

#[tokio::test]
async fn first_authenticated_call_creates_the_account_once() {
    let mut app = TestApp::spawn().await;
    for _ in 0..3 {
        app.grpc
            .list_my_characters(ListMyCharactersRequest {})
            .await
            .unwrap();
    }
    assert_eq!(app.accounts.len(), 1);
    assert!(app.accounts.contains(AccountId::from_uuid(account())));
}

#[tokio::test]
async fn non_bearer_scheme_is_unauthenticated() {
    let app = TestApp::spawn().await;
    let mut client =
        nightfall_api::interface::grpc::pb::game_service_client::GameServiceClient::connect(
            // Reuse the server through a raw client so the header is exactly what we send.
            app.grpc_base.clone(),
        )
        .await
        .unwrap();
    let mut req = tonic::Request::new(ListMyCharactersRequest {});
    req.metadata_mut()
        .insert("authorization", "Basic dGVzdDp0ZXN0".parse().unwrap());
    let err = client.list_my_characters(req).await.unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn unknown_methods_require_a_token_too() {
    let app = TestApp::spawn().await;
    let mut client = tonic::client::Grpc::new(
        tonic::transport::Channel::from_shared(app.grpc_base.clone())
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    client.ready().await.unwrap();
    let res: Result<tonic::Response<PingRequest>, _> = client
        .unary(
            tonic::Request::new(PingRequest::default()),
            "/nightfall.v1.GameService/NotARealMethod".parse().unwrap(),
            tonic_prost::ProstCodec::<PingRequest, PingRequest>::default(),
        )
        .await;
    assert_eq!(res.unwrap_err().code(), Code::Unauthenticated, "fail closed, no route probing");
}

/// Serves `jwks` at the Keycloak certs path; returns the issuer URL.
async fn jwks_server(jwks: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/realms/nightfall/protocol/openid-connect/certs",
        get(move || async move { ([("content-type", "application/json")], jwks) }),
    );
    tokio::spawn(async move { axum::serve(listener, router).await });
    format!("http://{addr}/realms/nightfall")
}

fn signed(claims: &serde_json::Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-a".into());
    encode(&header, claims, &EncodingKey::from_rsa_pem(KEY_A.as_bytes()).unwrap()).unwrap()
}

async fn keycloak_app() -> (TestApp, String) {
    let issuer = jwks_server(JWKS_A).await;
    let verifier = KeycloakVerifier::connect(&OidcConfig {
        issuer: issuer.clone(),
        audience: "nightfall-api".into(),
    })
    .await
    .unwrap();
    assert_eq!(verifier.key_count(), 1, "JWKS preloaded at boot");
    let app = TestApp::spawn_with(|deps| deps.tokens = Arc::new(verifier)).await;
    (app, issuer)
}

#[tokio::test]
async fn signed_jwt_authenticates_through_the_socket() {
    let (app, issuer) = keycloak_app().await;
    let sub = Uuid::now_v7();
    let now = chrono::Utc::now().timestamp();
    let token = signed(&json!({
        "iss": issuer, "aud": ["nightfall-api", "account"], "sub": sub.to_string(),
        "exp": now + 300, "iat": now,
    }));

    let res = app
        .game_with_token(&token)
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap();
    assert!(res.into_inner().characters.is_empty());
    assert!(app.accounts.contains(AccountId::from_uuid(sub)), "account = sub");
}

#[tokio::test]
async fn expired_wrong_audience_and_wrong_issuer_jwts_are_unauthenticated() {
    let (app, issuer) = keycloak_app().await;
    let now = chrono::Utc::now().timestamp();
    let sub = Uuid::now_v7().to_string();
    let bad = [
        json!({ "iss": issuer, "aud": "nightfall-api", "sub": sub, "exp": now - 120 }),
        json!({ "iss": issuer, "aud": "account", "sub": sub, "exp": now + 300 }),
        json!({ "iss": "http://evil.example/realms/nightfall", "aud": "nightfall-api",
                "sub": sub, "exp": now + 300 }),
        json!({ "iss": issuer, "aud": "nightfall-api", "sub": "not-a-uuid", "exp": now + 300 }),
    ];
    for claims in bad {
        let err = app
            .game_with_token(&signed(&claims))
            .list_my_characters(ListMyCharactersRequest {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::Unauthenticated, "{claims}");
    }
    assert!(app.accounts.is_empty(), "no account for a rejected token");
}
