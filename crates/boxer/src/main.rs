//! Boxer worker daemon CLI entrypoint.

#![forbid(unsafe_code)]

use std::{sync::Arc, time::Duration};

use clap::Parser;
use ironic::logging::{FileLogStorage, TimeSeriesConfig};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use pulsar_boxer::{
    config::{BoxerCli, BoxerConfig},
    control_plane::{
        BoxerControlPlaneService, build_control_plane_router, start_autodiscovery_heartbeat,
    },
    daemon::{BoxerDaemon, NatsBoxerBus},
    leases::HttpServerLeaseClient,
};
use tracing::info;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

struct BoxerOtelGuard(SdkTracerProvider);

impl Drop for BoxerOtelGuard {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

fn build_boxer_otlp(endpoint: &str) -> Option<SdkTracerProvider> {
    let service_name = std::env::var("OTEL_SERVICE_NAME")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pulsar-boxer".to_owned());
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .ok()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder_empty()
                .with_service_name(service_name)
                .build(),
        )
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    Some(provider)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();
    let boxer_log_dir = std::env::var("PULSAR_BOXER_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if let Ok(base) = std::env::var("PULSAR_LOG_DIR") {
                std::path::PathBuf::from(base).join("boxer")
            } else if std::path::Path::new("/var/log/pulsar").is_dir() {
                std::path::PathBuf::from("/var/log/pulsar/boxer")
            } else {
                std::path::PathBuf::from(".logs/boxer")
            }
        });
    let _ = std::fs::create_dir_all(&boxer_log_dir);
    let ts_layer = TimeSeriesConfig::default()
        .with_storage(Arc::new(FileLogStorage::new(&boxer_log_dir)))
        .layer();
    let ts_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,async_nats=warn")
    });
    let fmt_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,async_nats=warn")
    });
    let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .or_else(|_| std::env::var("PULSAR_OTEL_ENDPOINT"))
        .or_else(|_| std::env::var("OTLP_ENDPOINT"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let otlp_provider = otel_endpoint.as_deref().and_then(build_boxer_otlp);
    let otel_layer = otlp_provider.as_ref().map(|provider| {
        let tracer = provider.tracer("pulsar-boxer");
        tracing_opentelemetry::layer().with_tracer(tracer)
    });

    let _ = tracing_subscriber::registry()
        .with(ts_layer.with_filter(ts_filter))
        .with(tracing_subscriber::fmt::layer().with_filter(fmt_filter))
        .with(otel_layer)
        .try_init();

    let _otel_guard = otlp_provider.map(BoxerOtelGuard);
    if let Some(otel) = otel_endpoint
        && _otel_guard.is_some()
    {
        info!(otlp_endpoint = %otel, "pulsar-boxer OTLP exporter active alongside JSONL TimeSeriesLayer");
    }

    let cli = BoxerCli::parse();
    let cfg = Arc::new(BoxerConfig::from_cli(cli)?);
    let lease_client = Arc::new(HttpServerLeaseClient::new(
        cfg.server_internal_url.clone(),
        cfg.worker_token.clone(),
    ));
    let nats_bus = NatsBoxerBus::connect(&cfg).await?;
    let daemon = Arc::new(BoxerDaemon::new(
        cfg.as_ref().clone(),
        lease_client,
        nats_bus.clone(),
    )?);

    let cp_service = Arc::new(BoxerControlPlaneService::new(Arc::clone(&cfg)));
    let app = build_control_plane_router(Arc::clone(&cp_service), &cfg);

    let bind_addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    info!("Starting Boxer HTTP control plane on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    let _heartbeat_task = start_autodiscovery_heartbeat(
        Arc::clone(&cfg),
        Arc::clone(&cp_service),
        Duration::from_secs(30),
    );

    tokio::select! {
        res = daemon.run_nats_until_shutdown(&nats_bus) => {
            res?;
        }
        res = axum::serve(listener, app) => {
            res?;
        }
    }
    Ok(())
}
