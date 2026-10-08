//! Telemetry: structured logs, OpenTelemetry traces and metrics, and request correlation.
//!
//! [`init`] installs the global `tracing` subscriber and builds the [`Metrics`] catalogue.
//! Nothing is exported over the network unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set, so tests
//! and dependency-free dev run without a collector.

mod layers;
mod metrics;

use std::time::Duration;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter, WithExportConfig};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use parking_lot::Mutex;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Lives in the application layer, which carries it on zone inputs; re-exported here because
/// it is the telemetry contract for crossing queues.
pub use crate::application::trace::TraceCarrier;
pub use layers::{
    http_metrics, http_trace_layer, metrics_handler, record_account_id, request_id_layers,
    GrpcTelemetryLayer, UuidV7RequestId,
};
pub use metrics::{record_tick_stats, FrameDirection, Metrics, OutboxStats, OutboxStatsSource};

const DEFAULT_FILTER: &str = "info,tower_http=debug,sqlx=warn";
const METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(10);

/// The log/span filter: `spec` (normally `RUST_LOG`) or the default, with credential-bearing
/// dependencies capped at `info` whatever `spec` says. `async_nats` traces the raw `CONNECT`
/// frame, password included, so it must stay below that level in every build.
#[must_use]
pub fn log_filter(spec: Option<&str>) -> EnvFilter {
    let base = spec
        .and_then(|s| EnvFilter::try_new(s).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_FILTER));
    // The more specific target is listed too: an explicit `async_nats::connection=trace` in
    // `spec` would otherwise outrank the crate-wide cap.
    ["async_nats=info", "async_nats::connection=info"]
        .into_iter()
        .filter_map(|d| d.parse().ok())
        .fold(base, EnvFilter::add_directive)
}

/// What [`init`] needs. Read from the environment by [`TelemetryConfig::from_env`].
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// OTLP gRPC endpoint such as `http://localhost:4317`. `None` disables export.
    pub otlp_endpoint: Option<String>,
    /// `service.name` resource attribute.
    pub service_name: String,
    /// Human-readable stdout logs instead of JSON.
    pub pretty: bool,
}

impl TelemetryConfig {
    /// Reads `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME` and `TELEMETRY_PRETTY`.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
            service_name: std::env::var("OTEL_SERVICE_NAME")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "nightfall-api".into()),
            pretty: std::env::var("TELEMETRY_PRETTY").is_ok_and(|v| v == "1"),
        }
    }

    /// No export, pretty logs: for unit tests.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            otlp_endpoint: None,
            service_name: "nightfall-api".into(),
            pretty: true,
        }
    }
}

/// Owns the exporters. Pending spans and metrics are flushed when it is dropped; prefer
/// [`TelemetryGuard::shutdown`] from async code so the flush does not block a runtime thread.
pub struct TelemetryGuard {
    metrics: Metrics,
    tracer_provider: Mutex<Option<SdkTracerProvider>>,
    logger_provider: Mutex<Option<SdkLoggerProvider>>,
    exports_metrics: bool,
}

impl TelemetryGuard {
    /// The shared metric catalogue.
    #[must_use]
    pub fn metrics(&self) -> Metrics {
        self.metrics.clone()
    }

    /// Flushes and stops the exporters on a dedicated thread, so waiting on the collector
    /// never blocks a runtime worker.
    pub async fn shutdown(self) {
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("telemetry-shutdown".into())
            .spawn(move || {
                drop(self);
                done_tx.send(()).ok();
            });
        match spawned {
            Ok(_) => {
                done_rx.await.ok();
            },
            Err(e) => tracing::warn!(error = %e, "telemetry shutdown thread failed to start"),
        }
    }

