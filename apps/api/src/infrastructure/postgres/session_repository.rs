use std::future::Future;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    DbBackend, EntityTrait, QuerySelect, Statement, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use super::entities::{account_sessions, play_tickets};
use super::idempotency::{self, operation, Claim};
use crate::application::ports::{Admission, ConsumeError, IssueOutcome, IssuedTicket, NewTicket};
use crate::application::{IdempotencyKey, SessionRepository};
use crate::domain::{AccountId, CharacterId, PlayTicket, SessionGeneration, TicketHash};
use crate::infrastructure::telemetry::Metrics;

/// First ticket for an account starts at 1; every later one adds 1. The row lock taken by the
/// upsert serializes concurrent issues for one account.
const BUMP_GENERATION: &str = "\
    INSERT INTO account_sessions (account_id, generation, updated_at) VALUES ($1, 1, $2) \
    ON CONFLICT (account_id) DO UPDATE \
        SET generation = account_sessions.generation + 1, updated_at = EXCLUDED.updated_at \
    RETURNING generation";

/// What an `issue_play_ticket` idempotency record stores: the full response. The ticket is
/// in plain form here (a retry must return it), but it is only usable for its 60 s lifetime
/// and only once; the ticket table itself holds the hash.
#[derive(Serialize, Deserialize)]
struct StoredResponse {
    ticket: String,
    expires_at: DateTime<Utc>,
    ws_url: String,
}

impl StoredResponse {
    fn from_issued(t: &IssuedTicket) -> Self {
        Self {
            ticket: t.ticket.encode(),
            expires_at: t.expires_at,
            ws_url: t.ws_url.clone(),
        }
    }

    fn into_issued(self) -> anyhow::Result<IssuedTicket> {
        Ok(IssuedTicket {
            ticket: PlayTicket::parse(&self.ticket)?,
            expires_at: self.expires_at,
            ws_url: self.ws_url,
        })
    }
}

/// Play tickets and session generations in Postgres.
#[derive(Clone)]
pub struct PgSessionRepository {
    db: DatabaseConnection,
    metrics: Option<Metrics>,
}

impl PgSessionRepository {
    /// Wraps a connection (a `sqlx::PgPool` converts into one).
    #[must_use]
    pub fn new(db: impl Into<DatabaseConnection>) -> Self {
        Self {
            db: db.into(),
            metrics: None,
        }
    }

    /// Records every query in `db_query_seconds{repo="session",op}`.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Metrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn timed<T>(&self, op: &'static str, fut: impl Future<Output = T>) -> T {
        match &self.metrics {
            Some(m) => m.time_db("session", op, fut).await,
            None => fut.await,
        }
    }

    /// One transaction: claim the key with the full response, bump the generation, store the
    /// ticket hash. A known key writes nothing.
    async fn issue_tx(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        ticket: &NewTicket,
    ) -> anyhow::Result<IssueOutcome> {
        let tx = self.db.begin().await?;
        let response = serde_json::to_value(StoredResponse::from_issued(&ticket.response))?;
        let claim = idempotency::claim(
            &tx,
            ticket.account_id,
            operation::ISSUE_PLAY_TICKET,
            key,
            fingerprint,
            response,
        )
        .await?;
        if let Claim::Existing {
            fingerprint: stored_fp,
            response,
        } = claim
        {
            tx.commit().await?;
            if stored_fp != fingerprint {
                return Ok(IssueOutcome::KeyReused);
            }
            let stored: StoredResponse = serde_json::from_value(response)?;
            return Ok(IssueOutcome::Replayed(stored.into_issued()?));
        }

        let generation = bump_generation(&tx, ticket.account_id, ticket.issued_at).await?;
        play_tickets::Entity::insert(play_tickets::ActiveModel {
            ticket_hash: ActiveValue::Set(ticket.hash.as_bytes().to_vec()),
            account_id: ActiveValue::Set(ticket.account_id.as_uuid()),
            character_id: ActiveValue::Set(ticket.character_id.as_uuid()),
            generation: ActiveValue::Set(generation.get()),
            created_at: ActiveValue::Set(ticket.issued_at.fixed_offset()),
            expires_at: ActiveValue::Set(ticket.response.expires_at.fixed_offset()),
            consumed_at: ActiveValue::Set(None),
        })
        .exec_without_returning(&tx)
        .await?;

        tx.commit().await?;
        Ok(IssueOutcome::Issued {
            response: ticket.response.clone(),
            generation,
        })
    }

    /// One transaction: lock the ticket row, classify, mark consumed, compare generations.
    ///
    /// The generation is read without a lock, so an issue racing this consume may bump it
    /// just after; that admission then carries the older generation, which the zone actor
    /// fences (plan Revision 1, item 8).
    async fn consume_tx(
        &self,
        hash: &TicketHash,
        now: DateTime<Utc>,
    ) -> Result<Admission, ConsumeError> {
        let tx = self.db.begin().await.map_err(anyhow::Error::from)?;
        let row = play_tickets::Entity::find_by_id(hash.as_bytes().to_vec())
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(anyhow::Error::from)?
            .ok_or(ConsumeError::Unknown)?;
        if row.consumed_at.is_some() {
            return Err(ConsumeError::Consumed);
        }
        if row.expires_at <= now {
            return Err(ConsumeError::Expired);
        }
        let admission = Admission {
            account_id: AccountId::from_uuid(row.account_id),
            character_id: CharacterId::from_uuid(row.character_id),
            generation: SessionGeneration::new(row.generation)
                .ok_or_else(|| anyhow::anyhow!("non-positive generation in play_tickets"))?,
        };

        let mut consumed: play_tickets::ActiveModel = row.into();
        consumed.consumed_at = ActiveValue::Set(Some(now.fixed_offset()));
        consumed.update(&tx).await.map_err(anyhow::Error::from)?;

        let current = account_sessions::Entity::find_by_id(admission.account_id.as_uuid())
            .one(&tx)
            .await
            .map_err(anyhow::Error::from)?
            .and_then(|s| SessionGeneration::new(s.generation));
        tx.commit().await.map_err(anyhow::Error::from)?;

        if current.is_some_and(|g| admission.generation < g) {
            return Err(ConsumeError::Superseded);
        }
        Ok(admission)
    }
}

async fn bump_generation(
    tx: &DatabaseTransaction,
    account: AccountId,
    now: DateTime<Utc>,
) -> anyhow::Result<SessionGeneration> {
    let row = tx
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            BUMP_GENERATION,
            [account.as_uuid().into(), now.into()],
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("generation upsert returned no row"))?;
    let g: i32 = row.try_get("", "generation")?;
    SessionGeneration::new(g).ok_or_else(|| anyhow::anyhow!("non-positive session generation"))
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn issue_ticket_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        ticket: &NewTicket,
    ) -> anyhow::Result<IssueOutcome> {
        self.timed("issue_ticket_idempotent", self.issue_tx(key, fingerprint, ticket))
            .await
    }

    async fn consume_ticket(
        &self,
        hash: &TicketHash,
        now: DateTime<Utc>,
    ) -> Result<Admission, ConsumeError> {
        self.timed("consume_ticket", self.consume_tx(hash, now))
            .await
    }
}
