//! Authentication for the tonic server (Story 1.2, plan Revision 1 item 15).
//!
//! A tower layer rather than a tonic `Interceptor`: verification awaits (JWKS refresh, the
//! account upsert), and interceptors are synchronous. For every RPC except those in
//! [`PUBLIC_METHODS`] it reads `authorization: Bearer <token>`, runs [`Authenticate`], and
//! inserts the caller's [`AccountId`] into the request extensions, where handlers read it with
//! [`caller`]. A request without a valid token never reaches a handler.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::{header, HeaderMap, Request, Response};
use tonic::Status;
use tower::{Layer, Service};

use super::status::to_status;
use crate::application::use_cases::Authenticate;
use crate::domain::AccountId;
use crate::infrastructure::telemetry::record_account_id;

/// RPCs callable without a token, by gRPC path. Everything else requires one.
pub const PUBLIC_METHODS: &[&str] = &["/nightfall.v1.GameService/Ping"];

/// The authenticated caller of a request that passed [`AuthLayer`].
///
/// # Errors
/// `UNAUTHENTICATED` if the extension is missing, which only happens if a handler is mounted
/// without the layer: fail closed.
pub fn caller<T>(req: &tonic::Request<T>) -> Result<AccountId, Status> {
    req.extensions()
        .get::<AccountId>()
        .copied()
        .ok_or_else(|| Status::unauthenticated("authentication required"))
}

/// Tower layer producing [`Auth`].
#[derive(Clone)]
pub struct AuthLayer {
    authenticate: Arc<Authenticate>,
}

impl AuthLayer {
    /// Builds the layer around the authenticate use case.
    #[must_use]
    pub fn new(authenticate: Arc<Authenticate>) -> Self {
        Self { authenticate }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = Auth<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Auth {
            inner,
            authenticate: self.authenticate.clone(),
        }
    }
}

/// Service produced by [`AuthLayer`].
#[derive(Clone)]
pub struct Auth<S> {
    inner: S,
    authenticate: Arc<Authenticate>,
}

/// The token from `authorization: Bearer <token>` (scheme case-insensitive).
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

impl<S, B, RB> Service<Request<B>> for Auth<S>
where
    S: Service<Request<B>, Response = Response<RB>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
    RB: Default + Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<B>) -> Self::Future {
        // Take the instance that was polled ready; leave a fresh clone behind.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        if PUBLIC_METHODS.contains(&req.uri().path()) {
            return Box::pin(inner.call(req));
        }
        let token = bearer(req.headers()).map(str::to_owned);
        let authenticate = self.authenticate.clone();
        Box::pin(async move {
            let Some(token) = token else {
                return Ok(Status::unauthenticated("missing bearer token").into_http());
            };
            match authenticate.execute(&token).await {
                Ok(account) => {
                    record_account_id(account.as_uuid());
                    req.extensions_mut().insert(account);
                    inner.call(req).await
                },
                Err(e) => Ok(to_status(e).into_http()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(v).unwrap_or(HeaderValue::from_static("")),
        );
        h
    }

    #[test]
    fn bearer_parsing() {
        assert_eq!(bearer(&headers("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&headers("bearer abc")), Some("abc"));
        assert_eq!(bearer(&headers("Basic abc")), None);
        assert_eq!(bearer(&headers("Bearer ")), None);
        assert_eq!(bearer(&headers("Bearer")), None);
        assert_eq!(bearer(&HeaderMap::new()), None);
    }
}
