# Natuleadan® Pulsar™

> A self-sustaining, sovereign distributed AI agent harness written in Rust.

## What it is

Pulsar separates orchestration, sandboxed execution, provisioning and multi-platform interfaces into a one-way modular Cargo workspace, so each component evolves independently without leaking state across boundaries.

## What it provides

It provides shared wire contracts, an Ironic-style orchestrator, a sandbox supervisor with audited egress, and a zero-sudo installer. Contracts are the single source of truth consumed by every process.

## Architecture

Dependencies point one way: the protocol crate is foundational and the server, boxer, installer and client build on it, never on each other. Sessions are append-only and execution is fenced by monotonic tokens.

## Repository layout

The workspace groups crates for protocol, server, boxer, installer, client, styles and SDK, with deployment and validation helpers under `helpers/`. All source lives in the development branch until release.

## Getting started

Clone `https://github.com/natuleadan/pulsar.git`. This branch holds only the
project baseline; the source code, build manifests and full validation suite
live in the `dev` branch. Switch to `dev` to build and run the project.

## Development

Work happens on short-lived branches merged into `dev`. Commits follow
`type(scope): description` with a body of at least three bullets; commitlint
enforces the policy locally and in CI. The `main` branch is protected and holds
only this minimal baseline until the first release.

## Status

Active development. The current milestone delivers the backend, sandbox and infrastructure. User-facing client work and final certification are scheduled for upcoming batches; the first published version will be 0.1.0.

## License

Apache-2.0. See [LICENSE](LICENSE).

## Trademark

Natuleadan® Pulsar™ is a trademark of Natuleadan. Other constellation
repositories: Pulsar, Aurora, Pleaides, Andromeda and Orion.
