//! JWT verification against the Keycloak realm (plan Revision 1, item 15).
//!
//! * Keys come from `OIDC_ISSUER/protocol/openid-connect/certs`, fetched once at boot.
//! * A token whose `kid` is not cached triggers a refresh, at most once per
//!   [`REFRESH_INTERVAL`], so key rotation is picked up without letting garbage tokens turn
//!   the server into a JWKS-fetching amplifier.
//! * RS256 only; `iss`, `aud`, `exp`, `sub` required; [`LEEWAY_SECONDS`] of clock skew.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, PublicKeyUse};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use parking_lot::RwLock;
use serde::Deserialize;
use tokio::time::Instant;

use crate::application::ports::{AuthError, Claims};
use crate::application::TokenVerifier;

/// Minimum time between two JWKS fetches.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// Accepted clock skew on `exp` and `nbf`.
pub const LEEWAY_SECONDS: u64 = 30;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Which issuer and audience to trust. From `OIDC_ISSUER` and `OIDC_AUDIENCE`.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Exact `iss` value, e.g. `http://localhost:8080/realms/nightfall`.
    pub issuer: String,
    /// Required member of `aud`, e.g. `nightfall-api`.
    pub audience: String,
}

/// Where signing keys come from. HTTP in production; a fixed set in tests.
#[async_trait]
pub trait JwksSource: Send + Sync {
    /// Fetches the issuer's current key set.
    async fn fetch(&self) -> anyhow::Result<JwkSet>;
}

/// Fetches the JWKS from the issuer's well-known Keycloak path.
pub struct HttpJwksSource {
    client: reqwest::Client,
    url: String,
}

impl HttpJwksSource {
    /// Builds the source for `issuer`.
    pub fn new(issuer: &str) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().timeout(FETCH_TIMEOUT).build()?,
            url: format!("{}/protocol/openid-connect/certs", issuer.trim_end_matches('/')),
        })
    }
}

