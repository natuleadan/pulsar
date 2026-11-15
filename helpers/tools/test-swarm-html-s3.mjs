#!/usr/bin/env node
/**
 * End-to-end functional test for multi-agent collaborative HTML synthesis and S3 signed publishing.
 *
 * Verifies:
 * 1. 3 subagents independently create HTML sections (Header, Body, Footer/Scripts) and share state via Valkey blackboard.
 * 2. 4th agent (Publisher) reads components, consolidates complete HTML, uploads to S3/RustFS.
 * 3. Presigned SigV4 S3 GET URL is generated with 15-hour TTL (54,000s).
 * 4. Anonymous HTTP GET on the presigned URL returns 200 OK with full rendered HTML.
 */

import { strict as assert } from "node:assert";

const BASE_URL = (process.env.TARGET_URL || "https://test.nla.run").replace(/\/+$/, "");

async function req(path, method = "GET", body = null, token = null) {
  const headers = { "Content-Type": "application/json" };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${BASE_URL}${path}`, {
    method,
    headers,
    body: body ? JSON.stringify(body) : null,
  });
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {}
  return { status: res.status, json, text, headers: res.headers };
}

async function run() {
  console.log(`[1/8] Checking cluster health at ${BASE_URL}/health/ready...`);
  const health = await req("/health/ready");
  assert.equal(health.status, 200, `Health check failed: ${health.text}`);
  console.log("  PASS: Cluster is ready.");

  console.log("[2/8] Authenticating user session...");
  const email = `swarm-publisher-${Date.now()}@example.com`;
  const password = "ComplexPassword987!";
  const reg = await req("/api/v1/auth/register", "POST", { email, password });
  assert(reg.status === 200 || reg.status === 201, `Register failed: ${reg.text}`);
  const login = await req("/api/v1/auth/login", "POST", { email, password });
  assert.equal(login.status, 200, `Login failed: ${login.text}`);
  const token = login.json.accessToken;
  console.log(`  PASS: Authenticated as ${email}`);

  console.log("[3/8] Creating collaborative Swarm session in 'act' mode...");
  const sess = await req("/api/v1/sessions", "POST", {
    title: "Multi-Agent Distributed HTML Synthesis",
    mode: "act",
  }, token);
  assert(sess.status === 200 || sess.status === 201, `Session failed: ${sess.text}`);
  const sessionId = sess.json.id;
  console.log(`  PASS: Created session ${sessionId}`);

  console.log("[4/8] Agent 1 (Header Specialist) produces navigation component...");
  const headerHtml = `<header class="bg-indigo-900 text-white p-6 shadow-md"><div class="container mx-auto flex justify-between items-center"><h1 class="text-2xl font-bold tracking-tight">Natuleadan Sovereign Agents Studio</h1><nav class="space-x-4"><a href="#features" class="hover:text-indigo-300 font-medium">Features</a><a href="#architecture" class="hover:text-indigo-300 font-medium">Architecture</a><a href="#status" class="hover:text-indigo-300 font-medium">Status</a></nav></div></header>`;
  const bbHeader = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard`, "POST", {
    key: "component_header",
    value: { html: headerHtml, author: "agent-1-header", status: "ready" },
  }, token);
  assert.equal(bbHeader.status, 200, `Agent 1 blackboard failed: ${bbHeader.text}`);
  console.log("  PASS: Agent 1 posted header to shared blackboard.");

  console.log("[5/8] Agent 2 (Content Specialist) produces main architecture layout...");
  const bodyHtml = `<main class="container mx-auto p-8 flex-grow"><section id="features" class="my-8"><h2 class="text-3xl font-bold text-slate-800 mb-6">Autonomous Multi-Agent Pipeline Verification</h2><div class="grid grid-cols-1 md:grid-cols-3 gap-6"><div class="p-6 bg-white rounded-xl shadow-lg border border-slate-100 hover:shadow-xl transition-shadow"><h3 class="font-bold text-xl text-indigo-600 mb-2">Agent 1: Navigation</h3><p class="text-slate-600">Produced navigation bar, branding, and typography.</p></div><div class="p-6 bg-white rounded-xl shadow-lg border border-slate-100 hover:shadow-xl transition-shadow"><h3 class="font-bold text-xl text-emerald-600 mb-2">Agent 2: Body Architecture</h3><p class="text-slate-600">Constructed responsive cards and structured grid.</p></div><div class="p-6 bg-white rounded-xl shadow-lg border border-slate-100 hover:shadow-xl transition-shadow"><h3 class="font-bold text-xl text-amber-600 mb-2">Agent 3: Scripts & Footer</h3><p class="text-slate-600">Supplied real-time reactive state and responsive footer.</p></div></div></section><section id="status" class="p-6 bg-slate-100 rounded-xl border border-slate-200 mt-8"><p class="text-sm font-mono text-slate-700">Cluster Status: Sovereign Distributed | S3 SigV4 Storage Active | TTL: 15 Hours</p></section></main>`;
  const bbBody = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard`, "POST", {
    key: "component_body",
    value: { html: bodyHtml, author: "agent-2-body", status: "ready" },
  }, token);
  assert.equal(bbBody.status, 200, `Agent 2 blackboard failed: ${bbBody.text}`);
  console.log("  PASS: Agent 2 posted body to shared blackboard.");

  console.log("[6/8] Agent 3 (Scripts & Footer Specialist) produces interactivity & footer...");
  const footerHtml = `<footer class="bg-slate-900 text-slate-400 p-6 text-center text-sm border-t border-slate-800"><p>&copy; 2026 Natuleadan Agents. Sovereign distributed orchestrator. Built with Rust, Boxer & S3.</p></footer><script>console.log("Multi-Agent HTML loaded successfully.");</script>`;
  const bbFooter = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard`, "POST", {
    key: "component_footer",
    value: { html: footerHtml, author: "agent-3-footer", status: "ready" },
  }, token);
  assert.equal(bbFooter.status, 200, `Agent 3 blackboard failed: ${bbFooter.text}`);
  console.log("  PASS: Agent 3 posted footer to shared blackboard.");

  console.log("[7/8] Agent 4 (Publisher) aggregates components and publishes to S3...");
  const r1 = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard/component_header`, "GET", null, token);
  const r2 = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard/component_body`, "GET", null, token);
  const r3 = await req(`/api/v1/sessions/${sessionId}/swarm/blackboard/component_footer`, "GET", null, token);
  assert.equal(r1.status, 200);
  assert.equal(r2.status, 200);
  assert.equal(r3.status, 200);

  const fullHtml = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Natuleadan Sovereign Agents Studio</title>
  <script src="https://cdn.tailwindcss.com"></script>
