//! gRPC surface generated from packages/proto.
//!
//! Every RPC except `Ping` runs behind [`AuthLayer`]; handlers take the caller from the
//! request extensions ([`auth::caller`]), never from a request field (Story 1.6).

pub mod auth;
mod mapping;
mod session_service;
mod status;

pub use auth::{AuthLayer, PUBLIC_METHODS};
pub use session_service::SessionServiceImpl;

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::application::use_cases::{
    ChangeClass, CreateCharacter, CreateCharacterInput, GetCharacter, ListClasses,
    ListMyCharacters, Ping, TransferOptions,
};
use crate::application::IdempotencyKey;
use status::to_status;

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
    metrics: crate::infrastructure::telemetry::Metrics,
    ping: Ping,
    get_character: GetCharacter,
    create_character: CreateCharacter,
    list_my_characters: ListMyCharacters,
    list_classes: ListClasses,
    change_class: ChangeClass,
    transfer_options: TransferOptions,
}

impl GameServiceImpl {
    /// Builds the service from its use cases.
    #[allow(clippy::too_many_arguments)] // Explicit use-case injection at the composition boundary.
    #[must_use]
    pub fn new(
        metrics: crate::infrastructure::telemetry::Metrics,
        ping: Ping,
        get_character: GetCharacter,
        create_character: CreateCharacter,
        list_my_characters: ListMyCharacters,
        list_classes: ListClasses,
        change_class: ChangeClass,
        transfer_options: TransferOptions,
    ) -> Self {
        Self {
            metrics,
            ping,
            get_character,
            create_character,
            list_my_characters,
            list_classes,
            change_class,
            transfer_options,
        }
    }

    /// Wraps in the tonic server type.
    #[must_use]
    pub fn into_server(self) -> GameServiceServer<Self> {
        GameServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl GameService for GameServiceImpl {
    async fn list_classes(
        &self,
        req: Request<pb::ListClassesRequest>,
    ) -> Result<Response<pb::ListClassesResponse>, Status> {
        auth::caller(&req)?;
        Ok(Response::new(mapping::catalogue_to_pb(&self.list_classes.execute())))
    }

    async fn transfer_options(
        &self,
        req: Request<pb::TransferOptionsRequest>,
    ) -> Result<Response<pb::TransferOptionsResponse>, Status> {
        let caller = auth::caller(&req)?;
        let out = self
            .transfer_options
            .execute(caller, &req.get_ref().character_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(pb::TransferOptionsResponse {
            current_class_id: out.current_class_id.0,
            token_tier_1_count: out.token_tier_1_count,
            token_tier_2_count: out.token_tier_2_count,
            options: out
                .options
                .into_iter()
                .map(|o| pb::TransferOption {
                    class_id: o.class_id.0,
                    eligible: o.eligible,
                    unmet: o.unmet,
                })
                .collect(),
        }))
    }

    async fn change_class(
        &self,
        req: Request<pb::ChangeClassRequest>,
    ) -> Result<Response<pb::ChangeClassResponse>, Status> {
        let caller = auth::caller(&req)?;
        let req = req.into_inner();
        let key = IdempotencyKey::parse(&req.idempotency_key)
            .map_err(|e| to_status(crate::application::AppError::InvalidArgument(e.to_string())))?;
        let started = std::time::Instant::now();
        let result = self
            .change_class
            .execute(caller, &req.character_id, req.target_class_id, key)
            .await;
        self.metrics
            .record_class_transfer_latency(started.elapsed(), result.is_ok());
        let out = result.map_err(to_status)?;
        Ok(Response::new(mapping::transfer_result_to_pb(&out)))
    }

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
        let caller = auth::caller(&req)?;
        let c = self
            .get_character
            .execute(caller, &req.get_ref().character_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::character_to_pb(&c)))
    }

    async fn create_character(
        &self,
        req: Request<CreateCharacterRequest>,
    ) -> Result<Response<Character>, Status> {
        let caller = auth::caller(&req)?;
        // `account_id` on the wire is deprecated and ignored: the owner is the caller.
        let req = req.into_inner();
        let idempotency_key = IdempotencyKey::parse(&req.idempotency_key)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let race = mapping::race_from_pb(req.race)
            .ok_or_else(|| Status::invalid_argument("race must be specified"))?;
        let appearance =
            mapping::appearance_from_pb(req.sex, req.hair_style, req.hair_color, req.face)
                .map_err(to_status)?;
        let c = self
            .create_character
            .execute(CreateCharacterInput {
                idempotency_key,
                account_id: caller,
                name: req.name,
                race,
                base_class_id: req.base_class_id.map(crate::domain::class::ClassId),
                appearance,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::character_to_pb(&c)))
    }

    async fn list_my_characters(
        &self,
        req: Request<ListMyCharactersRequest>,
    ) -> Result<Response<ListMyCharactersResponse>, Status> {
        let caller = auth::caller(&req)?;
        let characters = self
            .list_my_characters
            .execute(caller)
            .await
            .map_err(to_status)?;
        Ok(Response::new(ListMyCharactersResponse {
            characters: characters.iter().map(mapping::character_to_pb).collect(),
        }))
    }
}

/// Convenience for the composition root and tests: an `Arc`-free way to hand over the service.
pub type SharedService = Arc<GameServiceImpl>;
