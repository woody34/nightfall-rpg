//! Application error type. Interface adapters map this to gRPC status codes and HTTP status.

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
