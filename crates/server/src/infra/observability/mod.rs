//! Observability infrastructure: metrics, distributed tracing, APM, logging, lifecycle, health, timeouts, plugins, and pagination.

pub mod apm;
pub mod health;
pub mod lifecycle;
pub mod logging;
pub mod metrics;
pub mod pagination;
pub mod plugin;
pub mod timeout;
pub mod tracing;

pub use apm::ApmConfig;
pub use health::ProviderHealthIndicator;
pub use lifecycle::{LifecycleManager, LifecycleSnapshot};
pub use logging::{
    LoggingConfig, OtelGuard, SystemLogQueryDto, SystemLogsResponse, archive_logs_to_blob,
    init_structured_logging, query_jsonl_logs,
};
pub use metrics::DomainMetrics;
pub use pagination::slice_paginated;
pub use plugin::{ObservabilityPlugin, apply_observability_plugins};
pub use timeout::PerRouteTimeoutInterceptor;
pub use tracing::{TraceContextMiddleware, W3cTraceContext, inject_w3c_traceparent};

/// Builds the global Ironic `RateLimitMiddleware` keyed by `x-agent-id`, `x-api-key`, or client IP.
#[must_use]
pub fn build_rate_limiter(
    max_requests: u64,
    window_secs: u64,
) -> ironic::security::RateLimitMiddleware {
    ironic::security::RateLimitMiddleware::new(max_requests, window_secs).with_key_resolver(
        std::sync::Arc::new(|ctx| {
            let req = ctx.request();
            if let Some(agent_id) = req
                .headers()
                .get("x-agent-id")
                .or_else(|| req.headers().get("x-box-id"))
                .and_then(|h| h.to_str().ok())
                .filter(|s| !s.is_empty())
            {
                return format!("agent:{agent_id}");
            }
            if let Some(api_key) = req
                .headers()
                .get("x-api-key")
                .and_then(|h| h.to_str().ok())
                .filter(|s| !s.is_empty())
            {
                return format!("key:{api_key}");
            }
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|all| all.split(',').next_back().map(str::trim))
                .filter(|ip| !ip.is_empty())
                .unwrap_or("127.0.0.1")
                .to_owned()
        }),
    )
}