    fn flush(&self) {
        if let Some(tp) = self.tracer_provider.lock().take() {
            if let Err(e) = tp.shutdown() {
                tracing::warn!(error = %e, "trace exporter shutdown failed");
            }
        }
        if let Some(lp) = self.logger_provider.lock().take() {
            if let Err(e) = lp.shutdown() {
                tracing::warn!(error = %e, "log exporter shutdown failed");
            }
        }
        if self.exports_metrics {
            if let Err(e) = self.metrics.provider().shutdown() {
                tracing::warn!(error = %e, "metrics exporter shutdown failed");
            }
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Installs the global subscriber and builds the metric catalogue.
///
/// A second call in the same process (parallel tests) keeps the first subscriber and still
/// returns a working guard with its own metrics.
pub fn init(cfg: &TelemetryConfig) -> anyhow::Result<TelemetryGuard> {
    let resource = Resource::builder()
        .with_service_name(cfg.service_name.clone())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();

    let registry = prometheus::Registry::new();
    let prom_reader = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()?;
    let mut meter_builder = SdkMeterProvider::builder()
        .with_resource(resource.clone())
        .with_reader(prom_reader);

    let mut tracer_provider = None;
    let mut logger_provider = None;
    if let Some(endpoint) = &cfg.otlp_endpoint {
        let span_exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.clone())
            .build()?;
        tracer_provider = Some(
            SdkTracerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(span_exporter)
                .build(),
        );
        let log_exporter = LogExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.clone())
            .build()?;
        logger_provider = Some(
            SdkLoggerProvider::builder()
                .with_resource(resource.clone())
                .with_batch_exporter(log_exporter)
                .build(),
        );
        let metric_exporter = MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint.clone())
            .build()?;
        meter_builder = meter_builder.with_reader(
            PeriodicReader::builder(metric_exporter)
                .with_interval(METRIC_EXPORT_INTERVAL)
                .build(),
        );
    }
    let metrics = Metrics::new(meter_builder.build(), registry);

    let fmt_layer = if cfg.pretty {
        tracing_subscriber::fmt::layer().pretty().boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .boxed()
    };
    let otel_layer = tracer_provider
        .as_ref()
        .map(|tp| tracing_opentelemetry::layer().with_tracer(tp.tracer("nightfall-api")));
    // Logs reach Loki through the OpenTelemetry logs bridge. The exporter's own transport
    // crates are excluded so exporting a log can never emit another log.
    let log_layer = logger_provider.as_ref().map(|lp| {
        OpenTelemetryTracingBridge::new(lp).with_filter(filter_fn(|meta| {
            let t = meta.target();
            !["hyper", "h2", "tonic", "tower", "reqwest", "opentelemetry"]
                .iter()
                .any(|p| t.starts_with(p))
        }))
    });
    let filter = log_filter(std::env::var("RUST_LOG").ok().as_deref());

    // `try_init` fails only when a subscriber already exists; that is fine and expected in tests.
    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .with(log_layer)
        .with(filter)
        .try_init()
        .ok();

    Ok(TelemetryGuard {
        metrics,
        exports_metrics: cfg.otlp_endpoint.is_some(),
        tracer_provider: Mutex::new(tracer_provider),
        logger_provider: Mutex::new(logger_provider),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn init_without_endpoint_exports_nothing_and_serves_metrics() {
        let guard = init(&TelemetryConfig::disabled()).unwrap();
        let text = guard.metrics().render().unwrap();
        assert!(text.contains("nightfall_ws_dropped_frames_total"), "{text}");
        guard.shutdown().await;
    }

    #[tokio::test]
    async fn init_twice_does_not_fail() {
        let a = init(&TelemetryConfig::disabled()).unwrap();
        let b = init(&TelemetryConfig::disabled()).unwrap();
        drop((a, b));
    }

    #[test]
    fn detached_metrics_render_catalogue_after_recording() {
        let m = Metrics::detached();
        m.tick_duration_seconds.record(0.004, &[]);
        let text = m.render().unwrap();
        assert!(text.contains("nightfall_tick_duration_seconds_bucket"), "{text}");
    }
}
