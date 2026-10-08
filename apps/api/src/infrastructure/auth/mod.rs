//! Token verification adapters (Story 1.2).
//!
//! * [`KeycloakVerifier`]: RS256 JWTs from the configured OIDC issuer, keys from its JWKS.
//! * [`DisabledVerifier`]: rejects everything; used when no issuer is configured, so public
//!   RPCs (`Ping`) still work and nothing else does.

mod keycloak;

use async_trait::async_trait;

pub use keycloak::{
    HttpJwksSource, JwksSource, KeycloakVerifier, OidcConfig, LEEWAY_SECONDS, REFRESH_INTERVAL,
};

use crate::application::ports::{AuthError, Claims};
use crate::application::TokenVerifier;

/// Rejects every token.
#[derive(Debug, Default, Clone, Copy)]
pub struct DisabledVerifier;

#[async_trait]
impl TokenVerifier for DisabledVerifier {
    async fn verify(&self, _token: &str) -> Result<Claims, AuthError> {
        Err(AuthError::Invalid("authentication is not configured on this server"))
    }
}
