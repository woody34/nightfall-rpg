//! Request correlation and request metrics for both servers.
//!
//! * HTTP: a uuid v7 `x-request-id` (set, put on the trace span, echoed on the response) and
//!   `http_requests_total{route,status}` keyed by the matched route template.
//! * gRPC: a tower layer that opens one span per RPC (`rpc.service`, `rpc.method`,
//!   `request_id`, and `account_id` once a handler records it) and counts
//!   `grpc_requests_total{service,method,code}`.
//!
//! The gRPC piece is a layer rather than a tonic `Interceptor` because an interceptor sees only
//! the request: it cannot hold a span open across the handler or observe the status code.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::trace::TraceLayer;
use tracing::{field, Instrument, Span};

use super::Metrics;

/// The header carrying the correlation id.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Generates time-ordered uuid v7 request ids.
#[derive(Debug, Clone, Copy, Default)]
pub struct UuidV7RequestId;

impl MakeRequestId for UuidV7RequestId {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        let id = uuid::Uuid::now_v7().to_string();
        HeaderValue::from_str(&id).ok().map(RequestId::new)
    }
}

/// The pair of layers that assign a request id and copy it onto the response.
///
/// Order matters: `Set` must be outermost so the trace span (inside it) can read the id, and
/// `Propagate` sits inside `Set` so the header it copies already exists.
#[must_use]
pub fn request_id_layers() -> (SetRequestIdLayer<UuidV7RequestId>, PropagateRequestIdLayer) {
    (
        SetRequestIdLayer::new(header::HeaderName::from_static(REQUEST_ID_HEADER), UuidV7RequestId),
        PropagateRequestIdLayer::new(header::HeaderName::from_static(REQUEST_ID_HEADER)),
    )
}

fn request_id_of(headers: &axum::http::HeaderMap) -> &str {
    headers
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

/// `TraceLayer` whose root span carries `request_id`.
#[must_use]
pub fn http_trace_layer<B>() -> TraceLayer<
    SharedClassifier<ServerErrorsAsFailures>,
    impl Fn(&axum::http::Request<B>) -> Span + Clone,
> {
    TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<B>| {
        tracing::info_span!(
            "http.request",
            otel.kind = "server",
            request_id = request_id_of(req.headers()),
            http.method = %req.method(),
            http.path = %req.uri().path(),
            http.status_code = field::Empty,
        )
    })
}

/// axum middleware: counts the request in `http_requests_total{route,status}`.
pub async fn http_metrics(State(metrics): State<Metrics>, req: Request, next: Next) -> Response {
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |m| m.as_str().to_owned());
    let res = next.run(req).await;
    Span::current().record("http.status_code", res.status().as_u16());
    metrics.record_http(&route, res.status().as_u16());
    res
}

/// `GET /metrics`: Prometheus text exposition.
pub async fn metrics_handler(State(metrics): State<Metrics>) -> Response {
    match metrics.render() {
        Ok(body) => ([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")], body)
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "metrics render failed");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
        },
    }
}

/// Attaches the authenticated account to the current RPC span. Call it from a handler once the
/// account is known; a no-op outside a span that declares `account_id`.
pub fn record_account_id(account_id: uuid::Uuid) {
    Span::current().record("account_id", field::display(account_id));
}

/// Tower layer for the tonic server. See the module docs.
#[derive(Clone)]
pub struct GrpcTelemetryLayer {
    metrics: Metrics,
}

impl GrpcTelemetryLayer {
    /// Builds the layer around the shared metrics.
    #[must_use]
    pub fn new(metrics: Metrics) -> Self {
        Self { metrics }
    }
}

impl<S> Layer<S> for GrpcTelemetryLayer {
    type Service = GrpcTelemetry<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcTelemetry {
            inner,
            metrics: self.metrics.clone(),
        }
    }
}

/// Service produced by [`GrpcTelemetryLayer`].
#[derive(Clone)]
pub struct GrpcTelemetry<S> {
    inner: S,
    metrics: Metrics,
}

impl<S, B, RB> Service<axum::http::Request<B>> for GrpcTelemetry<S>
where
    S: Service<axum::http::Request<B>, Response = axum::http::Response<RB>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
    RB: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<B>) -> Self::Future {
        // Take the instance that was polled ready; leave a fresh clone behind.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let metrics = self.metrics.clone();

        let path = req.uri().path().trim_start_matches('/');
        let (service, method) = path.rsplit_once('/').unwrap_or(("unknown", path));
        let (service, method) = (service.to_owned(), method.to_owned());
        let span = tracing::info_span!(
            "grpc.request",
            otel.kind = "server",
            otel.name = %format!("{service}/{method}"),
            rpc.system = "grpc",
            rpc.service = %service,
            rpc.method = %method,
            request_id = request_id_of(req.headers()),
            account_id = field::Empty,
            rpc.grpc.status_code = field::Empty,
        );

        Box::pin(
            {
                let span = span.clone();
                async move {
                    let res = inner.call(req).await?;
                    // Unary errors are "trailers-only" responses with `grpc-status` in the
                    // headers; success carries it in trailers, so an absent header means OK.
                    let code = res
                        .headers()
                        .get("grpc-status")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<i32>().ok())
                        .map_or(tonic::Code::Ok, tonic::Code::from);
                    let code = format!("{code:?}");
                    span.record("rpc.grpc.status_code", code.as_str());
                    metrics.record_grpc(&service, &method, &code);
                    Ok(res)
                }
            }
            .instrument(span),
        )
    }
}
