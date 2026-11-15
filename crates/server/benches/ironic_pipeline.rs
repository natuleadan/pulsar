#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
//! Criterion microbenchmarks measuring Ironic module graph compilation, metrics recording, and cryptographic hashing.

use std::hint::black_box;

use agents_server::{
    app::InfraModule,
    infra::{DomainMetrics, apply_observability_plugins},
    modules::auth::services::crypto::hash_password,
};
use criterion::{Criterion, criterion_group, criterion_main};
use ironic::{Module, ModuleDefinition, metrics::MetricsRegistry};
use sha2::{Digest, Sha256};

fn bench_metrics(c: &mut Criterion) {
    let counter = MetricsRegistry.counter("bench_counter_total", "Benchmark test counter");
    let domain = DomainMetrics::global();

    c.bench_function("metrics/counter_increment", |b| {
        b.iter(|| counter.inc());
    });
    c.bench_function("metrics/registry_scrape", |b| {
        b.iter(|| black_box(ironic::metrics::scrape()));
    });
    c.bench_function("metrics/domain_histogram_record", |b| {
        b.iter(|| domain.request_latency_ms.record(black_box(0.015)));
    });
}

fn bench_ironic_pipeline(c: &mut Criterion) {
    c.bench_function("ironic/infra_module_definition", |b| {
        b.iter(|| black_box(InfraModule::definition()));
    });
    c.bench_function("ironic/apply_observability_plugins", |b| {
        b.iter(|| {
            let builder = ModuleDefinition::builder::<InfraModule>();
            let _ = black_box(apply_observability_plugins(builder));
        });
    });
}

fn bench_crypto(c: &mut Criterion) {
    c.bench_function("crypto/sha256_token", |b| {
        b.iter(|| {
            let mut hasher = Sha256::new();
            hasher.update(black_box(b"agents-token-sample-material"));
            black_box(hasher.finalize());
        });
    });

    let mut group = c.benchmark_group("crypto_slow");
    group.sample_size(10);
    group.bench_function("argon2id_hash_password", |b| {
        b.iter(|| black_box(hash_password(black_box("BenchSecret123!")).unwrap()));
    });
    group.finish();
}

criterion_group!(benches, bench_metrics, bench_ironic_pipeline, bench_crypto);
criterion_main!(benches);
