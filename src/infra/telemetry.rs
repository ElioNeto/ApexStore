//! OTLP tracing and engine metrics with OpenTelemetry 0.33 and
//! tracing-opentelemetry 0.34.
//!
//! `OTEL_EXPORTER_OTLP_ENDPOINT` enables gRPC export. An unset or empty value
//! keeps console tracing and disables OTLP metrics. Export timeouts are five
//! seconds and metrics are collected every sixty seconds.
//!
//! # Provider lifecycle
//!
//! Since the OpenTelemetry 0.33 migration, [`init_tracing`] and [`init_metrics`]
//! return `Result<Option<Provider>, TelemetryError>` instead of `()`. Initialize
//! them inside a Tokio/Actix runtime, handle initialization errors, and retain
//! both returned providers. After stopping application work, await [`shutdown`]
//! **before** dropping the runtime, including when application startup fails.
//! Shutdown drains both signals and reports export failures; it moves blocking
//! SDK calls off the runtime thread so tonic can complete its I/O.
//!
//! Embedders that flush providers directly must likewise use `spawn_blocking`
//! for `force_flush()` on a current-thread runtime.

use opentelemetry::global;
use opentelemetry::metrics::{Counter, Meter};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace as sdk_trace;
use opentelemetry_sdk::Resource;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Read `OTEL_EXPORTER_OTLP_ENDPOINT` from the environment.
/// Returns `None` when the variable is unset or empty (telemetry disabled).
fn otlp_endpoint() -> Option<String> {
    let v = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_default();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

// ---------------------------------------------------------------------------
// Tracing
// ---------------------------------------------------------------------------

/// Initialise the tracing subscriber.
///
/// When `OTEL_EXPORTER_OTLP_ENDPOINT` is set, an OTLP exporter for traces is
/// registered as a `tracing` layer alongside `EnvFilter`.
///
/// Otherwise the standard `tracing_subscriber::fmt` layer is used (console).
/// Returns the provider when OTLP is enabled; retain it for [`shutdown`].
/// Exporter configuration and subscriber installation failures are returned.
/// Call once, inside a Tokio/Actix runtime when OTLP is enabled.
pub fn init_tracing() -> Result<Option<sdk_trace::SdkTracerProvider>, TelemetryError> {
    if let Some(endpoint) = otlp_endpoint() {
        let provider = build_tracer_provider(&endpoint)?;
        let tracer = provider.tracer("apexstore");
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        tracing_subscriber::registry()
            .with(filter)
            .with(telemetry_layer)
            .try_init()
            .map_err(|error| TelemetryError::Subscriber(Box::new(error)))?;
        global::set_tracer_provider(provider.clone());
        Ok(Some(provider))
    } else {
        // Fallback: standard console logging.
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_target(false)
            .with_level(true)
            .try_init()
            .map_err(TelemetryError::Subscriber)?;
        Ok(None)
    }
}

fn service_resource() -> Resource {
    Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", "apexstore"),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ])
        .build()
}

fn build_tracer_provider(endpoint: &str) -> Result<sdk_trace::SdkTracerProvider, TelemetryError> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()
        .map_err(TelemetryError::TraceExporter)?;

    Ok(sdk_trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(service_resource())
        .with_sampler(sdk_trace::Sampler::AlwaysOn)
        .build())
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Lazily-initialised OTel meter. Populated only when OTLP is enabled.
static OTEL_METER: std::sync::OnceLock<Meter> = std::sync::OnceLock::new();

/// Returns the global OTel `Meter` if OTLP metrics have been initialised.
pub fn otel_meter() -> Option<&'static Meter> {
    OTEL_METER.get()
}

/// Initialise the OpenTelemetry metrics pipeline (no-op when OTLP is not
/// configured). Retain the provider to flush and shut down before runtime exit.
pub fn init_metrics() -> Result<Option<SdkMeterProvider>, TelemetryError> {
    let Some(endpoint) = otlp_endpoint() else {
        return Ok(None);
    };
    let provider = build_meter_provider(&endpoint)?;
    global::set_meter_provider(provider.clone());
    let _ = OTEL_METER.set(global::meter("apexstore"));
    Ok(Some(provider))
}

fn build_meter_provider(endpoint: &str) -> Result<SdkMeterProvider, TelemetryError> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(5))
        .build()
        .map_err(TelemetryError::MetricExporter)?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(60))
        .build();
    Ok(SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(service_resource())
        .build())
}

