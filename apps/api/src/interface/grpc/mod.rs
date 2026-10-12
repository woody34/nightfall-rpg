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
    ChangeClass, CreateCharacter, CreateCharacterInput, GetCharacter, GetSkillCatalog, LearnSkill,
    LearnSkillInput, ListClasses, ListLearnableSkills, ListMyCharacters, Ping, TransferOptions,
};
use crate::application::{AppError, IdempotencyKey};
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
    phase3: Option<Phase3UseCases>,
    get_item_catalog: crate::application::use_cases::GetItemCatalog,
    get_inventory: Option<crate::application::use_cases::GetInventory>,
    dev_item_grants: bool,
    grant_item: Option<crate::application::use_cases::GrantItem>,
}

/// Phase 3 use cases shared by live composition and transport tests.
pub struct Phase3UseCases {
    /// Live actor-owned eligibility.
    pub list_learnable_skills: ListLearnableSkills,
    /// Exact-rank paid learning with durable receipts.
    pub learn_skill: LearnSkill,
    /// Immutable validated metadata.
    pub get_skill_catalog: GetSkillCatalog,
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
            phase3: None,
            get_item_catalog: crate::application::use_cases::GetItemCatalog::new(None),
            get_inventory: None,
            dev_item_grants: false,
            grant_item: None,
        }
    }

    /// Installs the actor-backed Phase 3 use cases after checkpoint persistence is wired.
    #[must_use]
    pub fn with_phase3(mut self, use_cases: Phase3UseCases) -> Self {
        self.phase3 = Some(use_cases);
        self
    }

    /// Installs a startup-validated catalogue without enabling live item mutations.
    #[must_use]
    pub fn with_item_catalogue(
        mut self,
        use_case: crate::application::use_cases::GetItemCatalog,
    ) -> Self {
        self.get_item_catalog = use_case;
        self
    }

    /// Installs retained reads through the same lifecycle barrier used by live sessions.
    #[must_use]
    pub fn with_get_inventory(
        mut self,
        use_case: crate::application::use_cases::GetInventory,
    ) -> Self {
        self.get_inventory = Some(use_case);
        self
    }

    /// Installs the typed authenticated actor materialization use case.
    #[must_use]
    pub fn with_grant_item(mut self, use_case: crate::application::use_cases::GrantItem) -> Self {
        self.grant_item = Some(use_case);
        self
    }

    /// Shares the exact startup auth setting; a missing grant runtime still fails closed.
    #[must_use]
    pub fn with_dev_item_grants(mut self, enabled: bool) -> Self {
        self.dev_item_grants = enabled;
        self
    }

    /// Wraps in the tonic server type.
    #[must_use]
    pub fn into_server(self) -> GameServiceServer<Self> {
        GameServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl GameService for GameServiceImpl {
    async fn get_item_catalog(
        &self,
        req: Request<pb::GetItemCatalogRequest>,
    ) -> Result<Response<pb::GetItemCatalogResponse>, Status> {
        auth::caller(&req)?;
        let rules = self.get_item_catalog.execute().map_err(to_status)?;
        let response = mapping::item_catalogue_to_pb(&rules).map_err(to_status)?;
        Ok(Response::new(response))
    }

    async fn get_inventory(
        &self,
        req: Request<pb::GetInventoryRequest>,
    ) -> Result<Response<pb::GetInventoryResponse>, Status> {
        let caller = auth::caller(&req)?;
        let use_case = self.get_inventory.as_ref().ok_or_else(|| {
            to_status(AppError::Unavailable("inventory zone is unavailable".into()))
        })?;
        let result = use_case
            .execute(caller, &req.get_ref().character_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(pb::GetInventoryResponse {
            inventory: Some(mapping::detached_inventory_to_pb(&result).map_err(to_status)?),
        }))
    }

    async fn grant_item(
        &self,
        req: Request<pb::GrantItemRequest>,
    ) -> Result<Response<pb::GrantItemResponse>, Status> {
        let caller = auth::caller(&req)?;
        crate::application::use_cases::grant_item::require_dev_grants(self.dev_item_grants)
            .map_err(to_status)?;
        let use_case = self.grant_item.as_ref().ok_or_else(|| {
            to_status(AppError::Unavailable("item runtime is unavailable".into()))
        })?;
        let body = req.into_inner();
        let receipt = use_case
            .execute(crate::application::use_cases::GrantItemInput {
                caller,
                character_id: body.character_id,
                template_id: body.template_id,
                count: body.count,
                idempotency_key: body.idempotency_key,
            })
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::grant_receipt_to_pb(&receipt).map_err(to_status)?))
    }

    async fn list_learnable_skills(
        &self,
        req: Request<pb::ListLearnableSkillsRequest>,
    ) -> Result<Response<pb::ListLearnableSkillsResponse>, Status> {
        let caller = auth::caller(&req)?;
        let use_cases = self.phase3.as_ref().ok_or_else(|| {
            to_status(AppError::Unavailable("phase 3 runtime is not yet available".into()))
        })?;
        let out = use_cases
            .list_learnable_skills
            .execute(caller, &req.get_ref().character_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(mapping::learnable_skills_to_pb(&out)))
    }

    async fn learn_skill(
        &self,
        req: Request<pb::LearnSkillRequest>,
    ) -> Result<Response<pb::LearnSkillResponse>, Status> {
        let caller = auth::caller(&req)?;
        let started = std::time::Instant::now();
        let req = req.into_inner();
        let result = match &self.phase3 {
            Some(use_cases) => {
                use_cases
                    .learn_skill
                    .execute(LearnSkillInput {
                        caller,
                        character_id: req.character_id,
                        skill_id: req.skill_id,
                        skill_level: req.skill_level,
                        idempotency_key: req.idempotency_key,
                    })
                    .await
            },
            None => Err(AppError::Unavailable("phase 3 runtime is not yet available".into())),
        };
        self.metrics.record_skill_learn_request(
            started.elapsed(),
            crate::infrastructure::telemetry::SkillLearnOutcome::from_result(&result),
        );
        let out = result.map_err(to_status)?;
        Ok(Response::new(mapping::learn_result_to_pb(&out)))
    }

    async fn get_skill_catalog(
        &self,
        req: Request<pb::GetSkillCatalogRequest>,
    ) -> Result<Response<pb::GetSkillCatalogResponse>, Status> {
        auth::caller(&req)?;
        let use_cases = self.phase3.as_ref().ok_or_else(|| {
            to_status(AppError::Unavailable("phase 3 catalogue is not yet available".into()))
        })?;
        let out = use_cases.get_skill_catalog.execute().map_err(to_status)?;
        Ok(Response::new(mapping::skill_catalogue_to_pb(&out)))
    }

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

