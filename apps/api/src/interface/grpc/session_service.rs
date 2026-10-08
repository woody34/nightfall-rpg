//! `SessionService`: admission to the real-time channel. Stub until Story 1.4.

use tonic::{Request, Response, Status};

use super::pb::session_service_server::{SessionService, SessionServiceServer};
use super::pb::{IssuePlayTicketRequest, IssuePlayTicketResponse};

/// `SessionService` implementation.
pub struct SessionServiceImpl;

impl SessionServiceImpl {
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
        _req: Request<IssuePlayTicketRequest>,
    ) -> Result<Response<IssuePlayTicketResponse>, Status> {
        Err(Status::unimplemented("Story 1.4"))
    }
}
