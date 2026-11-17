# Natuleadan® Pulsar™

> A self-sustaining, sovereign distributed AI agent harness written in Rust.

Pulsar separates orchestration, sandboxed execution, provisioning and multi-platform interfaces into a one-way modular Cargo workspace. Shared wire contracts keep every component independently evolvable.

## What it provides

- A shared protocol crate for HTTP, WebSocket, SSE and JSON-RPC contracts.
- An orchestrator daemon for sessions, tasks, workflows and integrations.
- A sandbox supervisor with isolation backends and audited egress.
- A zero-sudo installer for reproducible deployments.

## Layout

| Path | Purpose |
|---|---|
| `crates/protocol` | Shared wire contracts |
| `crates/server` | Orchestrator daemon |
| `crates/boxer` | Sandbox supervisor |
| `crates/installer` | Provisioning CLI |
| `crates/client` | Multi-platform client |
| `helpers/` | Deployment and validation tooling |

## Development

Requires Rust 1.100 or newer and pnpm. Run the same checks locally that CI
runs before opening a pull request.

```sh
pnpm validate
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Contributing](CONTRIBUTING.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
