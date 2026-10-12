//! Dev grants retain the production endpoint but require the exact startup auth gate (A15).
use crate::application::AppError;

/// Gate before any receipt lookup, queued input, allocator use or ownership write.
pub fn require_dev_grants(enabled: bool) -> Result<(), AppError> {
    if enabled {
        Ok(())
    } else {
        Err(AppError::FailedPrecondition("GrantItem requires AUTH_DEV_TOKENS=1".into()))
    }
}
