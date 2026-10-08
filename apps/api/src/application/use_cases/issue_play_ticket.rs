//! Issue a single-use play ticket for one of the caller's characters (Story 1.4, plan
//! Revision 1 items 7-9).
//!
//! Lifecycle: issue (this use case) bumps the account's session generation and stores the
//! ticket's hash with a 60 s expiry, in one transaction with the idempotency record that holds
//! the full response. The `/ws` handshake consumes it ([`super::ConsumePlayTicket`]).

use std::sync::Arc;

use crate::application::ports::{IssueOutcome, IssuedTicket, NewTicket};
use crate::application::{
    AppError, CharacterRepository, Clock, IdempotencyKey, SecretGenerator, SessionRepository,
};
use crate::domain::{AccountId, CharacterId, PlayTicket};

/// Validated input. Built by the transport adapter.
#[derive(Debug, Clone)]
pub struct IssuePlayTicketInput {
    /// Client retry key; a new one per connection attempt.
    pub idempotency_key: IdempotencyKey,
    /// The verified caller.
    pub account_id: AccountId,
    /// The character to play; must belong to the caller.
    pub character_id: CharacterId,
}

/// The issue-play-ticket use case.
pub struct IssuePlayTicket {
    characters: Arc<dyn CharacterRepository>,
    sessions: Arc<dyn SessionRepository>,
    secrets: Arc<dyn SecretGenerator>,
    clock: Arc<dyn Clock>,
    ws_url: String,
}

impl IssuePlayTicket {
    /// Builds the use case. `ws_url` is the public WebSocket address returned to clients.
    #[must_use]
    pub fn new(
        characters: Arc<dyn CharacterRepository>,
        sessions: Arc<dyn SessionRepository>,
        secrets: Arc<dyn SecretGenerator>,
        clock: Arc<dyn Clock>,
        ws_url: String,
    ) -> Self {
        Self {
            characters,
            sessions,
            secrets,
            clock,
            ws_url,
        }
    }