/// Errors initializing or draining the telemetry pipelines.
#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("failed to build OTLP trace exporter: {0}")]
    TraceExporter(#[source] opentelemetry_otlp::ExporterBuildError),
    #[error("failed to build OTLP metrics exporter: {0}")]
    MetricExporter(#[source] opentelemetry_otlp::ExporterBuildError),
    #[error("failed to install tracing subscriber: {0}")]
    Subscriber(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("telemetry shutdown failed (traces: {traces:?}, metrics: {metrics:?})")]
    Shutdown {
        traces: Option<opentelemetry_sdk::error::OTelSdkError>,
        metrics: Option<opentelemetry_sdk::error::OTelSdkError>,
    },
    #[error("telemetry shutdown task failed: {0}")]
    ShutdownTask(#[from] tokio::task::JoinError),
}

/// Export buffered telemetry and shut down both providers before the runtime exits.
///
/// The SDK shutdown calls block. Run them off the Actix current-thread runtime
/// so that the tonic channel can finish its I/O while shutdown waits for it.
/// Both signals are shut down even if one exporter fails.
pub async fn shutdown(
    traces: Option<sdk_trace::SdkTracerProvider>,
    metrics: Option<SdkMeterProvider>,
) -> Result<(), TelemetryError> {
    tokio::task::spawn_blocking(move || {
        let traces = traces.and_then(|provider| {
            // BatchSpanProcessor::shutdown does not propagate its final export
            // result. Flush explicitly so a rejected batch is reported, and
            // always shut down the worker even when that flush fails.
            let flush = provider.force_flush();
            let shutdown = provider.shutdown();
            flush.and(shutdown).err()
        });
        let metrics = metrics.and_then(|provider| provider.shutdown().err());
        if traces.is_some() || metrics.is_some() {
            Err(TelemetryError::Shutdown { traces, metrics })
        } else {
            Ok(())
        }
    })
    .await?
}

// ---------------------------------------------------------------------------
// OTel instruments — lightweight counter handles for EngineMetrics
// ---------------------------------------------------------------------------

/// A set of OpenTelemetry `Counter` instruments mirroring every counter in
/// `EngineMetrics`. Created by [`OtelInstruments::try_register`].
#[derive(Debug)]
pub struct OtelInstruments {
    pub sets: Counter<u64>,
    pub gets: Counter<u64>,
    pub deletes: Counter<u64>,
    pub scans: Counter<u64>,
    pub batch_sets: Counter<u64>,
    pub batch_deletes: Counter<u64>,
    pub flushes: Counter<u64>,
    pub compactions: Counter<u64>,
    pub set_latency: Counter<u64>,
    pub get_latency: Counter<u64>,
    pub delete_latency: Counter<u64>,
    pub scan_latency: Counter<u64>,
    pub flush_latency: Counter<u64>,
    pub compaction_latency: Counter<u64>,
    pub cache_hits: Counter<u64>,
    pub cache_misses: Counter<u64>,
    pub bloom_negatives: Counter<u64>,
    pub errors: Counter<u64>,
}

impl OtelInstruments {
    /// Register OTel counters using the global meter.
    ///
    /// Returns `None` when OTel has not been initialised (i.e.
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` was not set at startup).
    pub fn try_register() -> Option<Arc<Self>> {
        Some(Arc::new(Self::from_meter(otel_meter()?)))
    }

    fn from_meter(meter: &Meter) -> Self {
        /// Helper: register a u64 counter instrument.
        fn init(meter: &Meter, name: &'static str, desc: &'static str) -> Counter<u64> {
            meter.u64_counter(name).with_description(desc).build()
        }

        Self {
            sets: init(meter, "apexstore.sets", "Total number of set operations"),
            gets: init(meter, "apexstore.gets", "Total number of get operations"),
            deletes: init(
                meter,
                "apexstore.deletes",
                "Total number of delete operations",
            ),
            scans: init(meter, "apexstore.scans", "Total number of scan operations"),
            batch_sets: init(
                meter,
                "apexstore.batch_sets",
                "Items in batch set operations",
            ),
            batch_deletes: init(
                meter,
                "apexstore.batch_deletes",
                "Items in batch delete operations",
            ),
            flushes: init(
                meter,
                "apexstore.flushes",
                "Total number of memtable flushes",
            ),
            compactions: init(
                meter,
                "apexstore.compactions",
                "Total number of compactions",
            ),
            set_latency: init(
                meter,
                "apexstore.set_latency_us",
                "Cumulative microseconds in set",
            ),
            get_latency: init(
                meter,
                "apexstore.get_latency_us",
                "Cumulative microseconds in get",
            ),
            delete_latency: init(
                meter,
                "apexstore.delete_latency_us",
                "Cumulative microseconds in delete",
            ),
            scan_latency: init(
                meter,
                "apexstore.scan_latency_us",
                "Cumulative microseconds in scan",
            ),
            flush_latency: init(
                meter,
                "apexstore.flush_latency_us",
                "Cumulative microseconds in flush",
            ),
            compaction_latency: init(
                meter,
                "apexstore.compaction_latency_us",
                "Cumulative microseconds in compaction",
            ),
            cache_hits: init(meter, "apexstore.cache_hits", "Total number of cache hits"),
            cache_misses: init(
                meter,
                "apexstore.cache_misses",
                "Total number of cache misses",
            ),
            bloom_negatives: init(
                meter,
                "apexstore.bloom_filter_negatives",
                "Bloom filter negatives",
            ),
            errors: init(meter, "apexstore.errors", "Total number of errors"),
        }
    }
}

#[cfg(test)]
mod tests;
