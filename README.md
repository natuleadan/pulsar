# natuleadan-pulsar

Distributed, self-sustaining Rust agent harness combining an Ironic API orchestrator,
NATS JetStream swarm messaging, Valkey ephemeral state, PostgreSQL persistence,
modular Boxer sandbox execution, Traefik ephemeral previews, and multi-platform NSH clients.

## Crates

- **`pulsar-protocol`**: Shared protocol contracts, named codecs, JSON-RPC, and event streaming DTOs.
- **`pulsar-styles`**: Reusable cross-platform design tokens, component specs, and NSH-inspired UI primitives.
- **`pulsar-server`**: Central orchestrator daemon with Ironic NestJS-like modules, PostgreSQL, Valkey, NATS, MCP host, and autocron engine.
- **`pulsar-boxer`**: Sandbox daemon supervising `Runtime x Isolation` backends (`host`, `docker`, `openshell`), Traefik preview routes, and the native 2-level agent loop.
- **`pulsar-installer`**: VPS provisioning CLI coordinating external `.sh` hardening and cluster setup scripts.
- **`pulsar-client`**: Shared `client-core`, `client-tui`, and multi-platform `nsh` GUI (`web`, `ios`, `android`, `macos`, `windows`, `linux`) with `Server` and `Third-Party` modes.

## Development

```bash
# Install commitlint, Husky hooks, and workspace tooling
pnpm install

# Regenerate ThirdPartyNotices.txt from direct dependencies
pnpm notices

# Run full repository, formatting, clippy, test, notices, and release-plan checks
pnpm validate
```

## Documentation & Governance

- [AGENTS.md](AGENTS.md) — technical architecture rules, branch model, and validation gates.
- [CONTRIBUTING.md](CONTRIBUTING.md) — commit policy and semantic release workflow.
- [SECURITY.md](SECURITY.md) — vulnerability disclosure and security scope.
- [NOTICE](NOTICE) — Apache-2.0 attribution notice.
- [ThirdPartyNotices.txt](ThirdPartyNotices.txt) — generated full licenses of direct workspace dependencies.