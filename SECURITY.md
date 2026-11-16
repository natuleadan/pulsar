# Security Policy

## Supported versions

The latest release on `main` receives security fixes. Older versions are not
supported.

## Reporting a vulnerability

Do not open a public issue. Use the private advisory form:

https://github.com/natuleadan/agents/security/advisories/new

Include the affected crate or component, a minimal reproduction and the impact.
If the report involves a dependency or sandbox boundary, include the crate name,
version and target environment.

## Response

We acknowledge the report, triage it and coordinate disclosure after a fix is
available.

## Scope

This repository ships the distributed agent harness workspace (`agents-protocol`,
`agents-server`, `agents-boxer`, `agents-installer`, `agents-client`) and
deployment hardening scripts. Never commit live tokens, keys or `.env` files.

## OWASP Top 10:2025 Threat Coverage

| Vulnerability | Target Component | Defense & Implementation Mechanism |
|---|---|---|
| **A01: Broken Object Level Authorization (BOLA)** | `agents-server` (`sessions`, `tasks`, `workgroups`) | Strict ownership validation in services (`verify_session_owner`, `verify_workgroup_owner`), tenant ID hashing, and 403 Forbidden fail-closed enforcement across all session and task routes. |
| **A02: Cryptographic Failures** | `agents-server`, `agents-protocol` | Argon2id password hashing with random salt, constant-time verification (`constant_time_eq_hex`), AES-256-GCM / HMAC-SHA256 authenticated envelopes, and zeroized secrets in memory. |
| **A03: Injection** | `agents-server`, `agents-boxer` | Parameterized SQL queries via SQLx, strict JSON deserialization with schema validation (`garde`), and escaped shell arguments in sandboxed execution paths. |
| **A04: Insecure Design** | Whole Architecture | Monotonic tool decision resolution (`ToolDecisionContext`), plan-and-act execution mode, deterministic human-in-the-loop approval gates, and multi-tenant quota fencing. |
| **A05: Security Misconfiguration** | `agents-server`, Traefik | Mandatory security headers (`X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Strict-Transport-Security`), hardened systemd units (`NoNewPrivileges=true`, `ProtectSystem=strict`), and TLS termination. |
| **A06: Vulnerable and Outdated Components** | CI, Cargo Workspace | Weekly dependency audits (`cargo audit`), locked `Cargo.lock` with resolver publish age requirements, and automated third-party license notices verification (`ThirdPartyNotices.txt`). |
| **A07: Identification and Authentication Failures** | `agents-server` (`auth`, `identity`) | Expiring EdDSA/HMAC JWTs, cryptographically secure API keys with hashed storage (`sha256_hex`), rate limiting per IP and subject, and instant token revocation. |
| **A08: Software and Data Integrity Failures** | `agents-boxer`, `agents-installer` | Append-only hash-chained session event logs, SHA-256 verified artifact delivery with SigV4 pre-signing, and detached cryptographic signatures on update manifests. |
| **A09: Security Logging and Monitoring Failures** | `agents-server`, `agents-boxer` | Redacted structured JSONL logging, audit trail of all tool executions and approvals, OTLP tracing without leaking credentials or raw payloads, and 24h retention. |
| **A10: Server-Side Request Forgery (SSRF)** | `agents-boxer` (Egress Proxy) | Blind credential injection proxy, strict allowlist routing, complete raw TCP connection ban outside HTTP/HTTPS, and forbidden access to link-local/cloud metadata IPs (`169.254.169.254`). |

## OWASP Agentic AI Threat Matrix

| Threat Category | Risk Profile | Enforced Mitigation |
|---|---|---|
| **ASI-01: Prompt Injection & Manipulation** | Indirect prompt injection via tool outputs or untrusted web/document retrieval | Dynamic system tool masks per turn, tool call quarantine, strict system/user lane separation, and schema validation before execution. |
| **ASI-02: Tool Misuse & Excessive Agency** | Autonomous destructive mutations or unauthorized system operations | Plan-Act mode requiring explicit human gate approval (`POST /plan/approve`), dry-run preflight, and realpath-serialized mutation queues. |
| **ASI-03: Credential & Egress Leakage** | Agent attempting to exfiltrate API tokens or sensitive host environment keys | Landlock LSM file sandboxing, out-of-band credential proxy injection where sandbox containers only see placeholder keys, and redacted outputs. |
| **ASI-04: Subagent Inception & Runaway DAGs** | Unbounded recursive agent spawning leading to resource starvation or loops | Strict depth limit (`depth <= 2`, depth 3 triggers `SUBAGENT_DEPTH_EXCEEDED`), fencing tokens on lifecycle transitions, and active DAG watchdog. |
| **ASI-05: Inter-Agent Spoofing & Tampering** | Malicious agent forging steering commands or poisoning subagent communication | Mediated chat routes via server control plane, HMAC-SHA256 request signing with nonce validation, and isolated tenant message channels. |
| **ASI-06: Memory & State Poisoning** | Malicious injection into agent long-term memory or session checkpoints | Pure derived snapshots from immutable append-only event logs, cryptographically sealed state rollback checkpoints, and zero in-place mutation. |
| **ASI-07: Supply-Chain & MCP Risks** | Compromised Model Context Protocol servers or untrusted tool definitions | Strict tool schema validation, isolated process sandboxes per MCP connection, and permission narrowing across runtime boundaries. |
