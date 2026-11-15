//! Structured logging, TimeSeriesLayer JSONL rotation, OTLP initialization, and log querying.

use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::Arc,
};

use ironic::{
    OpenApiSchema,
    logging::{FileLogStorage, TimeSeriesConfig},
};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};
use serde::{Deserialize, Serialize};
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use super::apm::ApmConfig;
use crate::infra::adapters::BlobStorePort;

/// Configuration for structured file-backed time-series logging.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Directory where daily `.jsonl` log files are stored.
    pub log_dir: PathBuf,
    /// Minimum log level filter (`trace`, `debug`, `info`, `warn`, `error`).
    pub level: String,
    /// Whether structured events should also emit to stdout.
    pub stdout_enabled: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        let log_dir = std::env::var("AGENTS_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                if Path::new("/var/log/agents").is_dir() {
                    PathBuf::from("/var/log/agents")
                } else {
                    PathBuf::from(".logs")
                }
            });

        let level =
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,agents_server=debug".to_owned());

        Self {
            log_dir,
            level,
            stdout_enabled: true,
        }
    }
}

impl LoggingConfig {
    /// Creates a custom logging configuration pointing to `dir`.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: dir.into(),
            level: "info".to_owned(),
            stdout_enabled: true,
        }
    }

    /// Resolves the expected path for today's log file (`YYYY-MM-DD.jsonl`).
    #[must_use]
    pub fn today_log_file(&self) -> PathBuf {
        let now = chrono::Utc::now();
        let filename = format!("{}.jsonl", now.format("%Y-%m-%d"));
        self.log_dir.join(filename)
    }

    /// Ensures the log destination directory exists and is writable, falling back to `.logs` if needed.
    #[must_use]
    pub fn resolve_writable_dir(&self) -> PathBuf {
        if ensure_writable(&self.log_dir) {
            return self.log_dir.clone();
        }
        let fallback = PathBuf::from(".logs");
        let _ = ensure_writable(&fallback);
        fallback
    }

    /// Ensures the log destination directory exists.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if creating the target directory fails.
    pub fn ensure_dir(&self) -> Result<(), std::io::Error> {
        if !self.log_dir.exists() {
            std::fs::create_dir_all(&self.log_dir)?;
        }
        Ok(())
    }
}

fn ensure_writable(dir: &Path) -> bool {
    if !dir.exists() && std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write_probe");
    if OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&probe)
        .is_ok()
    {
        let _ = std::fs::remove_file(&probe);
        true
    } else {
        false
    }
}

fn build_env_filter(base_level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "{base_level},sqlx=warn,hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,async_nats=warn,rustls=warn"
        ))
    })
}

/// Guard that flushes and shuts down the OTLP tracer provider on process exit.
pub struct OtelGuard {
    provider: SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(err) = self.provider.shutdown() {
            tracing::warn!(error = %err, "Failed to flush OTLP tracer provider on shutdown");
        }
    }
}

fn build_otlp_provider(apm: &ApmConfig, endpoint: &str) -> Option<SdkTracerProvider> {
    let sampler = if (apm.sample_rate - 1.0).abs() < f64::EPSILON {
        Sampler::AlwaysOn
    } else if apm.sample_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(apm.sample_rate)
    };
    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(std::time::Duration::from_millis(apm.batch_interval_ms))
        .build()
    {
        Ok(exp) => exp,
        Err(err) => {
            eprintln!("[WARN] Failed to initialize OTLP span exporter ({endpoint}): {err}");
            return None;
        }
    };
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(sampler)
        .with_resource(
            Resource::builder_empty()
                .with_service_name(apm.service_name.clone())
                .build(),
        )
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    Some(provider)
}

