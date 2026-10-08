//! Shared harness: boots the real HTTP and gRPC servers on ephemeral ports with in-memory
//! adapters, and hands back clients. Every endpoint test goes through a real socket.
//!
//! Callers authenticate with `test:<account_uuid>` bearer tokens (`TestTokenVerifier`):
//! `app.grpc` / `app.session` act as [`ACCOUNT`], `app.game_as(..)` / `app.session_as(..)` as
//! anyone, `app.anon_grpc()` with no token at all.

#![allow(
    dead_code,
    missing_docs,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

pub mod capture;
pub mod pg;
pub mod ws;

use std::sync::Arc;

use nightfall_api::application::zone_actor::{IntervalTicks, ZoneHandle};
use nightfall_api::application::zone_registry::ZoneRegistry;
use nightfall_api::application::{CharacterRepository, TokenVerifier};
use nightfall_api::infrastructure::memory::{
    InMemoryAccountRepository, InMemoryCharacterRepository, InMemoryEventBus, InMemorySessionAudit,
    InMemorySessionRepository, TestTokenVerifier,
};
use nightfall_api::infrastructure::telemetry::Metrics;
use nightfall_api::interface::grpc::pb::game_service_client::GameServiceClient;
use nightfall_api::interface::grpc::pb::session_service_client::SessionServiceClient;
use nightfall_api::{
    bind, build_grpc_services, build_http_router, serve_grpc, serve_http, start_realtime,
    Dependencies,
};
use tokio_util::sync::CancellationToken;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::service::Interceptor;
use tonic::transport::Channel;
use uuid::Uuid;

/// Adds `authorization: Bearer <token>` to every call.
#[derive(Clone)]
pub struct Bearer(Option<MetadataValue<tonic::metadata::Ascii>>);

impl Bearer {
    pub fn token(token: &str) -> Self {
        Self(Some(format!("Bearer {token}").parse().unwrap()))
    }

    pub fn account(account: Uuid) -> Self {
        Self::token(&TestTokenVerifier::token_for(account))
    }
}

impl Interceptor for Bearer {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(v) = &self.0 {
            req.metadata_mut().insert("authorization", v.clone());
        }
        Ok(req)
    }
}

pub type Game = GameServiceClient<InterceptedService<Channel, Bearer>>;
pub type Session = SessionServiceClient<InterceptedService<Channel, Bearer>>;

