//! HTTP surface: health and readiness. Game traffic does not go over REST.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Json, Router};
use serde::Serialize;
use tower_http::cors::CorsLayer;

use crate::application::use_cases::TicketRejection;
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
            AppError::Unauthenticated(_) => StatusCode::UNAUTHORIZED,
            AppError::PermissionDenied(_) => StatusCode::FORBIDDEN,
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
            | AppError::Unauthenticated(_)
            | AppError::PermissionDenied(_)
            | AppError::AlreadyExists(_)
            | AppError::IdempotencyConflict => self.to_string(),
        };
        (status, Json(serde_json::json!({ "error": body }))).into_response()
    }
}

/// Maps a refused play ticket to the HTTP response sent *before* the WebSocket upgrade (plan
/// Revision 1, item 17): 401 for invalid, expired, or reused tickets; 409 when a newer ticket
/// for the same account superseded this one. Used by the `/ws` handler (Epic 4).
impl IntoResponse for TicketRejection {
    fn into_response(self) -> Response {
        let status = match &self {
            TicketRejection::Invalid | TicketRejection::Expired | TicketRejection::Consumed => {
                StatusCode::UNAUTHORIZED
            },
            TicketRejection::Superseded => StatusCode::CONFLICT,
            TicketRejection::Infrastructure(e) => {
                tracing::error!(error = ?e, "infrastructure error");
                StatusCode::INTERNAL_SERVER_ERROR
            },
        };
        let body = match &self {
            TicketRejection::Infrastructure(_) => "internal error".to_owned(),
            TicketRejection::Invalid
            | TicketRejection::Expired
            | TicketRejection::Consumed
            | TicketRejection::Superseded => self.to_string(),
        };
        (status, Json(serde_json::json!({ "error": body }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_errors_map_to_http_status() {
        let cases = [
            (AppError::InvalidArgument("x".into()), StatusCode::BAD_REQUEST),
            (AppError::Unauthenticated("x".into()), StatusCode::UNAUTHORIZED),
            (AppError::PermissionDenied("x".into()), StatusCode::FORBIDDEN),
            (AppError::IdempotencyConflict, StatusCode::CONFLICT),
            (
                AppError::Infrastructure(anyhow::anyhow!("x")),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (e, status) in cases {
            assert_eq!(e.into_response().status(), status);
        }
    }

    #[test]
    fn ticket_rejections_map_to_pre_upgrade_status() {
        let cases = [
            (TicketRejection::Invalid, StatusCode::UNAUTHORIZED),
            (TicketRejection::Expired, StatusCode::UNAUTHORIZED),
            (TicketRejection::Consumed, StatusCode::UNAUTHORIZED),
            (TicketRejection::Superseded, StatusCode::CONFLICT),
            (
                TicketRejection::Infrastructure(anyhow::anyhow!("x")),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (e, status) in cases {
            assert_eq!(e.into_response().status(), status);
        }
    }
}
