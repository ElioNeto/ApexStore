use super::*;
use crate::infra::metrics::EngineMetrics;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_proto::tonic::collector::metrics::v1::{
    metrics_service_server::{MetricsService, MetricsServiceServer},
    ExportMetricsServiceRequest, ExportMetricsServiceResponse,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_server::{TraceService, TraceServiceServer},
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::metrics::v1::{metric::Data, number_data_point};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot};
use tonic::{Request, Response, Status};

/// A local OTLP gRPC receiver. The production tonic exporters serialize and send
/// real protobuf requests to this listener; no SDK/exporter is substituted.
#[derive(Clone)]
struct Receiver {
    traces: mpsc::UnboundedSender<ExportTraceServiceRequest>,
    metrics: mpsc::UnboundedSender<ExportMetricsServiceRequest>,
    reject: Arc<AtomicBool>,
}

#[tonic::async_trait]
impl TraceService for Receiver {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        self.traces.send(request.into_inner()).unwrap();
        if self.reject.load(Ordering::SeqCst) {
            return Err(Status::invalid_argument("collector rejected traces"));
        }
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

#[tonic::async_trait]
impl MetricsService for Receiver {
    async fn export(
        &self,
        request: Request<ExportMetricsServiceRequest>,
    ) -> Result<Response<ExportMetricsServiceResponse>, Status> {
        self.metrics.send(request.into_inner()).unwrap();
        if self.reject.load(Ordering::SeqCst) {
            return Err(Status::invalid_argument("collector rejected metrics"));
        }
        Ok(Response::new(ExportMetricsServiceResponse::default()))
    }
}

struct Collector {
    endpoint: String,
    traces: mpsc::UnboundedReceiver<ExportTraceServiceRequest>,
    metrics: mpsc::UnboundedReceiver<ExportMetricsServiceRequest>,
    reject: Arc<AtomicBool>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl Collector {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (trace_tx, traces) = mpsc::unbounded_channel();
        let (metric_tx, metrics) = mpsc::unbounded_channel();
        let reject = Arc::new(AtomicBool::new(false));
        let receiver = Receiver {
            traces: trace_tx,
            metrics: metric_tx,
            reject: reject.clone(),
        };
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(TraceServiceServer::new(receiver.clone()))
                .add_service(MetricsServiceServer::new(receiver))
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async {
                        let _ = stopped.await;
                    },
                ),
        );
        Self {
            endpoint,
            traces,
            metrics,
            reject,
            stop,
            task,
        }
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    async fn trace(&mut self) -> ExportTraceServiceRequest {
        tokio::time::timeout(Duration::from_secs(10), self.traces.recv())
            .await
            .unwrap()
            .unwrap()
    }

