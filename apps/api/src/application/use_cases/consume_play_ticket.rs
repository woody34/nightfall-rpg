//! Consume a play ticket in the `/ws` handshake (Story 1.4; the handler lands in Epic 4).
//!
//! The handler maps [`TicketRejection`] to HTTP before the upgrade (plan Revision 1, item 17):
//! `Invalid`, `Expired`, `Consumed` are 401; `Superseded` is 409.

use std::sync::Arc;

use crate::application::ports::{Admission, ConsumeError};
use crate::application::{Clock, SessionRepository};
use crate::domain::PlayTicket;

/// Why a ticket was refused.
#[derive(Debug, thiserror::Error)]
pub enum TicketRejection {
    /// Malformed, or no such ticket. Deliberately one variant: the caller learns nothing
    /// about which.
    #[error("invalid play ticket")]
    Invalid,
    /// Past its 60 s lifetime.
    #[error("play ticket expired")]
    Expired,
    /// Already used.
    #[error("play ticket already used")]
    Consumed,
    /// A newer ticket exists for this account.
    #[error("play ticket superseded by a newer one")]
    Superseded,
    /// A port failed.
    #[error(transparent)]
    Infrastructure(#[from] anyhow::Error),
}

/// The consume-play-ticket use case.
pub struct ConsumePlayTicket {
    sessions: Arc<dyn SessionRepository>,
    clock: Arc<dyn Clock>,
}

impl ConsumePlayTicket {
    /// Builds the use case.
    #[must_use]
    pub fn new(sessions: Arc<dyn SessionRepository>, clock: Arc<dyn Clock>) -> Self {
        Self { sessions, clock }
    }

    /// Executes. `raw` is the ticket as presented in `Authorization: Bearer`.
    pub async fn execute(&self, raw: &str) -> Result<Admission, TicketRejection> {
        let ticket = PlayTicket::parse(raw).map_err(|_| TicketRejection::Invalid)?;
        self.sessions
            .consume_ticket(&ticket.hash(), self.clock.now())
            .await
            .map_err(|e| match e {
                ConsumeError::Unknown => TicketRejection::Invalid,
                ConsumeError::Consumed => TicketRejection::Consumed,
                ConsumeError::Expired => TicketRejection::Expired,
                ConsumeError::Superseded => TicketRejection::Superseded,
                ConsumeError::Other(e) => TicketRejection::Infrastructure(e),
            })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::application::use_cases::{IssuePlayTicket, IssuePlayTicketInput};
    use crate::application::IdempotencyKey;
    use crate::domain::{AccountId, Character, CharacterName, Race};
    use crate::infrastructure::memory::{
        InMemoryCharacterRepository, InMemorySessionRepository, ManualClock,
        SequentialSecretGenerator,
    };

    struct Sut {
        issue: IssuePlayTicket,
        consume: ConsumePlayTicket,
        clock: Arc<ManualClock>,
        character: Character,
    }

    fn sut() -> Sut {
        let characters = Arc::new(InMemoryCharacterRepository::default());
        let sessions = Arc::new(InMemorySessionRepository::default());
        let clock = Arc::new(ManualClock::default());
        let character = Character::create(
            AccountId::from_uuid(Uuid::from_u128(1)),
            CharacterName::new("Aria").unwrap(),
            Race::Elf,
        );
        characters.insert_for_test(character.clone());
        Sut {
            issue: IssuePlayTicket::new(
                characters,
                sessions.clone(),
                Arc::new(SequentialSecretGenerator::default()),
                clock.clone(),
                "ws://test/ws".into(),
            ),
            consume: ConsumePlayTicket::new(sessions, clock.clone()),
            clock,
            character,
        }
    }

    async fn issue(s: &Sut, key: u128) -> String {
        s.issue
            .execute(IssuePlayTicketInput {
                idempotency_key: IdempotencyKey::parse(&Uuid::from_u128(key).to_string()).unwrap(),
                account_id: s.character.account_id,
                character_id: s.character.id,
            })
            .await
            .unwrap()
            .ticket
            .encode()
    }

    #[tokio::test]
    async fn consume_returns_the_admission() {
        let s = sut();
        let t = issue(&s, 1).await;
        let a = s.consume.execute(&t).await.unwrap();
        assert_eq!(a.account_id, s.character.account_id);
        assert_eq!(a.character_id, s.character.id);
        assert_eq!(a.generation.get(), 1);
    }

    #[tokio::test]
    async fn consume_twice_fails() {
        let s = sut();
        let t = issue(&s, 1).await;
        s.consume.execute(&t).await.unwrap();
        let err = s.consume.execute(&t).await.unwrap_err();
        assert!(matches!(err, TicketRejection::Consumed), "{err:?}");
    }

    #[tokio::test]
    async fn consume_after_expiry_fails() {
        let s = sut();
        let t = issue(&s, 1).await;
        s.clock.advance(chrono::Duration::seconds(60));
        let err = s.consume.execute(&t).await.unwrap_err();
        assert!(matches!(err, TicketRejection::Expired), "{err:?}");
    }

    #[tokio::test]
    async fn consume_just_before_expiry_succeeds() {
        let s = sut();
        let t = issue(&s, 1).await;
        s.clock.advance(chrono::Duration::milliseconds(59_999));
        s.consume.execute(&t).await.unwrap();
    }

    #[tokio::test]
    async fn older_ticket_is_superseded_by_a_newer_one() {
        let s = sut();
        let old = issue(&s, 1).await;
        let new = issue(&s, 2).await;
        let err = s.consume.execute(&old).await.unwrap_err();
        assert!(matches!(err, TicketRejection::Superseded), "{err:?}");
        let a = s.consume.execute(&new).await.unwrap();
        assert_eq!(a.generation.get(), 2);
    }

    #[tokio::test]
    async fn unknown_and_malformed_tickets_are_invalid() {
        let s = sut();
        let unknown = PlayTicket::from_bytes([9; 32]).encode();
        assert!(matches!(s.consume.execute(&unknown).await, Err(TicketRejection::Invalid)));
        assert!(matches!(s.consume.execute("not a ticket").await, Err(TicketRejection::Invalid)));
    }

    #[tokio::test]
    async fn replayed_issue_after_consumption_returns_the_spent_ticket() {
        let s = sut();
        let t = issue(&s, 1).await;
        s.consume.execute(&t).await.unwrap();
        assert_eq!(issue(&s, 1).await, t, "same key, same ticket");
        let err = s.consume.execute(&t).await.unwrap_err();
        assert!(matches!(err, TicketRejection::Consumed), "{err:?}");
    }
}
