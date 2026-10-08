//! The metric catalogue (docs/planning/09-live-operations.md §3.4), created once and shared
//! through `Dependencies`.
//!
//! Instruments are OpenTelemetry instruments. The same readings reach Grafana by OTLP push
//! (when `OTEL_EXPORTER_OTLP_ENDPOINT` is set) and a Prometheus scrape on `/metrics`.
//! Names are given without the unit or `_total` suffix; both exporters add them, so
//! `nightfall_tick_duration` + unit `s` is `nightfall_tick_duration_seconds` everywhere.
//!
//! Labels are low-cardinality only: never character, account or session ids.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use parking_lot::RwLock;
use prometheus::{Encoder, Registry, TextEncoder};
use tokio::sync::watch;

use crate::application::session::SessionMetrics;
use crate::application::zone_actor::TickStats;

/// Tick budget is 100 ms; buckets run 0.25 ms to 500 ms, fine at the low end where a healthy
/// zone lives so a p99 can be read off them.
const TICK_BUCKETS: &[f64] = &[
    0.000_25, 0.000_5, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5,
];
/// Database and event-log latencies.
const IO_BUCKETS: &[f64] = &[
    0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 1.0,
];

/// Direction label of `nightfall_ws_frames_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDirection {
    /// Client to server.
    In,
    /// Server to client.
    Out,
}

impl FrameDirection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

/// A snapshot of outbox health, read on every metrics collection.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OutboxStats {
    /// Rows staged but not yet published.
    pub pending: u64,
    /// Age in seconds of the oldest unpublished row; 0 when the outbox is empty.
    pub lag_seconds: f64,
}

/// Hook the outbox relay implements (its `RelayStats` is the intended implementor) and
/// registers with [`Metrics::set_outbox_source`]. Must be cheap and non-blocking: it runs on
/// the metrics collection thread. Back it with atomics the relay updates.
pub trait OutboxStatsSource: Send + Sync {
    /// Current pending count and lag.
    fn snapshot(&self) -> OutboxStats;
}

type SharedSource = Arc<RwLock<Option<Arc<dyn OutboxStatsSource>>>>;

/// All instruments. Cloning is cheap (handles are reference counted).
#[derive(Clone)]
pub struct Metrics {
    /// Wall time of one simulation tick, recorded from the zone actor's `TickStats` by
    /// [`record_tick_stats`].
    pub tick_duration_seconds: Histogram<f64>,
    /// Connected sessions; the session actor moves it on start and end.
    pub sessions_active: UpDownCounter<i64>,
    /// WebSocket frames by direction: counted when read from and written to a socket.
    pub ws_frames_total: Counter<u64>,
    /// Frames dropped because a session's outbound queue was full (the session is then closed
    /// with 4429).
    pub ws_dropped_frames_total: Counter<u64>,
    /// Time to append one record to the event log. No producer yet.
    pub eventlog_publish_seconds: Histogram<f64>,
    db_query_seconds: Histogram<f64>,
    grpc_requests_total: Counter<u64>,
    http_requests_total: Counter<u64>,
    outbox_source: SharedSource,
    registry: Registry,
    provider: SdkMeterProvider,
}

impl Metrics {
    /// Builds the catalogue on `provider`, whose Prometheus reader writes into `registry`.
    pub(super) fn new(provider: SdkMeterProvider, registry: Registry) -> Self {
        let meter = opentelemetry::metrics::MeterProvider::meter(&provider, "nightfall-api");
        let outbox_source: SharedSource = Arc::default();
        register_outbox_gauges(&meter, &outbox_source);

        let metrics = Self {
            tick_duration_seconds: meter
                .f64_histogram("nightfall_tick_duration")
                .with_unit("s")
                .with_description("Wall time of one simulation tick")
                .with_boundaries(TICK_BUCKETS.to_vec())
                .build(),
            sessions_active: meter
                .i64_up_down_counter("nightfall_sessions_active")
                .with_description("Currently connected game sessions")
                .build(),
            ws_frames_total: meter
                .u64_counter("nightfall_ws_frames")
                .with_description("WebSocket frames by direction")
                .build(),
            ws_dropped_frames_total: meter
                .u64_counter("nightfall_ws_dropped_frames")
                .with_description("Frames dropped because a client queue was full")
                .build(),
            eventlog_publish_seconds: meter
                .f64_histogram("nightfall_eventlog_publish")
                .with_unit("s")
                .with_description("Time to publish one record to the event log")
                .with_boundaries(IO_BUCKETS.to_vec())
                .build(),
            db_query_seconds: meter
                .f64_histogram("nightfall_db_query")
                .with_unit("s")
                .with_description("Database query latency by repository and operation")
                .with_boundaries(IO_BUCKETS.to_vec())
                .build(),
            grpc_requests_total: meter
                .u64_counter("nightfall_grpc_requests")
                .with_description("gRPC requests by service, method and status code")
                .build(),
            http_requests_total: meter
                .u64_counter("nightfall_http_requests")
                .with_description("HTTP requests by matched route and status")
                .build(),
            outbox_source,
            registry,
            provider,
        };
        // A counter series that does not exist yet cannot be `increase()`d from zero, so the
        // series an alert depends on are created up front.
        metrics.ws_dropped_frames_total.add(0, &[]);
        metrics.record_ws_frames(FrameDirection::In, 0);
        metrics.record_ws_frames(FrameDirection::Out, 0);
        metrics
    }

