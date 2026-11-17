// Live SSH bootstrap & HTTP verification helpers for Mode A (Monolithic) and Mode B (Distributed VLAN).
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

const SSH_OPTS = [
  "-o",
  "BatchMode=yes",
  "-o",
  "StrictHostKeyChecking=no",
  "-o",
  "UserKnownHostsFile=/dev/null",
  "-o",
  "ConnectTimeout=5",
];

export function resolveBinaryDir(root) {
  if (process.env.PULSAR_BIN_DIR && existsSync(join(process.env.PULSAR_BIN_DIR, "pulsar-server"))) {
    return process.env.PULSAR_BIN_DIR;
  }
  const targets = [
    join(root, "target/x86_64-unknown-linux-gnu/release"),
    join(root, "target/x86_64-unknown-linux-gnu/debug"),
    join(root, "target/release"),
    join(root, "target/debug"),
  ];
  for (const dir of targets) {
    if (existsSync(join(dir, "pulsar-server"))) return dir;
  }
  throw new Error("Missing built pulsar-server binary; specify PULSAR_BIN_DIR or build target");
}

function runSsh(keyPath, targetHost, cmd, jumpHost = "") {
  const args = [...SSH_OPTS, "-i", keyPath];
  if (jumpHost) args.push("-J", jumpHost);
  args.push(targetHost, cmd);
  return execFileSync("ssh", args, { encoding: "utf8" });
}

function runScp(keyPath, srcPath, dstTarget, jumpHost = "") {
  const args = [...SSH_OPTS, "-i", keyPath, "-r"];
  if (jumpHost) args.push("-o", `ProxyJump=${jumpHost}`);
  args.push(srcPath, dstTarget);
  execFileSync("scp", args, { stdio: "pipe" });
}

async function waitForSsh(keyPath, host, maxWaitMs = 180_000, jumpHost = "") {
  const start = Date.now();
  while (Date.now() - start < maxWaitMs) {
    try {
      runSsh(keyPath, host, "echo ssh-ready", jumpHost);
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 4000));
    }
  }
  throw new Error(`Timed out waiting for SSH on ${host}`);
}

async function waitForHttpOk(url, timeoutMs = 60_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const r = await fetch(url);
      if (r.ok) return await r.text();
    } catch {
      // retry until daemon listens
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error(`Timed out waiting for HTTP 200 on ${url}`);
}

function deployBinaries(root, binDir, keyPath, targetHost, jumpHost = "") {
  runSsh(keyPath, targetHost, "mkdir -p /opt/pulsar /usr/local/bin", jumpHost);
  for (const bin of ["pulsar-server", "pulsar-boxer", "pulsar-installer"]) {
    const src = join(binDir, bin);
    if (existsSync(src)) {
      runScp(keyPath, src, `${targetHost}:/usr/local/bin/${bin}`, jumpHost);
    }
  }
  runScp(keyPath, join(root, "helpers/deploy/scripts"), `${targetHost}:/opt/pulsar/`, jumpHost);
  runSsh(
    keyPath,
    targetHost,
    "chmod +x /usr/local/bin/pulsar-* /opt/pulsar/scripts/*.sh && bash /opt/pulsar/scripts/00-preflight.sh",
    jumpHost,
  );
}

export async function verifyRemoteMonolithic({ root, vps, keyPath, keepDemo, logger }) {
  const binDir = resolveBinaryDir(root);
  const host = `root@${vps.publicIpv4}`;
  logger(`[LIVE] Waiting for SSH on monolithic VPS ${vps.publicIpv4}...`);
  await waitForSsh(keyPath, host);
  deployBinaries(root, binDir, keyPath, host);

  const jwtSecret = execFileSync("openssl", ["rand", "-hex", "32"], { encoding: "utf8" }).trim();
  const publicUrl = `http://${vps.publicIpv4}:8080`;
  const startCmd = [
    "nohup env",
    "PULSAR_BIND_ADDR=0.0.0.0:8080",
    `PULSAR_PUBLIC_URL=${publicUrl}`,
    `PULSAR_TRUSTED_ORIGINS=${publicUrl}`,
    `PULSAR_JWT_SECRET=${jwtSecret}`,
    "PULSAR_RATE_LIMIT_MAX=1000",
    "/usr/local/bin/pulsar-server serve >/var/log/pulsar-server.log 2>&1 </dev/null &",
  ].join(" ");
  runSsh(keyPath, host, startCmd);

  const ready = await waitForHttpOk(`${publicUrl}/health/ready`);
  const html = await waitForHttpOk(`${publicUrl}/`);
  if (!html.includes("Pulsar") || !html.includes("login-screen")) {
    throw new Error(`Unexpected root Web UI payload from ${publicUrl}/`);
  }
  logger(`[LIVE] Monolithic OK: ${publicUrl}/health/ready (${ready.slice(0, 60)})`);
  if (keepDemo) {
    logger(`[DEMO] Monolithic Web UI ready at ${publicUrl}/`);
  }
}

export async function verifyRemoteDistributed({ root, serverVps, boxerVps, keyPath, keepDemo, logger }) {
  const binDir = resolveBinaryDir(root);
  const srvHost = `root@${serverVps.publicIpv4}`;
  const boxHost = `root@${boxerVps.vlanIp}`;
  logger(`[LIVE] Waiting for SSH on distributed server ${serverVps.publicIpv4}...`);
  await waitForSsh(keyPath, srvHost);
  deployBinaries(root, binDir, keyPath, srvHost);

  const jwtSecret = execFileSync("openssl", ["rand", "-hex", "32"], { encoding: "utf8" }).trim();
  const publicUrl = `http://${serverVps.publicIpv4}:8080`;
  runSsh(
    keyPath,
    srvHost,
    [
      "nohup env",
      "PULSAR_BIND_ADDR=0.0.0.0:8080",
      `PULSAR_PUBLIC_URL=${publicUrl}`,
      `PULSAR_TRUSTED_ORIGINS=${publicUrl}`,
      `PULSAR_JWT_SECRET=${jwtSecret}`,
      "PULSAR_RATE_LIMIT_MAX=1000",
      "/usr/local/bin/pulsar-server serve >/var/log/pulsar-server.log 2>&1 </dev/null &",
    ].join(" "),
  );
  await waitForHttpOk(`${publicUrl}/health/ready`);

  logger(`[LIVE] Bootstrapping private-VLAN boxer ${boxerVps.vlanIp} via jump host ${serverVps.publicIpv4}...`);
  await waitForSsh(keyPath, boxHost, 120_000, srvHost);
  deployBinaries(root, binDir, keyPath, boxHost, srvHost);
  runSsh(
    keyPath,
    boxHost,
    `nohup env BOXER_NODE_ID=${boxerVps.name} SERVER_INTERNAL_URL=http://${serverVps.vlanIp}:8080 NATS_URL=nats://${serverVps.vlanIp}:4222 /usr/local/bin/pulsar-boxer >/var/log/pulsar-boxer.log 2>&1 </dev/null &`,
    srvHost,
  );
  const html = await waitForHttpOk(`${publicUrl}/`);
  if (!html.includes("Pulsar") || !html.includes("login-screen")) {
    throw new Error(`Unexpected root Web UI payload from ${publicUrl}/`);
  }
  logger(`[LIVE] Distributed control plane OK at ${publicUrl}/ (private boxer ${boxerVps.vlanIp})`);
  if (keepDemo) {
    logger(`[DEMO] Distributed Web UI ready at ${publicUrl}/`);
  }
}
