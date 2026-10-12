//! Out-of-world inventory availability policy (D15). Live inventory uses `InventorySync`.
use crate::application::AppError;

/// Checks required ports before refusing admitted characters. This guard must precede
/// retained inventory reads, so a live character can never receive stale database rows.
pub fn require_out_of_world(
    zone_available: bool,
    catalogue_available: bool,
    admitted: bool,
) -> Result<(), AppError> {
    if !zone_available || !catalogue_available {
        return Err(AppError::Unavailable("item runtime or catalogue is unavailable".into()));
    }
    if admitted {
        return Err(AppError::FailedPrecondition(
            "admitted characters receive InventorySync over the world channel".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d15_refuses_live_reads_and_missing_ports() {
        assert!(require_out_of_world(true, true, false).is_ok());
        assert!(matches!(
            require_out_of_world(true, true, true),
            Err(AppError::FailedPrecondition(_))
        ));
        for (zone, catalogue) in [(false, false), (true, false), (false, true)] {
            for admitted in [false, true] {
                assert!(matches!(
                    require_out_of_world(zone, catalogue, admitted),
                    Err(AppError::Unavailable(_))
                ));
            }
        }
    }
}
