# AGENTS.md

Technical instructions for agents and contributors. The README describes the
product; this file explains how to build, verify and work in the repository.

## Development

Clone the repository and run `pnpm install` to install the Husky hooks and
`commitlint`. Always use `pnpm` (never `npm` or `npx`).

```sh
pnpm install
pnpm validate
```

## Validation

`pnpm validate` is the single gate before pushing. It enforces:

- `cargo fmt --all -- --check` — formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` — lint with zero warnings.
- `cargo test --workspace` — the full unit and integration suite.
- `node helpers/tools/check-scripts.mjs` — validates deployment scripts (`bash -n`, `shellcheck`, `shfmt`, 500-line limit).
- `node helpers/tools/release-plan.ts --check` — matches `package.json` and `CHANGELOG.md`.
- `node helpers/tools/third-party-notices.mjs --check` — matches `ThirdPartyNotices.txt`.

Repository hygiene: every source, script and document stays within 500 lines;
workspace crates are `publish = false`; no hardcoded paths or secrets. Checks
emit `PASS`, `FAIL` or `SKIP` and fail the build when any check fails. Never
skip a check because a binary is missing.

## Repository layout

| Path | Purpose |
|---|---|
| `crates/protocol` | Shared wire contracts, named codecs, JSON-RPC and HTTP DTOs |
| `crates/styles` | Design tokens and layout primitives (`publish = false`) |
| `crates/server` | Orchestrator daemon: Ironic module tree, PostgreSQL, Valkey, NATS, MCP and Autocron |
| `crates/boxer` | Sandbox supervisor: leases, isolation backends, egress proxy and previews |
| `crates/installer` | Provisioning CLI that runs audited deployment scripts |
| `crates/client` | Client core, reactive terminal UI and multi-platform shell |
| `helpers/deploy/scripts/` | POSIX/Bash provisioning and service bootstrap scripts |
| `helpers/tools/` | Script validation, release planning and notice generation |

## Architecture and module rules

- **Ironic modules (`crates/server`)**: organize each domain under
  `src/modules/<domain>/` with `mod.rs`, `controller/`, `services/`,
  `repositories/`, `dto/` and `entities/`. Controllers stay thin; business logic
  lives in services; database queries live in repositories.
- **One-way dependencies**: `protocol` has no workspace dependencies; `server`,
  `boxer`, `installer` and `client` depend on it, never circularly on each other.
- **Rust toolchain and security**: stable Rust 1.100+ with Cargo
  `min-publish-age` dependency cooldowns and OWASP Top 10:2025 / OWASP ASI 2026
  guardrails — Argon2id with JWT, SHA-256 hashed opaque tokens, fail-closed
  sandbox isolation and `DropGuard` resource cleanup.
- **English only** across source, identifiers, comments, documentation and
  commit messages. Comments are short and explain non-obvious code; longer
  explanation belongs in `docs/`.

## Branches

`main` is the protected release branch; merges trigger tagging and release
artifacts. `dev` is the protected integration branch where feature PRs land
before consolidated releases to `main`. Work on short-lived branches
(`feat/<scope>`, `fix/<scope>`, `docs/<scope>`, `ci/<scope>`) and reach `dev` (or
`main`) only through pull requests reviewed by the maintainer. Never rewrite
published history.

## Commits

Format: `type(scope): description`, lowercase, in English.

- Header: 3 to 7 words counting `type(scope):`, maximum 100 characters, no
  trailing period, blank line after it.
- Body: at least three bullets `- **Label**: body`, each body with 3 to 7 words.
- Allowed scopes: `client`, `server`, `boxer`, `installer`, `protocol`, `tools`,
  `docs`, `ci`, `workflow`, `hooks`, `release`, `license`, `deps`.

## Releases

Versions derive from conventional commits since the last `v*` tag:

- `fix`, `perf` and `revert` bump patch.
- `feat` bumps minor.
- `upgrade` or a breaking change bumps major.
- `docs`, `ci`, `chore`, `test`, `style` and `refactor` do not trigger a release.

Before opening or updating a functional PR toward `main`, run `pnpm
release:prepare` to update `package.json` and `CHANGELOG.md`, then verify with
`pnpm release:check`.