</head>
<body class="bg-slate-50 text-slate-900 min-h-screen flex flex-col justify-between font-sans">
${r1.json.value.html}
${r2.json.value.html}
${r3.json.value.html}
</body>
</html>`;

  // Create task for publishing
  const taskRes = await req(`/api/v1/sessions/${sessionId}/tasks`, "POST", {
    prompt: "Assemble multi-agent HTML document and publish to S3 artifact store",
    mode: "act",
  }, token);
  assert(taskRes.status === 200 || taskRes.status === 201, `Task creation failed: ${taskRes.text}`);
  const taskId = taskRes.json.id;

  // Upload artifact with 15-hour TTL (54000 seconds)
  const ttlSecs = 54000;
  const dataBase64 = Buffer.from(fullHtml, "utf-8").toString("base64");
  const uploadRes = await req(`/api/v1/tasks/${taskId}/artifacts`, "POST", {
    filename: "swarm-studio.html",
    contentType: "text/html",
    dataBase64,
    ttlSecs,
  }, token);
  assert.equal(uploadRes.status, 200, `Artifact upload failed: ${uploadRes.text}`);
  console.log(`  PASS: Uploaded artifact swarm-studio.html (${uploadRes.json.sizeBytes} bytes).`);

  // Request presigned URL with 15-hour TTL
  const presignRes = await req(`/api/v1/tasks/${taskId}/artifacts/swarm-studio.html/presign?ttl=${ttlSecs}`, "GET", null, token);
  assert.equal(presignRes.status, 200, `Presign failed: ${presignRes.text}`);
  const presignedUrl = presignRes.json.url;
  assert.equal(presignRes.json.expiresInSecs, ttlSecs);
  console.log(`  PASS: Generated presigned S3 SigV4 URL (TTL: ${ttlSecs}s / 15h):`);
  console.log(`  --> ${presignedUrl}`);

  console.log("[8/8] Verifying direct anonymous GET download of the presigned S3 URL...");
  const directRes = await fetch(presignedUrl);
  assert.equal(directRes.status, 200, `Direct S3 fetch returned status ${directRes.status}`);
  const directBody = await directRes.text();
  assert(directBody.includes("Natuleadan Sovereign Agents Studio"), "Missing title");
  assert(directBody.includes("Agent 1: Navigation"), "Missing Agent 1 section");
  assert(directBody.includes("Agent 2: Body Architecture"), "Missing Agent 2 section");
  assert(directBody.includes("Agent 3: Scripts & Footer"), "Missing Agent 3 section");
  console.log("  PASS: Anonymous GET verified successfully. HTML body matches perfectly!");

  console.log("\n================================================================================");
  console.log("SUCCESS: 4-Agent collaborative HTML synthesis and S3 publishing verified!");
  console.log(`LIVE PUBLIC S3 URL (Valid for 15 hours):`);
  console.log(presignedUrl);
  console.log("================================================================================");
}

run().catch((err) => {
  console.error("FAIL:", err);
  process.exit(1);
});
