//! Composition root: read config, choose adapters, start servers.

use std::sync::Arc;

use nightfall_api::config::Config;
use nightfall_api::infrastructure::outbox::{JetStreamPublisher, OutboxRelay};
use nightfall_api::infrastructure::telemetry::{self, TelemetryConfig};
use nightfall_api::{
    bind, build_game_service, infrastructure, serve_grpc, serve_http, Dependencies,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let telemetry = telemetry::init(&TelemetryConfig::from_env())?;

    let result = run(telemetry.metrics()).await;

    // Flush buffered spans and metrics whether or not the servers exited cleanly.
    telemetry.shutdown().await;
    result
}

async fn run(metrics: telemetry::Metrics) -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let shutdown = CancellationToken::new();
    let (mut deps, relay) = build_dependencies(&cfg, metrics.clone(), &shutdown).await?;
    deps.metrics = metrics.clone();
    let service = build_game_service(&deps);

    let (http, grpc) = bind(cfg.http_addr, cfg.grpc_addr).await?;
    tracing::info!(http_addr = %cfg.http_addr, grpc_addr = %cfg.grpc_addr, "nightfall-api starting");

    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        stop_tx.send(true).ok();
    });

    let served = tokio::try_join!(
        serve_http(http, metrics.clone(), stopped(stop_rx.clone())),
        serve_grpc(grpc, service, metrics, stopped(stop_rx)),
    );

    // Servers are down; stop the relay and wait for its in-flight batch.
    shutdown.cancel();
    if let Some(relay) = relay {
        relay.join().await;
    }
    served?;
    tracing::info!("nightfall-api stopped");
    Ok(())
}

/// Resolves once the shutdown flag is set (or its sender is gone).
async fn stopped(mut rx: watch::Receiver<bool>) {
    rx.wait_for(|stop| *stop).await.ok();
}

/// Resolves on ctrl-c or, on unix, SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "cannot listen for ctrl-c");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            },
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            },
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

async fn build_dependencies(
    cfg: &Config,
    metrics: telemetry::Metrics,
    shutdown: &CancellationToken,
) -> anyhow::Result<(Dependencies, Option<OutboxRelay>)> {
    let mut deps = Dependencies::in_memory();
    let mut db = None;

    if let Some(url) = &cfg.database_url {
        let conn = infrastructure::postgres::connect(url).await?;
        deps.characters = Arc::new(
            infrastructure::postgres::PgCharacterRepository::new(conn.clone())
                .with_metrics(metrics),
        );
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
