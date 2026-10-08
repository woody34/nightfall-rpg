//! Composition root: read config, choose adapters, start servers.

use std::sync::Arc;

use nightfall_api::application::zone_actor::IntervalTicks;
use nightfall_api::application::zone_registry::ZoneRegistry;
use nightfall_api::config::Config;
use nightfall_api::infrastructure::auth::{KeycloakVerifier, OidcConfig};
use nightfall_api::infrastructure::memory::TestTokenVerifier;
use nightfall_api::infrastructure::outbox::{JetStreamPublisher, OutboxRelay};
use nightfall_api::infrastructure::telemetry::{self, TelemetryConfig};
use nightfall_api::{
    bind, build_grpc_services, build_http_router, infrastructure, serve_grpc, serve_http,
    start_realtime, Dependencies,
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
    let services = build_grpc_services(&deps);

    // The fixture zone, in memory, a new epoch per start (Story 3.4's bootstrap replaces this).
    let zones =
        ZoneRegistry::start_fixture(deps.clock.now().timestamp_millis(), IntervalTicks::new())?;
    let sessions_shutdown = CancellationToken::new();
    let realtime = start_realtime(&deps, zones, sessions_shutdown.clone());
    let router = build_http_router(&deps, &realtime);

    let (http, grpc) = bind(cfg.http_addr, cfg.grpc_addr).await?;
    tracing::info!(http_addr = %cfg.http_addr, grpc_addr = %cfg.grpc_addr, "nightfall-api starting");

    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        // Upgraded sockets are not drained by the HTTP server; close them (1001) now.
        sessions_shutdown.cancel();
        stop_tx.send(true).ok();
    });

    let served = tokio::try_join!(
        serve_http(http, router, stopped(stop_rx.clone())),
        serve_grpc(grpc, services, metrics, stopped(stop_rx)),
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
    deps.ws_public_url.clone_from(&cfg.ws_public_url);
    deps.session_limits.max_sessions_per_ip = cfg.ws_max_sessions_per_ip;
    // TODO(plan Story 3.2): the JetStream session audit adapter replaces this.
    tracing::warn!("session audit log not wired yet: per-session frames are not recorded");
    deps.audit = Arc::new(infrastructure::memory::DiscardSessionAudit);
    let mut db = None;

    if let Some(url) = &cfg.database_url {
        let conn = infrastructure::postgres::connect(url).await?;
        deps.characters = Arc::new(
            infrastructure::postgres::PgCharacterRepository::new(conn.clone())
                .with_metrics(metrics.clone()),
        );
        deps.accounts = Arc::new(
            infrastructure::postgres::PgAccountRepository::new(conn.clone())
                .with_metrics(metrics.clone()),
        );
        deps.sessions = Arc::new(
            infrastructure::postgres::PgSessionRepository::new(conn.clone())
                .with_metrics(metrics.clone()),
        );
        db = Some(conn);
    } else {
        tracing::warn!("DATABASE_URL not set: using in-memory persistence (data is lost on exit)");
    }

    match (&cfg.oidc_issuer, cfg.auth_dev_tokens) {
        (_, true) => {
            tracing::warn!("AUTH_DEV_TOKENS=1: accepting unsigned test:<uuid> tokens (dev only)");
            deps.tokens = Arc::new(TestTokenVerifier);
        },
        (Some(issuer), false) => {
            let oidc = OidcConfig {
                issuer: issuer.clone(),
                audience: cfg.oidc_audience.clone(),
            };
            deps.tokens = Arc::new(KeycloakVerifier::connect(&oidc).await?);
        },
        (None, false) => {
            tracing::warn!("OIDC_ISSUER not set: authentication disabled, only Ping is served");
        },
    }

    let mut relay = None;
    if let Some(url) = &cfg.nats_url {
        let bus = infrastructure::nats::NatsEventBus::connect(url).await?;
        let client = bus.client().clone();
        deps.bus = Arc::new(bus);
        if let Some(db) = db {
            // The relay is the only publisher of domain events: acknowledged JetStream publish,
            // deduplicated by outbox row id.
            let publisher = JetStreamPublisher::connect(client).await?;
            relay = Some(OutboxRelay::spawn(db, Arc::new(publisher), &metrics, shutdown.clone()));
            tracing::info!("outbox relay started");
        }
    } else {
        tracing::warn!("NATS_URL not set: using in-memory event bus");
    }

    Ok((deps, relay))
}
