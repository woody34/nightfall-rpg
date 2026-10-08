use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

use crate::application::ports::LoginOutcome;
use crate::application::AccountRepository;
use crate::domain::AccountId;
use crate::infrastructure::telemetry::Metrics;

/// One statement, so one implicit transaction. The `WHERE` on the conflict branch is what
/// keeps this cheap: inside the bump interval the row is locked briefly but not rewritten, so
/// an authenticated request costs no new tuple and no WAL. `created_at = $2` is true only for
/// the inserting request (an update requires the old `last_login_at`, and therefore
/// `created_at`, to be at least the interval older than `$2`).
const RECORD_LOGIN: &str = "\
    INSERT INTO accounts (id, created_at, last_login_at) VALUES ($1, $2, $2) \
    ON CONFLICT (id) DO UPDATE SET last_login_at = EXCLUDED.last_login_at \
        WHERE accounts.last_login_at <= $3 \
    RETURNING created_at = $2 AS created";

/// Account persistence in Postgres.
#[derive(Clone)]
pub struct PgAccountRepository {
    db: DatabaseConnection,
    metrics: Option<Metrics>,
}

impl PgAccountRepository {
    /// Wraps a connection (a `sqlx::PgPool` converts into one).
    #[must_use]
    pub fn new(db: impl Into<DatabaseConnection>) -> Self {
        Self {
            db: db.into(),
            metrics: None,
        }
    }

    /// Records every query in `db_query_seconds{repo="account",op}`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }
}

#[async_trait]
impl AccountRepository for PgAccountRepository {
    async fn record_login(
        &self,
        id: AccountId,
        now: DateTime<Utc>,
        min_interval: chrono::Duration,
    ) -> anyhow::Result<LoginOutcome> {
        let threshold = now
            .checked_sub_signed(min_interval)
            .ok_or_else(|| anyhow::anyhow!("login bump threshold out of range"))?;
        let stmt = Statement::from_sql_and_values(
            DbBackend::Postgres,
            RECORD_LOGIN,
            [id.as_uuid().into(), now.into(), threshold.into()],
        );
        let fut = self.db.query_one_raw(stmt);
        let row = match &self.metrics {
            Some(m) => m.time_db("account", "record_login", fut).await,
            None => fut.await,
        }?;
        Ok(match row {
            None => LoginOutcome::Unchanged,
            Some(r) if r.try_get::<bool>("", "created")? => LoginOutcome::Created,
            Some(_) => LoginOutcome::Bumped,
        })
    }
}
