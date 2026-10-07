//! HTTP surface: health and readiness. Game traffic does not go over REST.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::application::AppError;

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

/// Builds the router. Takes no dependencies today; use cases will be injected as `State`.
pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
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
