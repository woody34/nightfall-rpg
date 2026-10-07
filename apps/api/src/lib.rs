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

use std::net::SocketAddr;
use std::sync::Arc;

use application::use_cases::{CreateCharacter, GetCharacter, Ping};
use application::{CharacterRepository, Clock, EventBus};
use interface::grpc::GameServiceImpl;
use tokio::net::TcpListener;

/// The ports the server needs, already bound to adapters.
#[derive(Clone)]
pub struct Dependencies {
    /// Character persistence.
    pub characters: Arc<dyn CharacterRepository>,
    /// Outbound events.
    pub bus: Arc<dyn EventBus>,
    /// Wall clock.
    pub clock: Arc<dyn Clock>,
}

impl Dependencies {
    /// All in-memory adapters. Used by tests and by `main` when no URLs are configured.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            characters: Arc::new(infrastructure::memory::InMemoryCharacterRepository::default()),
            bus: Arc::new(infrastructure::memory::InMemoryEventBus::default()),
            clock: Arc::new(infrastructure::SystemClock),
        }
    }
}

/// Wires use cases to the gRPC service.
#[must_use]
pub fn build_game_service(deps: &Dependencies) -> GameServiceImpl {
    GameServiceImpl::new(
        Ping::new(deps.clock.clone()),
        GetCharacter::new(deps.characters.clone()),
        CreateCharacter::new(deps.characters.clone(), deps.bus.clone()),
    )
}

/// Serves HTTP on an already-bound listener until the task is dropped or the server errors.
pub async fn serve_http(listener: TcpListener) -> anyhow::Result<()> {
    axum::serve(listener, interface::http::router()).await?;
    Ok(())
}

/// Serves gRPC on an already-bound listener.
pub async fn serve_grpc(listener: TcpListener, service: GameServiceImpl) -> anyhow::Result<()> {
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    tonic::transport::Server::builder()
        .add_service(service.into_server())
        .serve_with_incoming(incoming)
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