pub struct TestApp {
    pub http_base: String,
    pub grpc_base: String,
    /// Authenticated as [`ACCOUNT`].
    pub grpc: Game,
    /// Authenticated as [`ACCOUNT`].
    pub session: Session,
    pub characters: Arc<InMemoryCharacterRepository>,
    pub accounts: Arc<InMemoryAccountRepository>,
    pub sessions: Arc<InMemorySessionRepository>,
    pub bus: Arc<InMemoryEventBus>,
    pub metrics: Metrics,
    /// Every session's frames, as the session layer recorded them.
    pub audit: Arc<InMemorySessionAudit>,
    /// The fixture zone `/ws` sessions play in (real 100 ms ticks).
    pub zone: ZoneHandle,
    /// `ws://127.0.0.1:<port>/ws`.
    pub ws_url: String,
    /// Closes every session (1001) when cancelled; cancelled on drop.
    pub sessions_shutdown: CancellationToken,
    channel: Channel,
    /// Server tasks; aborted when the app is dropped.
    http_task: tokio::task::JoinHandle<anyhow::Result<()>>,
    grpc_task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl TestApp {
    pub async fn spawn() -> Self {
        Self::spawn_with(|_| {}).await
    }

    /// Serves `repo` instead of the in-memory repository. `characters` is then an unused
    /// stand-in, so only use this for tests that never look at it (e.g. failure injection).
    pub async fn spawn_with_repo(repo: Arc<dyn CharacterRepository>) -> Self {
        Self::spawn_with(move |deps| deps.characters = repo).await
    }

    /// Like [`TestApp::spawn`], letting the test replace adapters first (a real token
    /// verifier, a fixed secret generator). The in-memory repositories stay observable.
    pub async fn spawn_with(customize: impl FnOnce(&mut Dependencies)) -> Self {
        Self::spawn_with_zone(customize, crate::common::fixture_zone()).await
    }

    pub async fn spawn_with_zone(
        customize: impl FnOnce(&mut Dependencies),
        state: nightfall_api::domain::zone::ZoneState,
    ) -> Self {
        let characters = Arc::new(InMemoryCharacterRepository::default());
        let accounts = Arc::new(InMemoryAccountRepository::default());
        let sessions = Arc::new(InMemorySessionRepository::default());
        let bus = Arc::new(InMemoryEventBus::default());
        let audit = Arc::new(InMemorySessionAudit::default());
        let metrics = Metrics::detached();
        let mut deps = Dependencies::in_memory();
        deps.characters = characters.clone();
        deps.accounts = accounts.clone();
        deps.sessions = sessions.clone();
        deps.bus = bus.clone();
        deps.metrics = metrics.clone();
        deps.tokens = Arc::new(TestTokenVerifier) as Arc<dyn TokenVerifier>;
        deps.audit = audit.clone();
        customize(&mut deps);
        let services = build_grpc_services(&deps);
        let zones = ZoneRegistry::from_handle(
            nightfall_api::application::zone_actor::ZoneActor::spawn(state, IntervalTicks::new()),
        );
        let zone = zones.fixture().clone();
        let sessions_shutdown = CancellationToken::new();
        let realtime = start_realtime(&deps, zones, sessions_shutdown.clone());
        let router = build_http_router(&deps, &realtime);

        let (http, grpc) = bind("127.0.0.1:0".parse().unwrap(), "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let http_addr = http.local_addr().unwrap();
        let grpc_addr = grpc.local_addr().unwrap();

        let http_task = tokio::spawn(serve_http(http, router, std::future::pending()));
        let grpc_task =
            tokio::spawn(serve_grpc(grpc, services, metrics.clone(), std::future::pending()));

        let channel = Channel::from_shared(format!("http://{grpc_addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let me = Bearer::account(account());
        Self {
            http_base: format!("http://{http_addr}"),
            ws_url: format!("ws://{http_addr}/ws"),
            audit,
            zone,
            sessions_shutdown,
            grpc_base: format!("http://{grpc_addr}"),
            grpc: GameServiceClient::with_interceptor(channel.clone(), me.clone()),
            session: SessionServiceClient::with_interceptor(channel.clone(), me),
            characters,
            accounts,
            sessions,
            bus,
            metrics,
            channel,
            http_task,
            grpc_task,
        }
    }

    /// `GameService` as `account`.
    pub fn game_as(&self, account: Uuid) -> Game {
        GameServiceClient::with_interceptor(self.channel.clone(), Bearer::account(account))
    }

    /// `SessionService` as `account`.
    pub fn session_as(&self, account: Uuid) -> Session {
        SessionServiceClient::with_interceptor(self.channel.clone(), Bearer::account(account))
    }

    /// `GameService` with an arbitrary bearer token.
    pub fn game_with_token(&self, token: &str) -> Game {
        GameServiceClient::with_interceptor(self.channel.clone(), Bearer::token(token))
    }

    /// `SessionService` with an arbitrary bearer token.
    pub fn session_with_token(&self, token: &str) -> Session {
        SessionServiceClient::with_interceptor(self.channel.clone(), Bearer::token(token))
    }

    /// `GameService` without any token.
    pub fn anon_grpc(&self) -> Game {
        GameServiceClient::with_interceptor(self.channel.clone(), Bearer(None))
    }

    /// `SessionService` without any token.
    pub fn anon_session(&self) -> Session {
        SessionServiceClient::with_interceptor(self.channel.clone(), Bearer(None))
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.sessions_shutdown.cancel();
        self.http_task.abort();
        self.grpc_task.abort();
    }
}

pub const KEY_A: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8e";
pub const KEY_B: &str = "0190a7e2-6f4c-7c3b-9f1a-3f4a5b6c7d8f";
/// The account `app.grpc` and `app.session` authenticate as.
pub const ACCOUNT: &str = "00000000-0000-0000-0000-000000000001";
/// Somebody else.
pub const OTHER_ACCOUNT: &str = "00000000-0000-0000-0000-000000000002";

pub fn account() -> Uuid {
    Uuid::parse_str(ACCOUNT).unwrap()
}

pub fn other_account() -> Uuid {
    Uuid::parse_str(OTHER_ACCOUNT).unwrap()
}

pub fn fixture_zone() -> nightfall_api::domain::zone::ZoneState {
    nightfall_api::application::zone_registry::fixture_state(1_000_000).unwrap()
}