    /// Metrics with only the Prometheus reader attached: tests and dependency-free dev.
    ///
    /// The exporter can only fail on a duplicate registration, impossible with a fresh
    /// registry; if it ever does, the instruments still work and `/metrics` is empty.
    #[must_use]
    pub fn detached() -> Self {
        let registry = Registry::new();
        let mut builder = SdkMeterProvider::builder();
        match opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
        {
            Ok(reader) => builder = builder.with_reader(reader),
            Err(e) => tracing::warn!(error = %e, "prometheus exporter unavailable"),
        }
        Self::new(builder.build(), registry)
    }

    /// Registers the outbox relay's stats so `outbox_pending` and `outbox_lag_seconds` report
    /// real values instead of zero.
    pub fn set_outbox_source(&self, source: Arc<dyn OutboxStatsSource>) {
        *self.outbox_source.write() = Some(source);
    }

    /// Counts frames in one direction.
    pub fn record_ws_frames(&self, direction: FrameDirection, count: u64) {
        self.ws_frames_total
            .add(count, &[KeyValue::new("direction", direction.as_str())]);
    }

    /// Runs `fut`, recording its duration in `db_query_seconds{repo,op}`.
    pub async fn time_db<T>(
        &self,
        repo: &'static str,
        op: &'static str,
        fut: impl Future<Output = T>,
    ) -> T {
        let start = Instant::now();
        let out = fut.await;
        self.db_query_seconds.record(
            start.elapsed().as_secs_f64(),
            &[KeyValue::new("repo", repo), KeyValue::new("op", op)],
        );
        out
    }

    /// Counts one finished gRPC request.
    pub(crate) fn record_grpc(&self, service: &str, method: &str, code: &str) {
        self.grpc_requests_total.add(
            1,
            &[
                KeyValue::new("service", service.to_owned()),
                KeyValue::new("method", method.to_owned()),
                KeyValue::new("code", code.to_owned()),
            ],
        );
    }

    /// Counts one finished HTTP request. `route` must be the matched route template.
    pub(crate) fn record_http(&self, route: &str, status: u16) {
        self.http_requests_total.add(
            1,
            &[
                KeyValue::new("route", route.to_owned()),
                KeyValue::new("status", i64::from(status)),
            ],
        );
    }

    /// Prometheus text exposition of the current readings, for `GET /metrics`.
    pub fn render(&self) -> anyhow::Result<String> {
        let mut buf = Vec::new();
        TextEncoder::new().encode(&self.registry.gather(), &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }

    pub(super) fn provider(&self) -> &SdkMeterProvider {
        &self.provider
    }
}

impl SessionMetrics for Metrics {
    fn session_opened(&self) {
        self.sessions_active.add(1, &[]);
    }

    fn session_closed(&self) {
        self.sessions_active.add(-1, &[]);
    }

    fn frames_in(&self, n: u64) {
        self.record_ws_frames(FrameDirection::In, n);
    }

    fn frames_out(&self, n: u64) {
        self.record_ws_frames(FrameDirection::Out, n);
    }

    fn frames_dropped(&self, n: u64) {
        self.ws_dropped_frames_total.add(n, &[]);
    }
}

/// Records every tick's duration from the zone actor's stats watch into
/// `tick_duration_seconds` until the zone stops. A watch keeps only the latest value, so a
/// recorder that falls more than a tick behind skips samples; at 10 Hz it does not.
pub async fn record_tick_stats(mut stats: watch::Receiver<TickStats>, metrics: Metrics) {
    while stats.changed().await.is_ok() {
        let micros = stats.borrow_and_update().duration_micros;
        metrics
            .tick_duration_seconds
            .record(Duration::from_micros(micros).as_secs_f64(), &[]);
    }
}

fn register_outbox_gauges(meter: &Meter, source: &SharedSource) {
    let pending_src = source.clone();
    meter
        .u64_observable_gauge("nightfall_outbox_pending")
        .with_description("Outbox rows staged but not yet published")
        .with_callback(move |obs| {
            let stats = pending_src.read().as_ref().map(|s| s.snapshot());
            obs.observe(stats.unwrap_or_default().pending, &[]);
        })
        .build();
    let lag_src = source.clone();
    meter
        .f64_observable_gauge("nightfall_outbox_lag")
        .with_unit("s")
        .with_description("Age of the oldest unpublished outbox row")
        .with_callback(move |obs| {
            let stats = lag_src.read().as_ref().map(|s| s.snapshot());
            obs.observe(stats.unwrap_or_default().lag_seconds, &[]);
        })
        .build();
}