#[async_trait]
impl JwksSource for HttpJwksSource {
    async fn fetch(&self) -> anyhow::Result<JwkSet> {
        Ok(self
            .client
            .get(&self.url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
}

/// [`TokenVerifier`] for Keycloak-issued access tokens.
pub struct KeycloakVerifier {
    validation: Validation,
    source: Arc<dyn JwksSource>,
    keys: RwLock<HashMap<String, DecodingKey>>,
    /// When the last fetch started. Held across the fetch, so concurrent misses wait for one
    /// fetch instead of each starting their own.
    last_fetch: tokio::sync::Mutex<Option<Instant>>,
}

impl KeycloakVerifier {
    /// Builds the verifier over HTTP and preloads the keys.
    pub async fn connect(cfg: &OidcConfig) -> anyhow::Result<Self> {
        Ok(Self::with_source(cfg, Arc::new(HttpJwksSource::new(&cfg.issuer)?)).await)
    }

    /// Builds the verifier over any key source and preloads the keys. A failed preload is
    /// logged, not fatal: the first request retries (subject to [`REFRESH_INTERVAL`]), so the
    /// server can start before Keycloak does.
    pub async fn with_source(cfg: &OidcConfig, source: Arc<dyn JwksSource>) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.algorithms = vec![Algorithm::RS256];
        validation.set_issuer(&[&cfg.issuer]);
        validation.set_audience(&[&cfg.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = LEEWAY_SECONDS;
        validation.validate_exp = true;
        validation.validate_nbf = true;

        let verifier = Self {
            validation,
            source,
            keys: RwLock::new(HashMap::new()),
            last_fetch: tokio::sync::Mutex::new(None),
        };
        let mut last = verifier.last_fetch.lock().await;
        match verifier.refresh(&mut last).await {
            Ok(n) => tracing::info!(keys = n, issuer = %cfg.issuer, "JWKS loaded"),
            Err(e) => tracing::error!(error = %e, issuer = %cfg.issuer, "JWKS preload failed"),
        }
        drop(last);
        verifier
    }

    /// Number of usable signing keys cached.
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.keys.read().len()
    }

    /// Fetches and replaces the key cache. Caller holds the `last_fetch` lock.
    async fn refresh(&self, last: &mut Option<Instant>) -> anyhow::Result<usize> {
        *last = Some(Instant::now());
        let set = self.source.fetch().await?;
        let keys = signing_keys(&set);
        let n = keys.len();
        *self.keys.write() = keys;
        Ok(n)
    }

    fn cached(&self, kid: &str) -> Option<DecodingKey> {
        self.keys.read().get(kid).cloned()
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let Some(key) = self.cached(kid) {
            return Ok(key);
        }
        let mut last = self.last_fetch.lock().await;
        // Another request may have refreshed while this one waited.
        if let Some(key) = self.cached(kid) {
            return Ok(key);
        }
        if last.is_some_and(|t| t.elapsed() < REFRESH_INTERVAL) {
            return Err(AuthError::UnknownKey);
        }
        match self.refresh(&mut last).await {
            Ok(n) => tracing::info!(keys = n, "JWKS refreshed for an unknown key id"),
            Err(e) => {
                tracing::warn!(error = %e, "JWKS refresh failed");
                return Err(AuthError::Unavailable(e));
            },
        }
        self.cached(kid).ok_or(AuthError::UnknownKey)
    }
}

/// RS256 signature keys with a `kid`; encryption keys and other algorithms are skipped
/// (Keycloak publishes an `RSA-OAEP` encryption key in the same set).
fn signing_keys(set: &JwkSet) -> HashMap<String, DecodingKey> {
    set.keys
        .iter()
        .filter(|k| !matches!(k.common.public_key_use, Some(PublicKeyUse::Encryption)))
        .filter(|k| matches!(k.common.key_algorithm, None | Some(KeyAlgorithm::RS256)))
        .filter(|k| matches!(k.algorithm, AlgorithmParameters::RSA(_)))
        .filter_map(|k| {
            let kid = k.common.key_id.clone()?;
            match DecodingKey::from_jwk(k) {
                Ok(key) => Some((kid, key)),
                Err(e) => {
                    tracing::warn!(kid, error = %e, "skipping unusable JWK");
                    None
                },
            }
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Default)]
struct RealmAccess {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct RawClaims {
    sub: String,
    aud: Audience,
    exp: i64,
    #[serde(default)]
    realm_access: RealmAccess,
}

// `ErrorKind` is `#[non_exhaustive]`, so a wildcard arm is unavoidable.
#[allow(clippy::wildcard_enum_match_arm)]
fn map_error(e: &jsonwebtoken::errors::Error) -> AuthError {
    match e.kind() {
        ErrorKind::ExpiredSignature => AuthError::Expired,
        ErrorKind::InvalidIssuer => AuthError::Invalid("wrong issuer"),
        ErrorKind::InvalidAudience => AuthError::Invalid("wrong audience"),
        ErrorKind::InvalidSignature => AuthError::Invalid("bad signature"),
        ErrorKind::InvalidAlgorithm => AuthError::Invalid("algorithm must be RS256"),
        ErrorKind::ImmatureSignature => AuthError::Invalid("token not yet valid"),
        ErrorKind::MissingRequiredClaim(_) => AuthError::Invalid("missing required claim"),
        ErrorKind::InvalidToken
        | ErrorKind::Base64(_)
        | ErrorKind::Json(_)
        | ErrorKind::Utf8(_) => AuthError::Malformed,
        _ => AuthError::Invalid("token rejected"),
    }
}

#[async_trait]
impl TokenVerifier for KeycloakVerifier {
    async fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::Malformed)?;
        // Checked before any key lookup so an HS256 token can never be verified with an RSA
        // public key used as an HMAC secret.
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::Invalid("algorithm must be RS256"));
        }
        let kid = header.kid.ok_or(AuthError::Invalid("missing key id"))?;
        let key = self.key_for(&kid).await?;
        let data = decode::<RawClaims>(token, &key, &self.validation).map_err(|e| map_error(&e))?;
        let c = data.claims;
        Ok(Claims {
            sub: c.sub,
            aud: match c.aud {
                Audience::One(a) => vec![a],
                Audience::Many(a) => a,
            },
            roles: c.realm_access.roles,
            exp: c.exp,
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    use super::*;

    const KEY_A: &str = include_str!("../../../tests/fixtures/jwt/key-a.pem");
    const KEY_B: &str = include_str!("../../../tests/fixtures/jwt/key-b.pem");
    const JWKS_A: &str = include_str!("../../../tests/fixtures/jwt/jwks-a.json");
    const JWKS_AB: &str = include_str!("../../../tests/fixtures/jwt/jwks-ab.json");
    const ISS: &str = "http://localhost:8080/realms/nightfall";
    const AUD: &str = "nightfall-api";
    const SUB: &str = "fb71069a-b366-48df-9a4f-7093b3ae7da7";

    /// A key source whose set can be swapped, counting fetches.
    struct FakeSource {
        set: parking_lot::Mutex<&'static str>,
        fetches: AtomicUsize,
    }

    impl FakeSource {
        fn new(set: &'static str) -> Arc<Self> {
            Arc::new(Self {
                set: parking_lot::Mutex::new(set),
                fetches: AtomicUsize::new(0),
            })
        }

        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl JwksSource for FakeSource {
        async fn fetch(&self) -> anyhow::Result<JwkSet> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::from_str(*self.set.lock())?)
        }
    }

    fn cfg() -> OidcConfig {
        OidcConfig {
            issuer: ISS.into(),
            audience: AUD.into(),
        }
    }

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn claims() -> serde_json::Value {
        json!({
            "iss": ISS,
            "aud": [AUD, "account"],
            "sub": SUB,
            "exp": now() + 300,
            "iat": now(),
            "realm_access": { "roles": ["player", "offline_access"] },
        })
    }

    fn sign(alg: Algorithm, kid: &str, pem: &str, claims: &serde_json::Value) -> String {
        let mut header = Header::new(alg);
        header.kid = Some(kid.into());
        encode(&header, claims, &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()).unwrap()
    }

    fn token_a(claims: &serde_json::Value) -> String {
        sign(Algorithm::RS256, "test-a", KEY_A, claims)
    }

    async fn verifier() -> (KeycloakVerifier, Arc<FakeSource>) {
        let source = FakeSource::new(JWKS_A);
        (KeycloakVerifier::with_source(&cfg(), source.clone()).await, source)
    }

    #[tokio::test]
    async fn valid_token_yields_claims() {
        let (v, _) = verifier().await;
        let c = v.verify(&token_a(&claims())).await.unwrap();
        assert_eq!(c.sub, SUB);
        assert_eq!(c.aud, vec![AUD.to_owned(), "account".to_owned()]);
        assert_eq!(c.roles, vec!["player".to_owned(), "offline_access".to_owned()]);
    }

    #[tokio::test]
    async fn single_string_audience_is_accepted() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c["aud"] = json!(AUD);
        assert_eq!(v.verify(&token_a(&c)).await.unwrap().aud, vec![AUD.to_owned()]);
    }

    #[tokio::test]
    async fn encryption_keys_are_not_cached() {
        let (v, _) = verifier().await;
        assert_eq!(v.key_count(), 1, "jwks-a has one signing key and one enc key");
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c["exp"] = json!(now() - 120);
        assert!(matches!(v.verify(&token_a(&c)).await, Err(AuthError::Expired)));
    }

    #[tokio::test]
    async fn expiry_within_leeway_is_accepted() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c["exp"] = json!(now() - 10);
        v.verify(&token_a(&c)).await.unwrap();
    }

    #[tokio::test]
    async fn wrong_audience_is_rejected() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c["aud"] = json!(["account"]);
        let err = v.verify(&token_a(&c)).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid("wrong audience")), "{err:?}");
    }

    #[tokio::test]
    async fn wrong_issuer_is_rejected() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c["iss"] = json!("http://evil.example/realms/nightfall");
        let err = v.verify(&token_a(&c)).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid("wrong issuer")), "{err:?}");
    }

    #[tokio::test]
    async fn missing_subject_is_rejected() {
        let (v, _) = verifier().await;
        let mut c = claims();
        c.as_object_mut().unwrap().remove("sub");
        assert!(v.verify(&token_a(&c)).await.is_err());
    }

    #[tokio::test]
    async fn other_algorithms_are_rejected() {
        let (v, _) = verifier().await;
        // Same RSA key, different digest.
        let rs384 = sign(Algorithm::RS384, "test-a", KEY_A, &claims());
        let err = v.verify(&rs384).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid("algorithm must be RS256")), "{err:?}");

        // The classic confusion attack: HS256 keyed with the public key material.
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("test-a".into());
        let hs256 =
            encode(&header, &claims(), &EncodingKey::from_secret(JWKS_A.as_bytes())).unwrap();
        let err = v.verify(&hs256).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid("algorithm must be RS256")), "{err:?}");

        // alg=none is not even a parseable header.
        let none = format!(
            "{}.{}.",
            base64_url(r#"{"alg":"none","kid":"test-a"}"#),
            base64_url(&claims().to_string())
        );
        assert!(v.verify(&none).await.is_err());
    }

    fn base64_url(s: &str) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
    }

    #[tokio::test]
    async fn tampered_signature_is_rejected() {
        let (v, _) = verifier().await;
        // Signed by key B but claiming to be key A.
        let forged = sign(Algorithm::RS256, "test-a", KEY_B, &claims());
        let err = v.verify(&forged).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid("bad signature")), "{err:?}");
    }

    #[tokio::test]
    async fn garbage_is_malformed() {
        let (v, _) = verifier().await;
        assert!(matches!(v.verify("not-a-jwt").await, Err(AuthError::Malformed)));
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_kid_refreshes_at_most_once_per_interval() {
        let (v, source) = verifier().await;
        assert_eq!(source.fetches(), 1, "preloaded at boot");
        let token_b = sign(Algorithm::RS256, "test-b", KEY_B, &claims());

        // Within the interval of the boot fetch: no refresh, even after the issuer rotates.
        *source.set.lock() = JWKS_AB;
        assert!(matches!(v.verify(&token_b).await, Err(AuthError::UnknownKey)));
        assert_eq!(source.fetches(), 1);

        // After the interval, one refresh picks up the rotated key.
        tokio::time::advance(REFRESH_INTERVAL).await;
        v.verify(&token_b).await.unwrap();
        assert_eq!(source.fetches(), 2);

        // A flood of unknown kids right after costs no further fetches.
        let token_c = sign(Algorithm::RS256, "test-c", KEY_B, &claims());
        for _ in 0..10 {
            assert!(matches!(v.verify(&token_c).await, Err(AuthError::UnknownKey)));
        }
        assert_eq!(source.fetches(), 2);
        // Known keys keep working throughout.
        v.verify(&token_a(&claims())).await.unwrap();
    }

    struct DownSource;

    #[async_trait]
    impl JwksSource for DownSource {
        async fn fetch(&self) -> anyhow::Result<JwkSet> {
            anyhow::bail!("connection refused")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unreachable_issuer_is_unavailable_then_throttled() {
        let v = KeycloakVerifier::with_source(&cfg(), Arc::new(DownSource)).await;
        assert_eq!(v.key_count(), 0);
        // Boot fetch just failed: within the interval the token is simply unknown.
        assert!(matches!(v.verify(&token_a(&claims())).await, Err(AuthError::UnknownKey)));
        tokio::time::advance(REFRESH_INTERVAL).await;
        assert!(matches!(v.verify(&token_a(&claims())).await, Err(AuthError::Unavailable(_))));
    }
}
