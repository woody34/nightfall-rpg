//! HTTP surface: health and readiness. Game traffic does not go over REST.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Json, Router};
use serde::Serialize;
use tower_http::cors::CorsLayer;

use crate::application::AppError;
use crate::infrastructure::telemetry::{
    http_metrics, http_trace_layer, metrics_handler, request_id_layers, Metrics,
};

/// Body of `GET /health`.
#[derive(Debug, Serialize)]
pub struct Health {
    /// Always `"ok"` when the process can answer.
    pub status: &'static str,
    /// Crate version.
    pub version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Builds the router: `/health`, and `/metrics` in Prometheus text format.
///
/// Layers, outermost first: request id, trace span, id propagation, CORS, request metrics.
pub fn router(metrics: Metrics) -> Router {
    let (set_request_id, propagate_request_id) = request_id_layers();
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .layer(middleware::from_fn_with_state(metrics.clone(), http_metrics))
        .layer(CorsLayer::permissive())
        .layer(propagate_request_id)
        .layer(http_trace_layer())
        .layer(set_request_id)
        .with_state(metrics)
}

/// Maps application errors to HTTP status codes. Used once REST endpoints exist.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::InvalidArgument(_) => StatusCode::BAD_REQUEST,
            AppError::NotFound { .. } => StatusCode::NOT_FOUND,
            AppError::AlreadyExists(_) | AppError::IdempotencyConflict => StatusCode::CONFLICT,
            AppError::Infrastructure(e) => {
                tracing::error!(error = ?e, "infrastructure error");
                StatusCode::INTERNAL_SERVER_ERROR
            },
        };
        let body = match &self {
            AppError::Infrastructure(_) => "internal error".to_owned(),
            AppError::InvalidArgument(_)
            | AppError::NotFound { .. }
            | AppError::AlreadyExists(_)
            | AppError::IdempotencyConflict => self.to_string(),
        };
        (status, Json(serde_json::json!({ "error": body }))).into_response()
    }
}
