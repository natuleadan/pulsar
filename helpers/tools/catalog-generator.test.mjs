import assert from "node:assert/strict";
import test from "node:test";
import { generateCatalogHtml } from "./catalog-generator.mjs";

test("catalog-generator outputs strict zero-telemetry semantic HTML with checksums", () => {
  const html = generateCatalogHtml({
    releases: [
      {
        version: "0.2.0",
        channel: "stable",
        notes: "Major security and multiplatform release.",
        publishedAtMs: 1700000000000,
        assets: [
          {
            platform: "macOS aarch64",
            name: "Natuleadan-macos-arm64.dmg",
            url: "dist/Natuleadan-macos-arm64.dmg",
            sizeBytes: 15000000,
            sha256: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
          },
        ],
      },
    ],
    plugins: [
      {
        name: "test-plugin",
        version: "1.0.0",
        description: "Official sandbox plugin.",
      },
    ],
    baseUrl: "https://nla.run",
  });

  // Zero tracking & zero script tags
  assert.ok(!html.includes("<script"), "Catalog must not embed any script tags (zero telemetry)");
  assert.ok(!html.includes("google-analytics"), "No analytics allowed");
  assert.ok(!html.includes("cookie"), "No cookie references or trackers allowed");

  // Content verification
  assert.ok(html.includes("v0.2.0"), "Must include release version");
  assert.ok(html.includes("Natuleadan-macos-arm64.dmg"), "Must include asset name");
  assert.ok(html.includes("Natuleadan-macos-arm64.dmg.sha256"), "Must provide checksum link");
  assert.ok(html.includes("Natuleadan-macos-arm64.dmg.sig"), "Must provide Ed25519 signature link");
  assert.ok(html.includes("test-plugin"), "Must include official plugin");
  assert.ok(html.includes("shasum -a 256 -c"), "Must include offline verification guide");
});
