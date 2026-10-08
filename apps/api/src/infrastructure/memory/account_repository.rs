use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::application::ports::LoginOutcome;
use crate::application::AccountRepository;
use crate::domain::AccountId;

#[derive(Debug, Clone, Copy)]
struct Row {
    created_at: DateTime<Utc>,
    last_login_at: DateTime<Utc>,
}

/// Map-backed account store; the mutex makes `record_login` atomic like the Postgres upsert.
#[derive(Default)]
pub struct InMemoryAccountRepository {
    rows: Mutex<HashMap<AccountId, Row>>,
}

impl InMemoryAccountRepository {
    /// Number of accounts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.lock().len()
    }

    /// True when there are no accounts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` if the account exists.
    #[must_use]
    pub fn contains(&self, id: AccountId) -> bool {
        self.rows.lock().contains_key(&id)
    }

    /// Stored `last_login_at`.
    #[must_use]
    pub fn last_login(&self, id: AccountId) -> Option<DateTime<Utc>> {
        self.rows.lock().get(&id).map(|r| r.last_login_at)
    }

    /// Stored `created_at`.
    #[must_use]
    pub fn created_at(&self, id: AccountId) -> Option<DateTime<Utc>> {
        self.rows.lock().get(&id).map(|r| r.created_at)
    }
}

#[async_trait]
impl AccountRepository for InMemoryAccountRepository {
    async fn record_login(
        &self,
        id: AccountId,
        now: DateTime<Utc>,
        min_interval: chrono::Duration,
    ) -> anyhow::Result<LoginOutcome> {
        let threshold = now
            .checked_sub_signed(min_interval)
            .ok_or_else(|| anyhow::anyhow!("login bump threshold out of range"))?;
        let mut rows = self.rows.lock();
        match rows.get_mut(&id) {
            None => {
                rows.insert(
                    id,
                    Row {
                        created_at: now,
                        last_login_at: now,
                    },
                );
                Ok(LoginOutcome::Created)
            },
            Some(row) if row.last_login_at <= threshold => {
                row.last_login_at = now;
                Ok(LoginOutcome::Bumped)
            },
            Some(_) => Ok(LoginOutcome::Unchanged),
        }
    }
}
