//! Make sure the caller's account exists (Story 1.3). Runs after every successful token
//! verification, so it must be cheap and idempotent by construction: one upsert that writes
//! only on first login or when `last_login_at` is more than a minute old.

use std::sync::Arc;

use crate::application::ports::LoginOutcome;
use crate::application::{AccountRepository, AppError, Clock};
use crate::domain::AccountId;

/// `last_login_at` moves at most this often per account; requests in between write nothing.
pub const LOGIN_BUMP_INTERVAL_SECONDS: i64 = 60;

/// The ensure-account use case.
pub struct EnsureAccount {
    accounts: Arc<dyn AccountRepository>,
    clock: Arc<dyn Clock>,
}

impl EnsureAccount {
    /// Builds the use case.
    #[must_use]
    pub fn new(accounts: Arc<dyn AccountRepository>, clock: Arc<dyn Clock>) -> Self {
        Self { accounts, clock }
    }

    /// Executes. Errors are infrastructure only.
    pub async fn execute(&self, id: AccountId) -> Result<LoginOutcome, AppError> {
        let outcome = self
            .accounts
            .record_login(
                id,
                self.clock.now(),
                chrono::Duration::seconds(LOGIN_BUMP_INTERVAL_SECONDS),
            )
            .await?;
        if outcome == LoginOutcome::Created {
            tracing::info!(account_id = %id, "account created on first login");
        }
        Ok(outcome)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use chrono::Duration;
    use uuid::Uuid;

    use super::*;
    use crate::infrastructure::memory::{InMemoryAccountRepository, ManualClock};

    fn sut() -> (EnsureAccount, Arc<InMemoryAccountRepository>, Arc<ManualClock>) {
        let repo = Arc::new(InMemoryAccountRepository::default());
        let clock = Arc::new(ManualClock::default());
        (EnsureAccount::new(repo.clone(), clock.clone()), repo, clock)
    }

    fn account() -> AccountId {
        AccountId::from_uuid(Uuid::from_u128(1))
    }

    #[tokio::test]
    async fn two_logins_create_one_account() {
        let (uc, repo, _) = sut();
        assert_eq!(uc.execute(account()).await.unwrap(), LoginOutcome::Created);
        assert_eq!(uc.execute(account()).await.unwrap(), LoginOutcome::Unchanged);
        assert_eq!(repo.len(), 1);
    }

    #[tokio::test]
    async fn last_login_moves_at_most_once_per_interval() {
        let (uc, repo, clock) = sut();
        let t0 = clock.now();
        uc.execute(account()).await.unwrap();

        clock.advance(Duration::seconds(LOGIN_BUMP_INTERVAL_SECONDS - 1));
        assert_eq!(uc.execute(account()).await.unwrap(), LoginOutcome::Unchanged);
        assert_eq!(repo.last_login(account()), Some(t0));

        clock.advance(Duration::seconds(1));
        assert_eq!(uc.execute(account()).await.unwrap(), LoginOutcome::Bumped);
        assert_eq!(repo.last_login(account()), Some(clock.now()));
        assert_eq!(repo.created_at(account()), Some(t0), "created_at never moves");
    }
}
