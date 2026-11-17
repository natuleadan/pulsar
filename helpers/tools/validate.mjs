#!/usr/bin/env node
// Unified validation runner (Node, no shell scripts).
// Runs every check with a timeout, captures output, prints PASS/FAIL and
// exits non-zero if any check fails (G-27).
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const ROOT = process.cwd();
const TIMEOUT_MS = 300_000;
const MAX_LINES = 500;
const CHECKS = [];

function command(name, cmd, args) {
  CHECKS.push({ name, run: () => {
    const r = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", timeout: TIMEOUT_MS });
    if (r.error) return { ok: false, output: String(r.error) };
    const output = ((r.stdout || "") + (r.stderr || "")).trim();
    return { ok: r.status === 0, output };
  } });
}

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    const st = statSync(p);
    if (st.isDirectory()) {
      if (["target", "node_modules", ".git", ".nsh", "dist"].includes(name)) continue;
      walk(p, out);
    } else if (/\.(rs|md|sh)$/.test(name)) {
      out.push(p);
    }
  }
  return out;
}

function checkFileSize() {
  CHECKS.push({ name: "file size", run: () => {
    const bad = walk(ROOT)
      .map((f) => [f, readFileSync(f, "utf8").split("\n").length])
      .filter(([, n]) => n > MAX_LINES)
      .map(([f, n]) => `${relative(ROOT, f)} has ${n} lines`);
    return { ok: bad.length === 0, output: bad.join("\n") };
  } });
}


// --- checks common to all repos ---
command("cargo fmt", "cargo", ["fmt", "--all", "--", "--check"]);
command("cargo clippy", "cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
command("cargo test", "cargo", ["test", "--workspace"]);
command("cargo check", "cargo", ["--config", 'resolver.min-publish-age="7 days"', "check", "--workspace"]);
command("notices", "node", ["helpers/tools/third-party-notices.mjs", "--check"]);
checkFileSize();

// --- repo-specific checks are injected below ---

command("e2e-cloud test", "node", ["--test", "helpers/tools/e2e-cloud.test.mjs"]);
command("release-manifest test", "node", ["--test", "helpers/tools/release-manifest.test.mjs"]);
command("catalog-generator test", "node", ["--test", "helpers/tools/catalog-generator.test.mjs"]);
command("check-scripts", "node", ["helpers/tools/check-scripts.mjs"]);
command("release-plan", "node", ["helpers/tools/release-plan.ts", "--check"]);


let failures = 0;
for (const c of CHECKS) {
  const { ok, output } = c.run();
  if (ok) {
    console.log(`PASS ${c.name}`);
  } else {
    failures += 1;
    console.log(`FAIL ${c.name}`);
    if (output) console.log(output.split("\n").slice(-40).join("\n"));
  }
}
console.log(`TOTAL PASS=${CHECKS.length - failures} FAIL=${failures}`);
process.exit(failures === 0 ? 0 : 1);