/// Initializes structured logging (`fmt` + Ironic `TimeSeriesLayer` -> `YYYY-MM-DD.jsonl`)
/// and real OTLP distributed tracing when `OTEL_EXPORTER_OTLP_ENDPOINT` / `AGENTS_OTEL_ENDPOINT` / `OTLP_ENDPOINT` is set.
#[must_use]
pub fn init_structured_logging(logging: &LoggingConfig, apm: &ApmConfig) -> Option<OtelGuard> {
    let writable_dir = logging.resolve_writable_dir();
    let storage = Arc::new(FileLogStorage::new(&writable_dir));
    let ts_layer = TimeSeriesConfig::default().with_storage(storage).layer();
    let ts_filter = build_env_filter(&logging.level);
    let fmt_filter = build_env_filter(&logging.level);

    let otlp_provider = apm
        .otlp_endpoint
        .as_deref()
        .and_then(|endpoint| build_otlp_provider(apm, endpoint));
    let otel_layer = otlp_provider.as_ref().map(|provider| {
        let tracer = provider.tracer("agents-server");
        tracing_opentelemetry::layer().with_tracer(tracer)
    });

    let _ = tracing_subscriber::registry()
        .with(ts_layer.with_filter(ts_filter))
        .with(tracing_subscriber::fmt::layer().with_filter(fmt_filter))
        .with(otel_layer)
        .try_init();

    if let Some(endpoint) = &apm.otlp_endpoint
        && otlp_provider.is_some()
    {
        tracing::info!(
            service = %apm.service_name,
            otlp_endpoint = %endpoint,
            sample_rate = apm.sample_rate,
            "OpenTelemetry OTLP exporter active alongside JSONL TimeSeriesLayer"
        );
    }

    otlp_provider.map(|provider| OtelGuard { provider })
}

/// Query parameters for filtering structured JSONL logs.
#[derive(Clone, Debug, Default, Deserialize, Serialize, garde::Validate)]
pub struct SystemLogQueryDto {
    /// Filter by log level (`ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`).
    #[garde(length(max = 16))]
    pub level: Option<String>,
    /// Substring match on the tracing `target` module path.
    #[garde(length(max = 128))]
    pub target: Option<String>,
    /// Free-text substring match across message, trace_id, or fields.
    #[garde(length(max = 128))]
    pub q: Option<String>,
    /// Maximum number of log entries to return (default 100, capped at 500).
    #[garde(range(min = 1, max = 500))]
    pub limit: Option<usize>,
}

/// Response payload returned by the structured system log query endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemLogsResponse {
    /// Matching log entries ordered newest-first.
    pub logs: Vec<serde_json::Value>,
    /// Number of returned entries.
    pub total: usize,
    /// Active log directory scanned.
    pub log_dir: String,
}

impl OpenApiSchema for SystemLogsResponse {
    fn openapi_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["logs", "total", "log_dir"],
            "properties": {
                "logs": { "type": "array", "items": { "type": "object" } },
                "total": { "type": "integer", "example": 25 },
                "log_dir": { "type": "string", "example": "/var/log/agents" }
            }
        })
    }
}

/// Queries daily `.jsonl` structured logs from `log_dir`, returning newest matching entries first.
#[must_use]
pub fn query_jsonl_logs(log_dir: &Path, query: &SystemLogQueryDto) -> SystemLogsResponse {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let level_filter = query.level.as_deref().map(str::to_ascii_uppercase);
    let target_filter = query.target.as_deref().map(str::to_ascii_lowercase);
    let q_filter = query.q.as_deref().map(str::to_ascii_lowercase);

    let mut files: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    files.reverse();

    let mut matches = Vec::new();
    for path in files {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        for line in lines.into_iter().rev() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if let Some(ref lvl) = level_filter {
                let entry_lvl = val["level"].as_str().unwrap_or("").to_ascii_uppercase();
                if &entry_lvl != lvl {
                    continue;
                }
            }
            if let Some(ref tgt) = target_filter {
                let entry_tgt = val["target"].as_str().unwrap_or("").to_ascii_lowercase();
                if !entry_tgt.contains(tgt) {
                    continue;
                }
            }
            if let Some(ref q) = q_filter
                && !line.to_ascii_lowercase().contains(q)
            {
                continue;
            }
            matches.push(val);
            if matches.len() >= limit {
                break;
            }
        }
        if matches.len() >= limit {
            break;
        }
    }

    let total = matches.len();
    SystemLogsResponse {
        logs: matches,
        total,
        log_dir: log_dir.display().to_string(),
    }
}

/// Archives `.jsonl` log files from `log_dir` into [`BlobStorePort`] (S3 / Upstash / LocalFS).
pub async fn archive_logs_to_blob(log_dir: &Path, blob: &dyn BlobStorePort) -> usize {
    let mut archived = 0;
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let key = format!("observability/logs/{name}");
        if blob
            .put_blob(&key, bytes, "application/x-ndjson")
            .await
            .is_ok()
        {
            archived += 1;
        }
    }
    archived
}