    async fn metric(&mut self) -> ExportMetricsServiceRequest {
        tokio::time::timeout(Duration::from_secs(10), self.metrics.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

fn record_span(provider: &sdk_trace::SdkTracerProvider, name: &'static str) {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("apexstore")));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("storage_operation", operation = name);
        let _entered = span.enter();
        tracing::info!("operation completed");
    });
}

fn assert_resource(resource: &opentelemetry_proto::tonic::resource::v1::Resource) {
    for (key, expected) in [
        ("service.name", "apexstore"),
        ("service.version", env!("CARGO_PKG_VERSION")),
    ] {
        assert!(
            resource.attributes.iter().any(|attribute| {
                attribute.key == key
                    && attribute.value.as_ref().and_then(|v| v.value.as_ref())
                        == Some(&Value::StringValue(expected.to_string()))
            }),
            "missing resource {key}"
        );
    }
}

fn assert_trace(request: &ExportTraceServiceRequest, operation: &str) {
    let resource = &request.resource_spans[0];
    assert_resource(resource.resource.as_ref().unwrap());
    let scope = &resource.scope_spans[0];
    assert_eq!(scope.scope.as_ref().unwrap().name, "apexstore");
    assert!(
        scope.spans.iter().any(|span| {
            span.name == "storage_operation"
                && span.trace_id.len() == 16
                && span.span_id.len() == 8
                && span.attributes.iter().any(|attribute| {
                    attribute.key == "operation"
                        && attribute.value.as_ref().and_then(|v| v.value.as_ref())
                            == Some(&Value::StringValue(operation.to_string()))
                })
                && span
                    .events
                    .iter()
                    .any(|event| event.name == "operation completed")
        }),
        "missing span: {request:?}"
    );
}

fn assert_counter(request: &ExportMetricsServiceRequest, name: &str, value: i64) {
    let resource = &request.resource_metrics[0];
    assert_resource(resource.resource.as_ref().unwrap());
    let scope = &resource.scope_metrics[0];
    assert_eq!(scope.scope.as_ref().unwrap().name, "apexstore");
    let metric = scope
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap();
    let Some(Data::Sum(sum)) = &metric.data else {
        panic!("expected counter sum")
    };
    assert!(sum.is_monotonic);
    assert_eq!(
        sum.data_points[0].value,
        Some(number_data_point::Value::AsInt(value))
    );
}

// Actix uses the current-thread flavor. This also catches blocking shutdown
// regressions that a multithreaded runtime could conceal.
#[tokio::test(flavor = "current_thread")]
async fn otlp_flush_and_shutdown_deliver_traces_and_engine_metrics() {
    let mut collector = Collector::start().await;
    let traces = build_tracer_provider(&collector.endpoint).unwrap();
    let metrics = build_meter_provider(&collector.endpoint).unwrap();
    let mut engine_metrics = EngineMetrics::new();
    engine_metrics.set_otel_instruments(Some(Arc::new(OtelInstruments::from_meter(
        &metrics.meter("apexstore"),
    ))));
    engine_metrics.record_set(42);
    engine_metrics.record_delete(11);
    engine_metrics.record_scan(12);
    engine_metrics.record_batch_sets(3);
    engine_metrics.record_batch_deletes(4);
    engine_metrics.record_flush(13);
    engine_metrics.record_compaction(14);
    engine_metrics.record_cache_hit();
    engine_metrics.record_cache_miss();
    engine_metrics.record_bloom_negative();
    engine_metrics.record_error();
    record_span(&traces, "before_flush");
    let trace_flush = traces.clone();
    let metric_flush = metrics.clone();
    tokio::task::spawn_blocking(move || {
        trace_flush.force_flush().unwrap();
        metric_flush.force_flush().unwrap();
    })
    .await
    .unwrap();
    assert_trace(&collector.trace().await, "before_flush");
    let exported = collector.metric().await;
    assert_counter(&exported, "apexstore.sets", 1);
    assert_counter(&exported, "apexstore.set_latency_us", 42);
    for (name, value) in [
        ("apexstore.deletes", 1),
        ("apexstore.delete_latency_us", 11),
        ("apexstore.scans", 1),
        ("apexstore.scan_latency_us", 12),
        ("apexstore.batch_sets", 3),
        ("apexstore.batch_deletes", 4),
        ("apexstore.flushes", 1),
        ("apexstore.flush_latency_us", 13),
        ("apexstore.compactions", 1),
        ("apexstore.compaction_latency_us", 14),
        ("apexstore.cache_hits", 1),
        ("apexstore.cache_misses", 1),
        ("apexstore.bloom_filter_negatives", 1),
        ("apexstore.errors", 1),
    ] {
        assert_counter(&exported, name, value);
    }

    // No explicit flush here: shutdown itself must deliver the pending data.
    engine_metrics.record_get(17);
    record_span(&traces, "before_shutdown");
    shutdown(Some(traces.clone()), Some(metrics.clone()))
        .await
        .unwrap();
    assert_trace(&collector.trace().await, "before_shutdown");
    let exported = collector.metric().await;
    assert_counter(&exported, "apexstore.gets", 1);
    assert_counter(&exported, "apexstore.get_latency_us", 17);

    let error = shutdown(Some(traces), Some(metrics)).await.unwrap_err();
    assert!(matches!(
        error,
        TelemetryError::Shutdown {
            traces: Some(_),
            metrics: Some(_)
        }
    ));
    collector.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn collector_rejection_is_reported_and_both_pipelines_are_shutdown() {
    let mut collector = Collector::start().await;
    collector.reject.store(true, Ordering::SeqCst);
    let traces = build_tracer_provider(&collector.endpoint).unwrap();
    let metrics = build_meter_provider(&collector.endpoint).unwrap();
    let counter = metrics.meter("apexstore").u64_counter("rejected").build();
    counter.add(1, &[]);
    record_span(&traces, "rejected_flush");
    let trace_flush = traces.clone();
    let metric_flush = metrics.clone();
    let (trace_error, metric_error) = tokio::task::spawn_blocking(move || {
        (
            trace_flush.force_flush().unwrap_err(),
            metric_flush.force_flush().unwrap_err(),
        )
    })
    .await
    .unwrap();
    assert!(
        trace_error.to_string().contains("InvalidArgument"),
        "{trace_error}"
    );
    assert!(!metric_error.to_string().is_empty());
    collector.trace().await;
    collector.metric().await;

    // A failed export must not permanently disable either pipeline.
    collector.reject.store(false, Ordering::SeqCst);
    record_span(&traces, "recovered");
    counter.add(1, &[]);
    let trace_flush = traces.clone();
    let metric_flush = metrics.clone();
    tokio::task::spawn_blocking(move || {
        trace_flush.force_flush().unwrap();
        metric_flush.force_flush().unwrap();
    })
    .await
    .unwrap();
    assert_trace(&collector.trace().await, "recovered");
    assert_counter(&collector.metric().await, "rejected", 2);

    collector.reject.store(true, Ordering::SeqCst);
    record_span(&traces, "rejected_shutdown");
    counter.add(1, &[]);
    let error = shutdown(Some(traces.clone()), Some(metrics.clone()))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        TelemetryError::Shutdown {
            traces: Some(_),
            metrics: Some(_)
        }
    ));
    tokio::task::spawn_blocking(move || {
        assert!(traces.force_flush().is_err());
        assert!(metrics.force_flush().is_err());
    })
    .await
    .unwrap();
    assert_trace(&collector.trace().await, "rejected_shutdown");
    collector.metric().await;
    collector.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_endpoint_returns_build_errors_without_panicking() {
    assert!(matches!(
        build_tracer_provider("http://[invalid"),
        Err(TelemetryError::TraceExporter(_))
    ));
    assert!(matches!(
        build_meter_provider("http://[invalid"),
        Err(TelemetryError::MetricExporter(_))
    ));
    shutdown(None, None).await.unwrap();
}

#[test]
fn console_fallback_without_endpoint() {
    const CHILD: &str = "APEXSTORE_TEST_CONSOLE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        assert!(init_tracing().unwrap().is_none());
        assert!(init_metrics().unwrap().is_none());
        assert!(OtelInstruments::try_register().is_none());
        tracing::info!("telemetry console fallback works");
        return;
    }
    // Global subscribers and OnceLocks are process-scoped. Run the real public
    // initializers in fresh processes for both disabled endpoint forms.
    for endpoint in [None, Some("")] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infra::telemetry::tests::console_fallback_without_endpoint",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env_remove("RUST_LOG")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT");
        if let Some(endpoint) = endpoint {
            command.env("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("telemetry console fallback works")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn public_initializers_export_real_engine_operations() {
    const CHILD: &str = "APEXSTORE_TEST_ENGINE_TELEMETRY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let traces = init_tracing().unwrap();
        let metrics = init_metrics().unwrap();
        assert!(traces.is_some());
        assert!(metrics.is_some());
        let directory = tempfile::tempdir().unwrap();
        let config = crate::LsmConfig::builder()
            .dir_path(directory.path().to_path_buf())
            .encryption_enabled(false)
            .build()
            .unwrap();
        let engine = crate::LsmEngine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(64, 4096),
        )
        .unwrap();
        // Use the installed global subscriber and the engine's own registration.
        // No custom subscriber, meter, or instrument injection in this process.
        {
            let span = tracing::info_span!("storage_operation", operation = "real_engine");
            let _entered = span.enter();
            engine
                .set(b"telemetry-key".to_vec(), b"telemetry-value".to_vec())
                .unwrap();
            assert_eq!(
                engine.get(b"telemetry-key").unwrap(),
                Some(b"telemetry-value".to_vec())
            );
            engine.flush_memtable().unwrap();
            tracing::info!("operation completed");
        }
        engine.close();
        drop(engine);
        shutdown(traces, metrics).await.unwrap();
        return;
    }

    let mut collector = Collector::start().await;
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "infra::telemetry::tests::public_initializers_export_real_engine_operations",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", &collector.endpoint)
        .env("RUST_LOG", "info")
        .kill_on_drop(true);
    // Await without blocking: the parent runtime must serve the real gRPC calls.
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_trace(&collector.trace().await, "real_engine");
    let exported = collector.metric().await;
    assert_counter(&exported, "apexstore.sets", 1);
    assert_counter(&exported, "apexstore.gets", 1);
    assert_counter(&exported, "apexstore.flushes", 1);
    collector.stop().await;
}
