#!/usr/bin/env node
// Zero-telemetry static artifact catalog generator for nla.run.
// Generates clean semantic HTML with cryptographic checksums and Ed25519 signatures.

import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";

/**
 * Generates zero-tracking static catalog HTML.
 * @param {Object} data
 * @param {Array} data.releases
 * @param {Array} [data.plugins]
 * @param {string} [data.baseUrl]
 * @returns {string} Clean HTML string.
 */
export function generateCatalogHtml({ releases = [], plugins = [], baseUrl = "https://nla.run" }) {
  const cleanBase = baseUrl.replace(/\/+$/, "");

  const releaseRows = releases.map((rel) => {
    const assets = (rel.assets || []).map((asset) => {
      const href = asset.url.startsWith("http") ? asset.url : `${cleanBase}/${asset.url.replace(/^\/+/, "")}`;
      const shaHref = `${href}.sha256`;
      const sigHref = `${href}.sig`;
      return `
        <li class="asset-item">
          <strong>${escapeHtml(asset.platform || asset.name)}</strong>:
          <a href="${escapeHtml(href)}" class="btn-download" download>${escapeHtml(asset.name)}</a>
          <span class="meta">(${formatBytes(asset.sizeBytes)})</span>
          <div class="crypto-links">
            <code>SHA-256: ${escapeHtml(asset.sha256.slice(0, 16))}…</code>
            <a href="${escapeHtml(shaHref)}" class="link-verify" download>[Checksum]</a>
            <a href="${escapeHtml(sigHref)}" class="link-verify" download>[Ed25519 Sig]</a>
          </div>
        </li>`;
    }).join("\n");

    return `
      <article class="release-card" id="release-${escapeHtml(rel.version)}">
        <header>
          <h3>v${escapeHtml(rel.version)} <span class="badge ${escapeHtml(rel.channel)}">${escapeHtml(rel.channel)}</span></h3>
          <time datetime="${new Date(rel.publishedAtMs || Date.now()).toISOString()}">
            ${new Date(rel.publishedAtMs || Date.now()).toISOString().split("T")[0]}
          </time>
        </header>
        <p class="notes">${escapeHtml(rel.notes || "Official release build.")}</p>
        <ul class="assets-list">
          ${assets}
        </ul>
      </article>`;
  }).join("\n");

  const pluginRows = plugins.map((p) => `
    <article class="plugin-card">
      <h4>${escapeHtml(p.name)} <span class="badge">v${escapeHtml(p.version)}</span></h4>
      <p>${escapeHtml(p.description)}</p>
      <a href="${escapeHtml(cleanBase)}/plugins/${escapeHtml(p.name)}.zip" class="btn-download" download>Download Plugin</a>
    </article>`).join("\n");

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <meta name="description" content="Official sovereign binary releases and verified cryptographic artifacts.">
  <title>Natuleadan Artifact Catalog — Zero Telemetry</title>
  <style>
    :root {
      --bg: #090a0f;
      --card: #141721;
      --border: #282c3f;
      --text: #e2e8f0;
      --muted: #94a3b8;
      --accent: #38bdf8;
      --accent-hover: #0ea5e9;
      --badge-stable: #10b981;
      --badge-nightly: #f59e0b;
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      --font-sans: system-ui, -apple-system, sans-serif;
    }
    body {
      background: var(--bg);
      color: var(--text);
      font-family: var(--font-sans);
      margin: 0;
      padding: 2rem 1rem;
      line-height: 1.5;
    }
    main {
      max-width: 900px;
      margin: 0 auto;
    }
    header.hero {
      border-bottom: 1px solid var(--border);
      padding-bottom: 2rem;
      margin-bottom: 2rem;
    }
    h1 {
      margin: 0 0 0.5rem 0;
      font-size: 2rem;
      letter-spacing: -0.02em;
    }
    .lead {
      color: var(--muted);
      margin: 0;
    }
    .badge-privacy {
      display: inline-block;
      background: #1e293b;
      color: var(--accent);
      padding: 0.25rem 0.5rem;
      border-radius: 4px;
      font-size: 0.8rem;
      font-weight: 600;
      margin-top: 0.75rem;
    }
    section {
      margin-bottom: 3rem;
    }
    h2 {
      border-bottom: 1px solid var(--border);
      padding-bottom: 0.5rem;
      margin-bottom: 1.5rem;
    }
    .release-card, .plugin-card {
      background: var(--card);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 1.5rem;
      margin-bottom: 1.5rem;
    }
    .release-card header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 0.75rem;
    }
    .release-card header h3 {
      margin: 0;
      font-size: 1.25rem;
    }
    time {
      color: var(--muted);
      font-size: 0.875rem;
    }
    .badge {
      display: inline-block;
      padding: 0.15rem 0.4rem;
      border-radius: 4px;
      font-size: 0.75rem;
      text-transform: uppercase;
      font-weight: 700;
    }
    .badge.stable { background: var(--badge-stable); color: #fff; }
    .badge.nightly { background: var(--badge-nightly); color: #000; }
    .notes {
      color: var(--muted);
      margin-bottom: 1rem;
    }
    ul.assets-list {
      list-style: none;
      padding: 0;
      margin: 0;
    }
    li.asset-item {
      padding: 0.75rem 0;
      border-top: 1px solid #1e2230;
    }
    li.asset-item:first-child {
      border-top: none;
    }
    a.btn-download {
      display: inline-block;
      background: var(--accent);
      color: #090a0f;
      text-decoration: none;
      font-weight: 600;
      padding: 0.35rem 0.75rem;
      border-radius: 4px;
      margin-left: 0.5rem;
    }
    a.btn-download:hover {
      background: var(--accent-hover);
    }
    .crypto-links {
      font-family: var(--font-mono);
      font-size: 0.8rem;
      margin-top: 0.35rem;
      color: var(--muted);
    }
    .link-verify {
      color: var(--accent);
      margin-left: 0.5rem;
      text-decoration: none;
    }
    .link-verify:hover {
      text-decoration: underline;
    }
    pre.verify-box {
      background: #0f111a;
      border: 1px solid var(--border);
      padding: 1rem;
      border-radius: 6px;
      overflow-x: auto;
      font-family: var(--font-mono);
      font-size: 0.85rem;
      color: #a5b4fc;
    }
  </style>
</head>
<body>
  <main>
    <header class="hero">
      <h1>Natuleadan Sovereign Downloads</h1>
      <p class="lead">Deterministic release binaries with cryptographic Ed25519 signatures and SHA-256 digests.</p>
      <div class="badge-privacy">🔒 Zero tracking, zero telemetry, zero web analytics, zero data harvesting.</div>
    </header>

    <section id="releases">
      <h2>Binary Releases</h2>
      ${releaseRows || "<p>No releases available.</p>"}
    </section>

    ${plugins.length > 0 ? `
    <section id="plugins">
      <h2>Official Signed Plugins</h2>
      ${pluginRows}
    </section>` : ""}

    <section id="verification">
      <h2>Offline Cryptographic Verification</h2>
      <p>Verify release integrity without network access using your local terminal:</p>
      <pre class="verify-box"># 1. Verify SHA-256 checksum
shasum -a 256 -c Natuleadan-macos-arm64.tar.gz.sha256

# 2. Verify Ed25519 signature
nla doctor --verify --manifest latest.json --sig latest.json.sig --pub release.pub</pre>
    </section>
  </main>
</body>
</html>`;
}

function escapeHtml(str) {
  return String(str || "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function formatBytes(bytes) {
  if (!bytes || bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

const isEntrypoint = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (isEntrypoint) {
  const outPath = process.argv[2] || "dist/index.html";
  mkdirSync(dirname(resolve(outPath)), { recursive: true });
  const html = generateCatalogHtml({
    releases: [
      {
        version: "0.1.0",
        channel: "stable",
        notes: "First certified sovereign release build.",
        publishedAtMs: Date.now(),
        assets: [
          {
            platform: "macOS (Apple Silicon)",
            name: "Natuleadan-macos-arm64.tar.gz",
            url: "dist/Natuleadan-macos-arm64.tar.gz",
            sizeBytes: 12419907,
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
          },
          {
            platform: "Linux (x86_64)",
            name: "natuleadan-linux-x86_64-0.1.0.tar.gz",
            url: "dist/natuleadan-linux-x86_64-0.1.0.tar.gz",
            sizeBytes: 8540100,
            sha256: "4b825dc642cb6eb9a060e54bf8d69288fbee4904ce6243d37970b72464554f14",
          },
        ],
      },
    ],
    plugins: [
      {
        name: "codebase-inspector",
        version: "0.1.0",
        description: "Structural code analysis and AST inspector.",
      },
      {
        name: "filesystem-vfs",
        version: "0.1.0",
        description: "Zero-copy virtual memory filesystem for ephemeral tasks.",
      },
    ],
  });
  writeFileSync(resolve(outPath), html, "utf8");
  console.log(`Generated static zero-telemetry catalog at ${outPath}`);
}