    /// Executes. A retry with the same key returns the identical ticket, even if it has since
    /// been consumed or has expired.
    ///
    /// # Errors
    /// `NotFound` (no such character), `PermissionDenied` (another account's character),
    /// `IdempotencyConflict` (key reused for a different character).
    pub async fn execute(&self, input: IssuePlayTicketInput) -> Result<IssuedTicket, AppError> {
        let character = self
            .characters
            .get(input.character_id)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "character",
                id: input.character_id.to_string(),
            })?;
        if character.account_id != input.account_id {
            return Err(AppError::PermissionDenied(format!(
                "character {} belongs to another account",
                input.character_id
            )));
        }

        let ticket = self.secrets.play_ticket()?;
        let now = self.clock.now();
        let expires_at = now
            .checked_add_signed(chrono::Duration::seconds(PlayTicket::TTL_SECONDS))
            .ok_or_else(|| anyhow::anyhow!("ticket expiry out of range"))?;
        let new = NewTicket {
            account_id: input.account_id,
            character_id: input.character_id,
            hash: ticket.hash(),
            issued_at: now,
            response: IssuedTicket {
                ticket,
                expires_at,
                ws_url: self.ws_url.clone(),
            },
        };
        let fingerprint = Self::fingerprint(&input);

        match self
            .sessions
            .issue_ticket_idempotent(&input.idempotency_key, &fingerprint, &new)
            .await?
        {
            IssueOutcome::Issued {
                response,
                generation,
            } => {
                tracing::info!(
                    character_id = %input.character_id,
                    generation = generation.get(),
                    "play ticket issued"
                );
                Ok(response)
            },
            IssueOutcome::Replayed(response) => {
                tracing::info!(key = %input.idempotency_key, "issue_play_ticket replayed");
                Ok(response)
            },
            IssueOutcome::KeyReused => Err(AppError::IdempotencyConflict),
        }
    }

    /// The account scopes the key, so only the character distinguishes two requests.
    fn fingerprint(input: &IssuePlayTicketInput) -> String {
        format!("v1|{}", input.character_id)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::{Character, CharacterName, Race};
    use crate::infrastructure::memory::{
        InMemoryCharacterRepository, InMemorySessionRepository, ManualClock,
        SequentialSecretGenerator,
    };

    const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
    const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";
    const WS: &str = "ws://localhost:3000/ws";

    struct Sut {
        uc: IssuePlayTicket,
        characters: Arc<InMemoryCharacterRepository>,
        sessions: Arc<InMemorySessionRepository>,
        clock: Arc<ManualClock>,
    }

    fn sut() -> Sut {
        let characters = Arc::new(InMemoryCharacterRepository::default());
        let sessions = Arc::new(InMemorySessionRepository::default());
        let clock = Arc::new(ManualClock::default());
        let uc = IssuePlayTicket::new(
            characters.clone(),
            sessions.clone(),
            Arc::new(SequentialSecretGenerator::default()),
            clock.clone(),
            WS.into(),
        );
        Sut {
            uc,
            characters,
            sessions,
            clock,
        }
    }

    fn account(n: u128) -> AccountId {
        AccountId::from_uuid(Uuid::from_u128(n))
    }

    fn seed(s: &Sut, owner: AccountId, name: &str) -> CharacterId {
        let c = Character::create(owner, CharacterName::new(name).unwrap(), Race::Human);
        s.characters.insert_for_test(c.clone());
        c.id
    }

    fn input(key: &str, owner: AccountId, character_id: CharacterId) -> IssuePlayTicketInput {
        IssuePlayTicketInput {
            idempotency_key: IdempotencyKey::parse(key).unwrap(),
            account_id: owner,
            character_id,
        }
    }

    #[tokio::test]
    async fn issues_a_ticket_valid_for_sixty_seconds() {
        let s = sut();
        let c = seed(&s, account(1), "Aria");
        let out = s.uc.execute(input(KEY_A, account(1), c)).await.unwrap();
        assert_eq!(out.ws_url, WS);
        assert_eq!(out.expires_at, s.clock.now() + chrono::Duration::seconds(60));
        assert_eq!(s.sessions.ticket_count(), 1);
        assert_eq!(s.sessions.generation(account(1)).unwrap().get(), 1);
    }

    #[tokio::test]
    async fn same_key_replays_the_identical_ticket_without_a_second_write() {
        let s = sut();
        let c = seed(&s, account(1), "Aria");
        let first = s.uc.execute(input(KEY_A, account(1), c)).await.unwrap();
        s.clock.advance(chrono::Duration::seconds(5));
        let second = s.uc.execute(input(KEY_A, account(1), c)).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(s.sessions.ticket_count(), 1);
        assert_eq!(s.sessions.generation(account(1)).unwrap().get(), 1, "no second bump");
    }

    #[tokio::test]
    async fn same_key_for_a_different_character_is_a_conflict() {
        let s = sut();
        let c1 = seed(&s, account(1), "Aria");
        let c2 = seed(&s, account(1), "Brea");
        s.uc.execute(input(KEY_A, account(1), c1)).await.unwrap();
        let err =
            s.uc.execute(input(KEY_A, account(1), c2))
                .await
                .unwrap_err();
        assert!(matches!(err, AppError::IdempotencyConflict), "{err:?}");
        assert_eq!(s.sessions.ticket_count(), 1);
    }

    #[tokio::test]
    async fn another_accounts_character_is_permission_denied_and_writes_nothing() {
        let s = sut();
        let c = seed(&s, account(1), "Aria");
        let err = s.uc.execute(input(KEY_A, account(2), c)).await.unwrap_err();
        assert!(matches!(err, AppError::PermissionDenied(_)), "{err:?}");
        assert_eq!(s.sessions.ticket_count(), 0);
    }

    #[tokio::test]
    async fn unknown_character_is_not_found() {
        let s = sut();
        let err =
            s.uc.execute(input(KEY_A, account(1), CharacterId::new()))
                .await
                .unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }), "{err:?}");
        assert_eq!(s.sessions.ticket_count(), 0);
    }

    #[tokio::test]
    async fn each_new_key_bumps_the_generation_and_mints_a_new_ticket() {
        let s = sut();
        let c = seed(&s, account(1), "Aria");
        let a = s.uc.execute(input(KEY_A, account(1), c)).await.unwrap();
        let b = s.uc.execute(input(KEY_B, account(1), c)).await.unwrap();
        assert_ne!(a.ticket, b.ticket);
        assert_eq!(s.sessions.generation(account(1)).unwrap().get(), 2);
    }
}
