//! `AppError` -> gRPC status. The single mapping point (api-guidelines.md section 3).

use tonic::Status;

use crate::application::AppError;

/// Maps application errors to canonical gRPC codes. Infrastructure details never leak.
pub(crate) fn to_status(e: AppError) -> Status {
    match e {
        AppError::InvalidArgument(m) => Status::invalid_argument(m),
        AppError::NotFound { entity, id } => Status::not_found(format!("{entity} {id} not found")),
        AppError::AlreadyExists(m) => Status::already_exists(m),
        AppError::Unauthenticated(m) => Status::unauthenticated(m),
        AppError::PermissionDenied(m) => Status::permission_denied(m),
        AppError::IdempotencyConflict => Status::failed_precondition(e.to_string()),
        AppError::FailedPrecondition(m) => Status::failed_precondition(m),
        AppError::ResourceExhausted(m) => Status::resource_exhausted(m),
        AppError::Unavailable(m) => Status::unavailable(m),
        AppError::Infrastructure(err) => {
            tracing::error!(error = ?err, "infrastructure error");
            Status::internal("internal error")
        },
    }
}

#[cfg(test)]
mod tests {
    use tonic::Code;

    use super::*;

    #[test]
    fn every_variant_maps_to_its_code() {
        let cases = [
            (AppError::InvalidArgument("x".into()), Code::InvalidArgument),
            (
                AppError::NotFound {
                    entity: "character",
                    id: "1".into(),
                },
                Code::NotFound,
            ),
            (AppError::AlreadyExists("x".into()), Code::AlreadyExists),
            (AppError::Unauthenticated("x".into()), Code::Unauthenticated),
            (AppError::PermissionDenied("x".into()), Code::PermissionDenied),
            (AppError::IdempotencyConflict, Code::FailedPrecondition),
            (AppError::FailedPrecondition("x".into()), Code::FailedPrecondition),
            (AppError::ResourceExhausted("x".into()), Code::ResourceExhausted),
            (AppError::Unavailable("x".into()), Code::Unavailable),
            (AppError::Infrastructure(anyhow::anyhow!("db down")), Code::Internal),
        ];
        for (e, code) in cases {
            assert_eq!(to_status(e).code(), code);
        }
    }

    #[test]
    fn infrastructure_details_do_not_leak() {
        let s = to_status(AppError::Infrastructure(anyhow::anyhow!("password=hunter2")));
        assert_eq!(s.message(), "internal error");
    }
}
