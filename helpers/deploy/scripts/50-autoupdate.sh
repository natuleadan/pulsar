#!/usr/bin/env bash
set -euo pipefail

# Automated rolling update for pulsar-server and pulsar-boxer daemons.
# Fetches release-packaged binaries from GitHub Releases without local compilation.

REPO="${PULSAR_REPO:-natuleadan/pulsar}"
RELEASE_TAG="${PULSAR_RELEASE_TAG:-nightly-dev}"
STATE_FILE="${PULSAR_STATE_FILE:-/etc/pulsar/installed-sha}"
BIN_DIR="${PULSAR_BIN_DIR:-/usr/local/bin}"
HEALTH_URL="${PULSAR_HEALTH_URL:-http://127.0.0.1:8080/health/ready}"
DRY_RUN="${PULSAR_DRY_RUN:-0}"
CHECK_ONLY="0"
INSTALL_TIMER="0"

usage() {
  cat <<'EOF'
Usage: 50-autoupdate.sh [--check] [--apply] [--install-timer] [--repo org/repo] [--tag tag] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      CHECK_ONLY="1"
      shift
      ;;
    --apply)
      CHECK_ONLY="0"
      shift
      ;;
    --install-timer)
      INSTALL_TIMER="1"
      shift
      ;;
    --repo)
      REPO="${2:?missing value for --repo}"
      shift 2
      ;;
    --tag)
      RELEASE_TAG="${2:?missing value for --tag}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN="1"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "[FAIL] unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 && "${CHECK_ONLY}" != "1" ]]; then
  echo "[FAIL] 50-autoupdate.sh must run as root to install binaries (use --check or --dry-run)" >&2
  exit 1
fi

if [[ "${INSTALL_TIMER}" == "1" ]]; then
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] install ${BIN_DIR}/pulsar-autoupdate and enable systemd pulsar-autoupdate.timer"
    exit 0
  fi
  echo "[INFO] Installing systemd autoupdate timer..."
  install -d -m 0755 "${BIN_DIR}"
  if [[ "$(readlink -f "${BASH_SOURCE[0]}")" != "$(readlink -f "${BIN_DIR}/pulsar-autoupdate" 2>/dev/null || true)" ]]; then
    install -m 0755 "${BASH_SOURCE[0]}" "${BIN_DIR}/pulsar-autoupdate"
  fi

  cat >/etc/systemd/system/pulsar-autoupdate.service <<EOF
[Unit]
Description=Pulsar Rolling Nightly Autoupdater
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=${BIN_DIR}/pulsar-autoupdate --apply
StandardOutput=journal
StandardError=journal
EOF

  cat >/etc/systemd/system/pulsar-autoupdate.timer <<'EOF'
[Unit]
Description=Periodic Check for Pulsar Nightly Updates

[Timer]
OnBootSec=2min
OnUnitActiveSec=5min
RandomizedDelaySec=30s
Persistent=true

[Install]
WantedBy=timers.target
EOF

  systemctl daemon-reload
  systemctl enable --now pulsar-autoupdate.timer
  echo "[PASS] Systemd autoupdate timer installed and active (runs every 5 minutes)."
  exit 0
fi

# 1. Query GitHub for latest dev commit SHA
LATEST_SHA=$(curl -fsSL -H "Accept: application/vnd.github.v3+json" \
  "https://api.github.com/repos/${REPO}/commits/dev" |
  python3 -c "import json, sys; print(json.load(sys.stdin).get('sha', '')[:7])" 2>/dev/null || true)

CURRENT_SHA=""
if [[ -f "${STATE_FILE}" ]]; then
  CURRENT_SHA=$(tr -d ' \n\r' <"${STATE_FILE}" | cut -c1-7)
fi

echo "[INFO] Current installed SHA: ${CURRENT_SHA:-unknown}"
echo "[INFO] Latest upstream dev SHA: ${LATEST_SHA:-unknown}"

if [[ -n "${LATEST_SHA}" && "${LATEST_SHA}" == "${CURRENT_SHA}" ]]; then
  echo "[PASS] Pulsar daemons already at latest dev commit: ${CURRENT_SHA}"
  exit 0
fi

if [[ "${CHECK_ONLY}" == "1" ]]; then
  echo "[INFO] Update available: ${CURRENT_SHA:-unknown} -> ${LATEST_SHA:-unknown}"
  exit 0
fi

TARBALL_URL="https://github.com/${REPO}/releases/download/${RELEASE_TAG}/pulsar-daemons-x86_64-unknown-linux-gnu.tar.gz"
TMP_DIR=$(mktemp -d /tmp/pulsar-update-XXXXXX)
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "[INFO] Downloading release package from ${TARBALL_URL}..."
curl -fsSL "${TARBALL_URL}" -o "${TMP_DIR}/daemons.tar.gz"

echo "[INFO] Extracting release binaries..."
tar -xzf "${TMP_DIR}/daemons.tar.gz" -C "${TMP_DIR}"

if [[ ! -f "${TMP_DIR}/pulsar-server" || ! -f "${TMP_DIR}/pulsar-boxer" ]]; then
  echo "[FAIL] Downloaded package does not contain pulsar-server or pulsar-boxer" >&2
  exit 1
fi

# Backup existing binaries if present
BACKUP_DIR="/var/lib/pulsar/backup"
mkdir -p "${BACKUP_DIR}"
if [[ -f "${BIN_DIR}/pulsar-server" ]]; then
  cp -p "${BIN_DIR}/pulsar-server" "${BACKUP_DIR}/pulsar-server.prev"
fi
if [[ -f "${BIN_DIR}/pulsar-boxer" ]]; then
  cp -p "${BIN_DIR}/pulsar-boxer" "${BACKUP_DIR}/pulsar-boxer.prev"
fi

# Atomic install (unlinks inode so text file busy error is avoided)
echo "[INFO] Installing binaries to ${BIN_DIR}..."
install -m 0755 "${TMP_DIR}/pulsar-server" "${BIN_DIR}/pulsar-server"
install -m 0755 "${TMP_DIR}/pulsar-boxer" "${BIN_DIR}/pulsar-boxer"

# Record installed SHA
mkdir -p "$(dirname "${STATE_FILE}")"
echo "${LATEST_SHA}" >"${STATE_FILE}"

# Restart systemd daemons
echo "[INFO] Restarting systemd daemons..."
systemctl restart pulsar-server pulsar-boxer

# Verify health check
echo "[INFO] Verifying health check on ${HEALTH_URL}..."
HEALTHY=0
for attempt in $(seq 1 15); do
  echo "[INFO] Health check attempt ${attempt}/15..."
  if curl -fsS "${HEALTH_URL}" >/dev/null 2>&1; then
    HEALTHY=1
    break
  fi
  sleep 1
done

if [[ "${HEALTHY}" -eq 1 ]]; then
  echo "[PASS] Successfully updated Pulsar to ${LATEST_SHA}. Daemons active and healthy."
else
  echo "[WARN] Health check did not pass within 15s. Rolling back..." >&2
  if [[ -f "${BACKUP_DIR}/pulsar-server.prev" ]]; then
    install -m 0755 "${BACKUP_DIR}/pulsar-server.prev" "${BIN_DIR}/pulsar-server"
    install -m 0755 "${BACKUP_DIR}/pulsar-boxer.prev" "${BIN_DIR}/pulsar-boxer"
    systemctl restart pulsar-server pulsar-boxer
  fi
  echo "[FAIL] Rollback complete. Inspect systemctl status pulsar-server." >&2
  exit 1
fi
