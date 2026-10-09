//! Composition root: read config, choose adapters, start servers.

use std::sync::Arc;
use std::time::Duration;

use nightfall_api::application::replay_log::{WatermarkReason, AUDIT_BUFFER};
use nightfall_api::application::zone_registry::ZoneRegistry;
use nightfall_api::config::Config;
use nightfall_api::infrastructure::auth::{KeycloakVerifier, OidcConfig};
use nightfall_api::infrastructure::eventlog::{
    EventLogSessionAudit, JetStreamEventLog, SessionAuditDrain,
};
use nightfall_api::infrastructure::memory::TestTokenVerifier;
use nightfall_api::infrastructure::outbox::{JetStreamPublisher, OutboxRelay};
use nightfall_api::infrastructure::telemetry::{self, TelemetryConfig};
use nightfall_api::zone_runtime::{self, ZoneRuntimeConfig};
use nightfall_api::{
    bind, build_grpc_services, build_http_router, infrastructure, serve_grpc, serve_http,
    start_realtime, Dependencies,
};
use sea_orm::DatabaseConnection;
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
    let (mut deps, relay, db, audit_drain) =
        build_dependencies(&cfg, metrics.clone(), &shutdown).await?;
    deps.metrics = metrics.clone();
    // A new zone epoch every start (plan §8 #5); closed with a watermark on the way out.
    let zone =
        zone_runtime::start(&ZoneRuntimeConfig::from_env(), cfg.nats_url.as_deref(), db, &deps)
            .await?;
    let services = build_grpc_services(&deps);

    let zones = ZoneRegistry::from_handle(zone.handle().clone());
    let sessions_shutdown = CancellationToken::new();
    let realtime = start_realtime(&deps, zones, sessions_shutdown.clone());
    let router = build_http_router(&deps, &realtime);

    let (http, grpc) = bind(cfg.http_addr, cfg.grpc_addr).await?;
    tracing::info!(http_addr = %cfg.http_addr, grpc_addr = %cfg.grpc_addr, "nightfall-api starting");

    let (stop_tx, stop_rx) = watch::channel(false);
    let signal_sessions_shutdown = sessions_shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, draining");
        // Upgraded sockets are not drained by the HTTP server; close them (1001) now.
        signal_sessions_shutdown.cancel();
        stop_tx.send(true).ok();
    });

    let served = tokio::try_join!(
        serve_http(http, router, stopped(stop_rx.clone())),
        serve_grpc(grpc, services, metrics, stopped(stop_rx)),
    );

    // Also close upgraded sockets on a server error. HTTP does not await these tasks.
    sessions_shutdown.cancel();
    if tokio::time::timeout(Duration::from_secs(5), async {
        while !realtime.sessions.registry.is_empty().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_err()
    {
        tracing::warn!("session drain timed out before audit flush");
    }
    // Sessions stopped producing frames; close the zone epoch, then flush audit and relay.
    if let Err(e) = zone.shutdown(WatermarkReason::Shutdown).await {
        tracing::error!(error = %e, "zone epoch left incomplete");
    }
    shutdown.cancel();
    if let Some(drain) = audit_drain {
        drain.shutdown().await;
    }
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
) -> anyhow::Result<(
    Dependencies,
    Option<OutboxRelay>,
    Option<DatabaseConnection>,
    Option<SessionAuditDrain>,
)> {
    let mut deps = Dependencies::in_memory();
    deps.ws_public_url.clone_from(&cfg.ws_public_url);
    deps.session_limits.max_sessions_per_ip = cfg.ws_max_sessions_per_ip;
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
    let mut audit_drain = None;
    if let Some(url) = &cfg.nats_url {
        let bus = infrastructure::nats::NatsEventBus::connect(url).await?;
        let client = bus.client().clone();
        deps.bus = Arc::new(bus);
        // When using the relay, narrow legacy NF_EVENTS before creating replay streams.
        // Otherwise connect rejects any overlapping stream instead of publishing without acks.
        // No audit worker, checkpoint save or socket runs until stream creation succeeds.
        let publisher = if db.is_some() {
            Some(JetStreamPublisher::connect(client.clone()).await?)
        } else {
            None
        };
        let log = JetStreamEventLog::connect(client.clone(), metrics.clone()).await?;
        let (audit, drain) = EventLogSessionAudit::spawn(
            Arc::new(log),
            deps.clock.clone(),
            Arc::new(metrics.clone()),
            AUDIT_BUFFER,
        );
        audit_drain = Some(drain);
        deps.audit = Arc::new(infrastructure::eventlog::CheckpointAudit::spawn(
            client.clone(),
            Arc::new(audit),
            Arc::new(metrics.clone()),
        ));
        if let (Some(db), Some(publisher)) = (db.clone(), publisher) {
            // The relay is the only publisher of domain events: acknowledged JetStream publish,
            // deduplicated by outbox row id.
            relay = Some(OutboxRelay::spawn(db, Arc::new(publisher), &metrics, shutdown.clone()));
            tracing::info!("outbox relay started");
        }
    } else {
        tracing::warn!("NATS_URL not set: using in-memory event bus");
        tracing::warn!("NATS_URL not set: session audit frames are discarded");
    }

    Ok((deps, relay, db, audit_drain))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::StreamExt as _;
    use nightfall_api::application::checkpoint::CheckpointAck;
    use nightfall_api::application::replay_log::{SessionInRecord, SessionOutRecord};
    use nightfall_api::application::{IdempotencyKey, SessionAuditContext};
    use nightfall_api::domain::zone::{Tick, ZoneId};
    use nightfall_api::domain::{CharacterId, SessionId};
    use nightfall_api::infrastructure::eventlog::{
        session_subject, RETENTION, SESSIONS_STREAM, ZONES_STREAM,
    };
    use nightfall_api::infrastructure::outbox::EVENTS_STREAM;
    use uuid::Uuid;

    fn config(nats_url: Option<String>) -> Config {
        Config {
            http_addr: "127.0.0.1:0".parse().unwrap(),
            grpc_addr: "127.0.0.1:0".parse().unwrap(),
            database_url: None,
            nats_url,
            oidc_issuer: None,
            oidc_audience: "nightfall".into(),
            auth_dev_tokens: true,
            ws_public_url: "ws://127.0.0.1:3000/ws".into(),
            ws_max_sessions_per_ip: 10,
        }
    }

    #[tokio::test]
    async fn missing_nats_keeps_the_discard_default_and_starts_no_audit_worker() {
        let (deps, relay, db, drain) = build_dependencies(
            &config(None),
            telemetry::Metrics::detached(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(relay.is_none());
        assert!(db.is_none());
        assert!(drain.is_none());
        // The composition root remains usable in dependency-free dev.
        deps.audit
            .record_in(SessionId::new(), None, &Bytes::from_static(b"invalid"));
        deps.audit
            .record_out(SessionId::new(), &Bytes::from_static(b"out"));
    }

    #[tokio::test]
    async fn nats_wiring_creates_disjoint_streams_before_audit_and_preserves_checkpoints() {
        let Ok(url) = std::env::var("NATS_URL") else {
            return;
        };
        let client = async_nats::connect(&url).await.unwrap();
        // Match existing adapter tests: NF_EVENTS may exist from an older dev stack.
        JetStreamPublisher::connect(client.clone()).await.unwrap();
        let (deps, relay, db, drain) = build_dependencies(
            &config(Some(url)),
            telemetry::Metrics::detached(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert!(relay.is_none());
        assert!(db.is_none());
        let context = async_nats::jetstream::new(client.clone());
        let mut stream = context.get_stream(SESSIONS_STREAM).await.unwrap();
        let info = stream.info().await.unwrap();
        assert_eq!(info.config.subjects, vec!["nightfall.session.*.*"]);
        assert_eq!(info.config.storage, async_nats::jetstream::stream::StorageType::File);
        assert_eq!(info.config.max_age, RETENTION);
        for (name, subjects) in [
            (ZONES_STREAM, vec!["nightfall.zone.*.*.*"]),
            (EVENTS_STREAM, vec!["nightfall.*.*"]),
        ] {
            assert_eq!(
                context
                    .get_stream(name)
                    .await
                    .unwrap()
                    .info()
                    .await
                    .unwrap()
                    .config
                    .subjects,
                subjects
            );
        }
        let session = SessionId::new();
        let checkpoint_subject = format!("nightfall.session.{session}.checkpoint");
        let mut sub = client.subscribe(checkpoint_subject.clone()).await.unwrap();
        client.flush().await.unwrap();
        let position = SessionAuditContext {
            zone: ZoneId(7),
            epoch: 8,
            tick: Tick(9),
        };
        let incoming = Bytes::from_static(b"raw-in");
        let outgoing = Bytes::from_static(b"raw-out");
        deps.audit
            .record_in_context(session, Some(3), &incoming, position);
        deps.audit.record_out_context(session, &outgoing, position);
        let ack = CheckpointAck {
            character_id: CharacterId::new(),
            key: IdempotencyKey::from_uuid(Uuid::now_v7()),
            revision: 7,
        };
        deps.audit.record_checkpoint(session, &ack);
        drain.unwrap().shutdown().await;
        let stored_in = stream
            .get_last_raw_message_by_subject(&session_subject(session.as_uuid(), "in"))
            .await
            .unwrap();
        let stored_out = stream
            .get_last_raw_message_by_subject(&session_subject(session.as_uuid(), "out"))
            .await
            .unwrap();
        let decoded_in = SessionInRecord::decode(&stored_in.payload).unwrap();
        assert_eq!(decoded_in.frame, incoming);
        assert_eq!(decoded_in.session, session.as_uuid());
        assert_eq!(decoded_in.seq, 3);
        assert_eq!(decoded_in.zone, position.zone);
        assert_eq!(decoded_in.epoch, position.epoch);
        assert_eq!(decoded_in.tick_seen, position.tick);
        assert!(decoded_in.recv_unix_ms > 0);
        assert_eq!(
            SessionOutRecord::decode(&stored_out.payload).unwrap(),
            SessionOutRecord {
                session: session.as_uuid(),
                zone: position.zone,
                epoch: position.epoch,
                tick: position.tick,
                frame: outgoing,
            }
        );
        let published = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.subject.as_str(), checkpoint_subject);
        assert_eq!(serde_json::from_slice::<CheckpointAck>(&published.payload).unwrap(), ack);
        let checkpoint = stream
            .get_last_raw_message_by_subject(&checkpoint_subject)
            .await
            .unwrap();
        assert_eq!(serde_json::from_slice::<CheckpointAck>(&checkpoint.payload).unwrap(), ack);
    }
}
