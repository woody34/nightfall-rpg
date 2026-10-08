//! Composition root: read config, choose adapters, start servers.

use std::sync::Arc;

use nightfall_api::config::Config;
use nightfall_api::infrastructure::outbox::{JetStreamPublisher, OutboxRelay};
use nightfall_api::{
    bind, build_game_service, infrastructure, serve_grpc, serve_http, Dependencies,
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = Config::from_env()?;
    let shutdown = CancellationToken::new();
    let (deps, relay) = build_dependencies(&cfg, &shutdown).await?;
    let service = build_game_service(&deps);

    let (http, grpc) = bind(cfg.http_addr, cfg.grpc_addr).await?;
    tracing::info!(http_addr = %cfg.http_addr, grpc_addr = %cfg.grpc_addr, "nightfall-api starting");

    let served = tokio::try_join!(serve_http(http), serve_grpc(grpc, service));
    shutdown.cancel();
    if let Some(relay) = relay {
        relay.join().await;
    }
    served?;
    Ok(())
}

async fn build_dependencies(
    cfg: &Config,
    shutdown: &CancellationToken,
) -> anyhow::Result<(Dependencies, Option<OutboxRelay>)> {
    let mut deps = Dependencies::in_memory();
    let mut db = None;

    if let Some(url) = &cfg.database_url {
        let conn = infrastructure::postgres::connect(url).await?;
        deps.characters =
            Arc::new(infrastructure::postgres::PgCharacterRepository::new(conn.clone()));
        db = Some(conn);
    } else {
        tracing::warn!("DATABASE_URL not set: using in-memory persistence (data is lost on exit)");
    }

    let mut relay = None;
    if let Some(url) = &cfg.nats_url {
        deps.bus = Arc::new(infrastructure::nats::NatsEventBus::connect(url).await?);
        if let Some(db) = db {
            // The relay is the only publisher of domain events: acknowledged JetStream publish,
            // deduplicated by outbox row id.
            let client = async_nats::connect(url).await?;
            let publisher = JetStreamPublisher::connect(client).await?;
            relay = Some(OutboxRelay::spawn(db, Arc::new(publisher), shutdown.clone()));
            tracing::info!("outbox relay started");
        }
    } else {
        tracing::warn!("NATS_URL not set: using in-memory event bus");
    }

    Ok((deps, relay))
}
