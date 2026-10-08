//! Nightfall game server library. `main.rs` is the composition root; everything testable
//! lives here so integration tests can build the same servers in-process.
//!
//! Layers (dependencies point inward only):
//! `interface` -> `application` -> `domain`, with `infrastructure` implementing
//! `application::ports`. See docs/engineering/architecture.md.

pub mod application;
pub mod config;
pub mod domain;
pub mod infrastructure;
pub mod interface;

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use application::use_cases::{
    Authenticate, CreateCharacter, EnsureAccount, GetCharacter, IssuePlayTicket, ListMyCharacters,
    Ping,
};
use application::{
    AccountRepository, CharacterRepository, Clock, EventBus, SecretGenerator, SessionRepository,
    TokenVerifier,
};
use infrastructure::telemetry::{GrpcTelemetryLayer, Metrics};
use interface::grpc::{AuthLayer, GameServiceImpl, SessionServiceImpl};
use tokio::net::TcpListener;

/// Default public WebSocket address returned by `IssuePlayTicket` (`WS_PUBLIC_URL`).
pub const DEFAULT_WS_PUBLIC_URL: &str = "ws://localhost:3000/ws";

/// The ports the server needs, already bound to adapters.
#[derive(Clone)]
pub struct Dependencies {
    /// Character persistence.
    pub characters: Arc<dyn CharacterRepository>,
    /// Account persistence.
    pub accounts: Arc<dyn AccountRepository>,
    /// Play tickets and session generations.
    pub sessions: Arc<dyn SessionRepository>,
    /// Bearer-token verification.
    pub tokens: Arc<dyn TokenVerifier>,
    /// Source of play-ticket secrets.
    pub secrets: Arc<dyn SecretGenerator>,
    /// Outbound events.
    pub bus: Arc<dyn EventBus>,
    /// Wall clock.
    pub clock: Arc<dyn Clock>,
    /// The metric catalogue; `main` replaces it with the exported one.
    pub metrics: Metrics,
    /// Public WebSocket URL handed to clients with each play ticket.
    pub ws_public_url: String,
}

impl Dependencies {
    /// All in-memory adapters. Used by tests and by `main` when no URLs are configured.
    ///
    /// Tokens are verified by [`infrastructure::auth::DisabledVerifier`], which rejects
    /// everything; tests and dev setups swap in a real or test verifier explicitly.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            characters: Arc::new(infrastructure::memory::InMemoryCharacterRepository::default()),
            accounts: Arc::new(infrastructure::memory::InMemoryAccountRepository::default()),
            sessions: Arc::new(infrastructure::memory::InMemorySessionRepository::default()),
            tokens: Arc::new(infrastructure::auth::DisabledVerifier),
            secrets: Arc::new(infrastructure::secrets::OsSecretGenerator::default()),
            bus: Arc::new(infrastructure::memory::InMemoryEventBus::default()),
            clock: Arc::new(infrastructure::SystemClock),
            metrics: Metrics::detached(),
            ws_public_url: DEFAULT_WS_PUBLIC_URL.to_owned(),
        }
    }
}

/// Everything the tonic server mounts: the services and the authentication layer in front
/// of them.
pub struct GrpcServices {
    /// `GameService`.
    pub game: GameServiceImpl,
    /// `SessionService`.
    pub session: SessionServiceImpl,
    /// Verifies the bearer token of every non-public RPC.
    pub auth: AuthLayer,
}

/// Wires use cases to the gRPC services.
#[must_use]
pub fn build_grpc_services(deps: &Dependencies) -> GrpcServices {
    let authenticate = Authenticate::new(
        deps.tokens.clone(),
        EnsureAccount::new(deps.accounts.clone(), deps.clock.clone()),
    );
    GrpcServices {
        game: GameServiceImpl::new(
            Ping::new(deps.clock.clone()),
            GetCharacter::new(deps.characters.clone()),
            CreateCharacter::new(deps.characters.clone()),
            ListMyCharacters::new(deps.characters.clone()),
        ),
        session: SessionServiceImpl::new(IssuePlayTicket::new(
            deps.characters.clone(),
            deps.sessions.clone(),
            deps.secrets.clone(),
            deps.clock.clone(),
            deps.ws_public_url.clone(),
        )),
        auth: AuthLayer::new(Arc::new(authenticate)),
    }
}

/// Serves HTTP on an already-bound listener until `shutdown` resolves (in-flight requests
/// finish) or the server errors.
pub async fn serve_http(
    listener: TcpListener,
    metrics: Metrics,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, interface::http::router(metrics))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

/// Serves gRPC on an already-bound listener until `shutdown` resolves.
///
/// Layers, outermost first: request id, telemetry span and metrics, authentication. Auth runs
/// inside the span so a rejected call is still traced and counted, and so the span gets the
/// caller's `account_id`.
pub async fn serve_grpc(
    listener: TcpListener,
    services: GrpcServices,
    metrics: Metrics,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    tonic::transport::Server::builder()
        .layer(
            tower::ServiceBuilder::new()
                .layer(infrastructure::telemetry::request_id_layers().0)
                .layer(GrpcTelemetryLayer::new(metrics))
                .layer(services.auth)
                .into_inner(),
        )
        .add_service(services.game.into_server())
        .add_service(services.session.into_server())
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await?;
    Ok(())
}

/// Binds both listeners and returns their actual addresses (port 0 resolves here).
pub async fn bind(
    http: SocketAddr,
    grpc: SocketAddr,
) -> anyhow::Result<(TcpListener, TcpListener)> {
    Ok((TcpListener::bind(http).await?, TcpListener::bind(grpc).await?))
}
