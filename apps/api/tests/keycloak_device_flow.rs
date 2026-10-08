//! End to end against the Compose Keycloak (plan Revision 1, item 15). Runs only when
//! `KEYCLOAK_URL` is set (e.g. `http://localhost:8080`); needs `bash`, `curl`, `jq`, `openssl`.
//!
//! Obtains a real access token for `testplayer` through the device authorization grant,
//! automated by `infra/keycloak/device-flow-demo.sh`, then calls authenticated RPCs on a
//! server whose verifier fetched the realm's JWKS over HTTP.

#![allow(
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::TestApp;
use nightfall_api::domain::AccountId;
use nightfall_api::infrastructure::auth::{KeycloakVerifier, OidcConfig};
use nightfall_api::interface::grpc::pb::{
    self, CreateCharacterRequest, GetCharacterRequest, IssuePlayTicketRequest,
    ListMyCharactersRequest,
};
use tonic::Code;
use uuid::Uuid;

async fn device_flow_token(issuer: &str) -> String {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../infra/keycloak/device-flow-demo.sh");
    let out = tokio::process::Command::new("bash")
        .arg(script)
        .env("OIDC_ISSUER", issuer)
        .env("OUTPUT", "access_token")
        .output()
        .await
        .unwrap();
    assert!(
        out.status.success(),
        "device flow failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

fn subject(token: &str) -> Uuid {
    use base64::Engine as _;
    let payload = token.split('.').nth(1).unwrap();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    Uuid::parse_str(claims["sub"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn real_keycloak_token_authenticates_character_and_ticket_rpcs() {
    let Ok(base) = std::env::var("KEYCLOAK_URL") else {
        return;
    };
    let issuer = format!("{}/realms/nightfall", base.trim_end_matches('/'));
    let token = device_flow_token(&issuer).await;
    let sub = subject(&token);

    let verifier = KeycloakVerifier::connect(&OidcConfig {
        issuer,
        audience: "nightfall-api".into(),
    })
    .await
    .unwrap();
    assert!(verifier.key_count() >= 1, "realm JWKS fetched at boot");
    let app = TestApp::spawn_with(|deps| deps.tokens = Arc::new(verifier)).await;

    let mut game = app.game_with_token(&token);
    let created = game
        .create_character(CreateCharacterRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            name: "Keycloaker".into(),
            race: pb::Race::Human as i32,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(app.accounts.contains(AccountId::from_uuid(sub)), "account = token sub");

    let mine = game
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap()
        .into_inner()
        .characters;
    assert_eq!(mine, vec![created.clone()]);
    game.get_character(GetCharacterRequest {
        character_id: created.id.clone(),
    })
    .await
    .unwrap();

    let ticket = app
        .session_with_token(&token)
        .issue_play_ticket(IssuePlayTicketRequest {
            idempotency_key: Uuid::now_v7().to_string(),
            character_id: created.id,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ticket.ticket.len(), 43);

    // One flipped signature character is enough to be refused.
    let mut tampered = token.clone().into_bytes();
    let last = tampered.len() - 2;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    let err = app
        .game_with_token(std::str::from_utf8(&tampered).unwrap())
        .list_my_characters(ListMyCharactersRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}
