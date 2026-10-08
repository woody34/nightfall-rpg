use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::application::ports::{Admission, ConsumeError, IssueOutcome, IssuedTicket, NewTicket};
use crate::application::{IdempotencyKey, SessionRepository};
use crate::domain::{AccountId, CharacterId, SessionGeneration, TicketHash};

struct Ticket {
    account_id: AccountId,
    character_id: CharacterId,
    generation: SessionGeneration,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct State {
    tickets: HashMap<TicketHash, Ticket>,
    generations: HashMap<AccountId, SessionGeneration>,
    keys: HashMap<(AccountId, IdempotencyKey), (String, IssuedTicket)>,
}

/// Map-backed play tickets and session generations under one mutex, so each method is atomic
/// like its Postgres transaction.
#[derive(Default)]
pub struct InMemorySessionRepository {
    state: Mutex<State>,
}

impl InMemorySessionRepository {
    /// Number of stored tickets (consumed or not).
    #[must_use]
    pub fn ticket_count(&self) -> usize {
        self.state.lock().tickets.len()
    }

    /// The account's current session generation, if it was ever issued a ticket.
    #[must_use]
    pub fn generation(&self, account: AccountId) -> Option<SessionGeneration> {
        self.state.lock().generations.get(&account).copied()
    }
}

#[async_trait]
impl SessionRepository for InMemorySessionRepository {
    async fn issue_ticket_idempotent(
        &self,
        key: &IdempotencyKey,
        fingerprint: &str,
        ticket: &NewTicket,
    ) -> anyhow::Result<IssueOutcome> {
        let mut s = self.state.lock();
        let scoped = (ticket.account_id, *key);
        if let Some((stored_fp, response)) = s.keys.get(&scoped) {
            if stored_fp != fingerprint {
                return Ok(IssueOutcome::KeyReused);
            }
            return Ok(IssueOutcome::Replayed(response.clone()));
        }
        if s.tickets.contains_key(&ticket.hash) {
            anyhow::bail!("play ticket hash collision");
        }
        let generation = match s.generations.get(&ticket.account_id) {
            None => SessionGeneration::FIRST,
            Some(g) => g
                .next()
                .ok_or_else(|| anyhow::anyhow!("session generation overflow"))?,
        };
        s.generations.insert(ticket.account_id, generation);
        s.tickets.insert(
            ticket.hash,
            Ticket {
                account_id: ticket.account_id,
                character_id: ticket.character_id,
                generation,
                expires_at: ticket.response.expires_at,
                consumed_at: None,
            },
        );
        s.keys
            .insert(scoped, (fingerprint.to_owned(), ticket.response.clone()));
        Ok(IssueOutcome::Issued {
            response: ticket.response.clone(),
            generation,
        })
    }

    async fn consume_ticket(
        &self,
        hash: &TicketHash,
        now: DateTime<Utc>,
    ) -> Result<Admission, ConsumeError> {
        let mut s = self.state.lock();
        let current = {
            let t = s.tickets.get(hash).ok_or(ConsumeError::Unknown)?;
            s.generations.get(&t.account_id).copied()
        };
        let t = s.tickets.get_mut(hash).ok_or(ConsumeError::Unknown)?;
        if t.consumed_at.is_some() {
            return Err(ConsumeError::Consumed);
        }
        if t.expires_at <= now {
            return Err(ConsumeError::Expired);
        }
        t.consumed_at = Some(now);
        if current.is_some_and(|g| t.generation < g) {
            return Err(ConsumeError::Superseded);
        }
        Ok(Admission {
            account_id: t.account_id,
            character_id: t.character_id,
            generation: t.generation,
        })
    }
}