impl Phase3UseCases {
    /// Compatibility entry point for explicitly constructed V5 transport fixtures.
    pub async fn experimental(
        characters: Arc<dyn crate::application::CharacterRepository>,
        sessions: Arc<crate::application::session::SessionContext>,
    ) -> anyhow::Result<Self> {
        Self::for_runtime(characters, sessions).await
    }

    /// Builds real use cases for a V5 actor with an installed atomic learning lane.
    pub async fn for_runtime(
        characters: Arc<dyn crate::application::CharacterRepository>,
        sessions: Arc<crate::application::session::SessionContext>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(sessions.zone.phase3_enabled(), "V5 zone required");
        let lane = sessions
            .zone
            .checkpoints
            .service()
            .ok_or_else(|| anyhow::anyhow!("checkpoint lane required"))?;
        anyhow::ensure!(
            lane.lock().await.learning_enabled(),
            "atomic learning persistence required"
        );
        let snapshot = sessions.zone.snapshot().await?;
        let phase = snapshot
            .phase3
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Phase3 rules missing"))?;
        let classes = snapshot
            .classes
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("class registry missing"))?;
        let catalogue = Arc::new(
            crate::application::use_cases::get_skill_catalog::RegistrySkillCatalogue::new(
                classes,
                phase.rules.skills(),
            )?,
        );
        Ok(Self {
            list_learnable_skills: ListLearnableSkills::new(
                characters.clone(),
                Some(sessions.clone()),
            ),
            learn_skill: LearnSkill::new(
                characters,
                sessions.clone(),
                catalogue.clone(),
                Some(sessions),
            ),
            get_skill_catalog: GetSkillCatalog::new(Some(catalogue)),
        })
    }
}
