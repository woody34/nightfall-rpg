//! Application error type. Interface adapters map this to gRPC status codes and HTTP status.

use std::str::FromStr;

use crate::domain::character::NameError;

/// Every way a use case can fail. Variants are deliberately coarse: they are the contract with
/// the transport layer, which maps each one to exactly one status code.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// The caller's input was rejected before touching any port.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The requested entity does not exist.
    #[error("{entity} {id} not found")]
    NotFound {
        /// Entity kind, e.g. `"character"`.
        entity: &'static str,
        /// Identifier as the caller sent it.
        id: String,
    },
    /// A uniqueness rule was violated.
    #[error("already exists: {0}")]
    AlreadyExists(String),
    /// The caller is not authenticated: no bearer token, or one that failed verification.
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),
    /// The caller is authenticated but may not act on this resource (another account's
    /// character).
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The idempotency key was reused with a different request body.
    #[error("idempotency key reused with a different request")]
    IdempotencyConflict,
    /// A port failed for a reason the caller cannot fix.
    #[error(transparent)]
    Infrastructure(#[from] anyhow::Error),
}

impl From<NameError> for AppError {
    fn from(e: NameError) -> Self {
        AppError::InvalidArgument(e.to_string())
    }
}

/// Parses a wire identifier, naming `field` in the `InvalidArgument` message on failure.
///
/// # Errors
/// `InvalidArgument("{field} must be a UUID")` when `raw` is not a UUID.
pub fn parse_id<T: FromStr>(field: &'static str, raw: &str) -> Result<T, AppError> {
    raw.parse()
        .map_err(|_| AppError::InvalidArgument(format!("{field} must be a UUID")))
}
