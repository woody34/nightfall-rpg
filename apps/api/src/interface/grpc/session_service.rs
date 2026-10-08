//! `SessionService`: admission to the real-time channel (Story 1.4).

use tonic::{Request, Response, Status};

use super::pb::session_service_server::{SessionService, SessionServiceServer};
use super::pb::{IssuePlayTicketRequest, IssuePlayTicketResponse};
use super::{auth, status::to_status};
use crate::application::use_cases::{IssuePlayTicket, IssuePlayTicketInput};
use crate::application::{parse_id, IdempotencyKey};
use crate::domain::CharacterId;

/// `SessionService` implementation.
pub struct SessionServiceImpl {
    issue_play_ticket: IssuePlayTicket,
}

impl SessionServiceImpl {
    /// Builds the service from its use cases.
    #[must_use]
    pub fn new(issue_play_ticket: IssuePlayTicket) -> Self {
        Self { issue_play_ticket }
    }

    /// Wraps in the tonic server type.
    #[must_use]
    pub fn into_server(self) -> SessionServiceServer<Self> {
        SessionServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl SessionService for SessionServiceImpl {
    async fn issue_play_ticket(
        &self,
        req: Request<IssuePlayTicketRequest>,
    ) -> Result<Response<IssuePlayTicketResponse>, Status> {
        let caller = auth::caller(&req)?;
        let req = req.into_inner();
        let idempotency_key = IdempotencyKey::parse(&req.idempotency_key)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let character_id: CharacterId =
            parse_id("character_id", &req.character_id).map_err(to_status)?;
        let issued = self
            .issue_play_ticket
            .execute(IssuePlayTicketInput {
                idempotency_key,
                account_id: caller,
                character_id,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(IssuePlayTicketResponse {
            ticket: issued.ticket.encode(),
            expires_at_ms: issued.expires_at.timestamp_millis(),
            ws_url: issued.ws_url,
        }))
    }
}
