# AGENTS.md — Pulsar

Technical orientation for agents and contributors. The README describes the
product; this file explains how to build, verify and work in the repository.

## Overview

Pulsar is a self-sustaining distributed Rust agent harness: modular Ironic-style
orchestration in `server`, sandboxed execution in `boxer`, shared wire contracts
in `protocol`, zero-sudo provisioning in `installer`, and multi-platform clients.

## Repository layout

| Path | Purpose |
|---|---|
| `crates/protocol` | Shared wire contracts, named codecs, JSON-RPC and HTTP DTOs |
| `crates/server` | Orchestrator daemon: modules, PostgreSQL, Valkey, NATS, MCP and Autocron |
| `crates/boxer` | Sandbox supervisor: leases, isolation backends, egress proxy and previews |
| `crates/installer` | Provisioning CLI that runs audited deployment scripts |
| `crates/client` | Client core, reactive terminal UI and multi-platform shell |
| `crates/styles` | Design tokens and layout primitives |
| `helpers/deploy/scripts/` | POSIX/Bash provisioning and service bootstrap scripts |
| `helpers/tools/` | Script validation, release planning and notice generation |

Dependencies point one way: `protocol` has no workspace dependencies; `server`,
`boxer`, `installer` and `client` depend on it, never on each other.

## Development

Use `pnpm` (never `npm` or `npx`). Install hooks with `pnpm install`, then run:

```sh
pnpm validate
```

This is the single gate before pushing. It runs formatting, clippy with
`-D warnings`, the full workspace test suite, script validation and repository
hygiene checks. Checks emit `PASS`, `FAIL` or `SKIP` and fail on any error.

## Conventions

- English across source, docs, file names, output and commits.
- Comments are written in English and only where they explain non-obvious code;
  longer explanation belongs in `docs/`.
- Source files stay within 500 lines.
- Never print secrets; `.env.example` holds placeholders only.
- Use short-lived branches (`feat/...`, `fix/...`, `docs/...`), open a pull
  request against `dev` and consolidate into `main` after review. Never rewrite
  published history.
- Commit format: `type(scope): description`, scope in English, 3–7 words, a body
  with at least three bullets `- **Title**: body` and one newline between them.
  Author `leojara95`; never add a `Co-authored-by` trailer.
