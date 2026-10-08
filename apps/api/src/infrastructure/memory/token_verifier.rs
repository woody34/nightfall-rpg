use async_trait::async_trait;

use crate::application::ports::{AuthError, Claims};
use crate::application::TokenVerifier;

/// Accepts `test:<account_uuid>` and nothing else. For tests and, behind the explicit
/// `AUTH_DEV_TOKENS=1` opt-in, local development without Keycloak. Never for a deployment.
#[derive(Debug, Default, Clone, Copy)]
pub struct TestTokenVerifier;

impl TestTokenVerifier {
    /// The token that authenticates as `account`.
    #[must_use]
    pub fn token_for(account: uuid::Uuid) -> String {
        format!("test:{account}")
    }
}

#[async_trait]
impl TokenVerifier for TestTokenVerifier {
    async fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        let sub = token.strip_prefix("test:").ok_or(AuthError::Malformed)?;
        Ok(Claims {
            sub: sub.to_owned(),
            aud: vec!["nightfall-api".to_owned()],
            roles: vec!["player".to_owned()],
            exp: i64::MAX,
        })
    }
}
