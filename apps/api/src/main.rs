//! Composition root: read config, choose adapters, start servers.

use std::sync::Arc;

use nightfall_api::config::Config;
use nightfall_api::{
    bind, build_game_service, infrastructure, serve_grpc, serve_http, Dependencies,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cfg = Config::from_env()?;
    let deps = build_dependencies(&cfg).await?;
    let service = build_game_service(&deps);

    let (http, grpc) = bind(cfg.http_addr, cfg.grpc_addr).await?;
    tracing::info!(http_addr = %cfg.http_addr, grpc_addr = %cfg.grpc_addr, "nightfall-api starting");

    tokio::try_join!(serve_http(http), serve_grpc(grpc, service))?;
    Ok(())
}

async fn build_dependencies(cfg: &Config) -> anyhow::Result<Dependencies> {
    let mut deps = Dependencies::in_memory();

    if let Some(url) = &cfg.database_url {
        let pool = infrastructure::postgres::connect(url).await?;
        deps.characters = Arc::new(infrastructure::postgres::PgCharacterRepository::new(pool));
    } else {
        tracing::warn!("DATABASE_URL not set: using in-memory persistence (data is lost on exit)");
    }

    if let Some(url) = &cfg.nats_url {
        deps.bus = Arc::new(infrastructure::nats::NatsEventBus::connect(url).await?);
    } else {
        tracing::warn!("NATS_URL not set: using in-memory event bus");
    }

    Ok(deps)
}
