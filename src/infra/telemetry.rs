use opentelemetry::global;
use opentelemetry::metrics::{Counter, Meter};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
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
pub fn init_tracing() {
    if let Some(endpoint) = otlp_endpoint() {
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(&endpoint)
                    .with_timeout(Duration::from_secs(5)),
            )
            .with_trace_config(
                sdk_trace::config()
                    .with_resource(Resource::new(vec![
                        KeyValue::new("service.name", "apexstore"),
                        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                    ]))
                    .with_sampler(sdk_trace::Sampler::AlwaysOn),
            )
            .install_batch(opentelemetry_sdk::runtime::Tokio)
            .expect("Failed to install OTLP trace exporter");

        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"));

        tracing_subscriber::registry()
            .with(filter)
            .with(telemetry_layer)
            .init();
    } else {
        // Fallback: standard console logging
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_target(false)
            .with_level(true)
            .init();
    }
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
/// configured).
pub fn init_metrics() {
    let endpoint = match otlp_endpoint() {
        Some(ep) => ep,
        None => return, // no-op: OTel not configured
    };

    let resource = Resource::new(vec![
        KeyValue::new("service.name", "apexstore"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ]);

    // Build the OTLP metric exporter using the tonic (gRPC) protocol.
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&endpoint)
        .with_timeout(Duration::from_secs(5));

    let provider = opentelemetry_otlp::new_pipeline()
        .metrics(opentelemetry_sdk::runtime::Tokio)
        .with_exporter(exporter)
        .with_resource(resource)
        .with_period(Duration::from_secs(60))
        .with_timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build OTLP metrics pipeline");

    // Register as the global meter provider so that `global::meter()` works.
    global::set_meter_provider(provider.clone());

    let meter = global::meter("apexstore");
    let _ = OTEL_METER.set(meter);
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
        let meter = otel_meter()?;

        /// Helper: register a u64 counter instrument.
        fn init(meter: &Meter, name: &'static str, desc: &'static str) -> Counter<u64> {
            meter.u64_counter(name).with_description(desc).init()
        }

        Some(Arc::new(Self {
            sets: init(meter, "apexstore.sets", "Total number of set operations"),
            gets: init(meter, "apexstore.gets", "Total number of get operations"),
            deletes: init(meter, "apexstore.deletes", "Total number of delete operations"),
            scans: init(meter, "apexstore.scans", "Total number of scan operations"),
            batch_sets: init(meter, "apexstore.batch_sets", "Items in batch set operations"),
            batch_deletes: init(meter, "apexstore.batch_deletes", "Items in batch delete operations"),
            flushes: init(meter, "apexstore.flushes", "Total number of memtable flushes"),
            compactions: init(meter, "apexstore.compactions", "Total number of compactions"),
            set_latency: init(meter, "apexstore.set_latency_us", "Cumulative microseconds in set"),
            get_latency: init(meter, "apexstore.get_latency_us", "Cumulative microseconds in get"),
            delete_latency: init(meter, "apexstore.delete_latency_us", "Cumulative microseconds in delete"),
            scan_latency: init(meter, "apexstore.scan_latency_us", "Cumulative microseconds in scan"),
            flush_latency: init(meter, "apexstore.flush_latency_us", "Cumulative microseconds in flush"),
            compaction_latency: init(
                meter,
                "apexstore.compaction_latency_us",
                "Cumulative microseconds in compaction",
            ),
            cache_hits: init(meter, "apexstore.cache_hits", "Total number of cache hits"),
            cache_misses: init(meter, "apexstore.cache_misses", "Total number of cache misses"),
            bloom_negatives: init(
                meter,
                "apexstore.bloom_filter_negatives",
                "Bloom filter negatives",
            ),
            errors: init(meter, "apexstore.errors", "Total number of errors"),
        }))
    }
}
