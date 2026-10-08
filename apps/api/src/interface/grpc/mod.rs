//! gRPC surface generated from packages/proto.

mod mapping;
mod session_service;

pub use session_service::SessionServiceImpl;

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::application::use_cases::{CreateCharacter, CreateCharacterInput, GetCharacter, Ping};
use crate::application::{AppError, IdempotencyKey};

/// Generated protobuf types.
#[allow(
    missing_docs,
    unreachable_pub,
    clippy::pedantic,
    clippy::all,
    clippy::wildcard_enum_match_arm,
    clippy::str_to_string
)]
pub mod pb {
    tonic::include_proto!("nightfall.v1");
}

use pb::game_service_server::{GameService, GameServiceServer};
use pb::{
    Character, CreateCharacterRequest, GetCharacterRequest, ListMyCharactersRequest,
    ListMyCharactersResponse, PingRequest, PingResponse,
};

/// `GameService` implementation. Holds use cases, nothing else.
pub struct GameServiceImpl {
    ping: Ping,
    get_character: GetCharacter,
    create_character: CreateCharacter,
}

impl GameServiceImpl {
    /// Builds the service from its use cases.
    #[must_use]
    pub fn new(ping: Ping, get_character: GetCharacter, create_character: CreateCharacter) -> Self {
        Self {
            ping,
            get_character,
            create_character,
        }
    }

    /// Wraps in the tonic server type.
    #[must_use]
    pub fn into_server(self) -> GameServiceServer<Self> {
        GameServiceServer::new(self)
    }
}

/// Maps application errors to canonical gRPC codes. Infrastructure details never leak.
fn to_status(e: AppError) -> Status {
    match e {
        AppError::InvalidArgument(m) => Status::invalid_argument(m),
        AppError::NotFound { entity, id } => Status::not_found(format!("{entity} {id} not found")),
        AppError::AlreadyExists(m) => Status::already_exists(m),
        AppError::IdempotencyConflict => Status::failed_precondition(e.to_string()),
        AppError::Infrastructure(err) => {
            tracing::error!(error = ?err, "infrastructure error");
            Status::internal("internal error")
        },
    }
}

#[tonic::async_trait]
impl GameService for GameServiceImpl {
    async fn ping(&self, req: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let out = self.ping.execute(&req.get_ref().client_version);
        Ok(Response::new(PingResponse {
            server_version: out.server_version.to_owned(),
            server_time_ms: out.server_time_ms,
        }))
    }

    async fn get_character(
        &self,
        req: Request<GetCharacterRequest>,
    ) -> Result<Response<Character>, Status> {
        let c = self
            .get_character
            .execute(&req.get_ref().character_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::character_to_pb(&c)))
    }

    async fn create_character(
        &self,
        req: Request<CreateCharacterRequest>,
    ) -> Result<Response<Character>, Status> {
        let req = req.into_inner();
        let idempotency_key = IdempotencyKey::parse(&req.idempotency_key)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        #[allow(deprecated)] // removed in Story 1.6: the account will come from the token
        let account_id = uuid::Uuid::parse_str(&req.account_id)
            .map_err(|_| Status::invalid_argument("account_id must be a UUID"))?;
        let race = mapping::race_from_pb(req.race)
            .ok_or_else(|| Status::invalid_argument("race must be specified"))?;
        let c = self
            .create_character
            .execute(CreateCharacterInput {
                idempotency_key,
                account_id,
                name: req.name,
                race,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::character_to_pb(&c)))
    }

    async fn list_my_characters(
        &self,
        _req: Request<ListMyCharactersRequest>,
    ) -> Result<Response<ListMyCharactersResponse>, Status> {
        Err(Status::unimplemented("Story 1.4"))
    }
}

/// Convenience for the composition root and tests: an `Arc`-free way to hand over the service.
pub type SharedService = Arc<GameServiceImpl>;
