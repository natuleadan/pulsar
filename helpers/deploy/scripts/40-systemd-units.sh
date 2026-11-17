#!/usr/bin/env bash
set -euo pipefail

# Install hardened systemd service units and logrotate rules for pulsar-server and pulsar-boxer.
ROLE="monolithic"
ENV_FILE="/etc/pulsar/pulsar.env"
BIN_DIR="/usr/local/bin"
DRY_RUN="${PULSAR_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 40-systemd-units.sh [--role monolithic|server|boxer] [--env-file path] [--bin-dir dir] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:?missing value for --role}"
      shift 2
      ;;
    --env-file)
      ENV_FILE="${2:?missing value for --env-file}"
      shift 2
      ;;
    --bin-dir)
      BIN_DIR="${2:?missing value for --bin-dir}"
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

case "${ROLE}" in
  monolithic | server | boxer) ;;
  *)
    echo "[FAIL] unsupported role: ${ROLE}" >&2
    exit 2
    ;;
esac

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 40-systemd-units.sh must run as root (use --dry-run to inspect)" >&2
  exit 1
fi

write_file() {
  local dest="$1"
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] write ${dest}"
    cat >/dev/null
  else
    mkdir -p "$(dirname "${dest}")"
    cat >"${dest}"
  fi
}

run_cmd() {
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] $*"
  else
    "$@"
  fi
}

if [[ "${DRY_RUN}" != "1" ]]; then
  if ! id -u pulsar >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin pulsar
  fi
  install -d -m 0750 -o root -g pulsar /etc/pulsar
  if [[ -f "${ENV_FILE}" ]]; then
    chown root:pulsar "${ENV_FILE}"
    chmod 0640 "${ENV_FILE}"
  fi
  install -d -m 0750 -o pulsar -g pulsar /var/lib/pulsar /var/lib/pulsar/boxes /var/log/pulsar
else
  echo "[DRY-RUN] ensure unprivileged system user 'pulsar' and /etc/pulsar, /var/lib/pulsar"
fi

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  write_file /etc/systemd/system/pulsar-server.service <<EOF
[Unit]
Description=Pulsar Ironic Orchestrator Server
After=network-online.target postgresql.service docker.service
Wants=network-online.target docker.service

[Service]
Type=simple
User=pulsar
Group=pulsar
Environment="OTEL_SERVICE_NAME=pulsar-server"
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_DIR}/pulsar-server
Restart=on-failure
RestartSec=3
TimeoutStopSec=30
KillSignal=SIGTERM
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectControlGroups=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths=/var/lib/pulsar /var/log/pulsar

[Install]
WantedBy=multi-user.target
EOF
fi

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  write_file /etc/systemd/system/pulsar-boxer.service <<EOF
[Unit]
Description=Pulsar Boxer Sandbox Supervisor
After=network-online.target docker.service
Wants=network-online.target docker.service

[Service]
Type=simple
User=pulsar
Group=pulsar
Environment="OTEL_SERVICE_NAME=pulsar-boxer"
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_DIR}/pulsar-boxer
Restart=on-failure
RestartSec=3
TimeoutStopSec=30
KillSignal=SIGTERM
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ProtectKernelTunables=true
ProtectControlGroups=true
CapabilityBoundingSet=
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
ReadWritePaths=/var/lib/pulsar /var/log/pulsar

[Install]
WantedBy=multi-user.target
EOF
fi

write_file /etc/logrotate.d/pulsar <<'EOF'
/var/log/pulsar/*.log {
  daily
  rotate 14
  compress
  delaycompress
  missingok
  notifempty
  create 0640 pulsar pulsar
}
EOF

run_cmd systemctl daemon-reload
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  run_cmd systemctl enable --now pulsar-server.service
  run_cmd systemctl restart pulsar-server.service
fi
if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  run_cmd systemctl enable --now pulsar-boxer.service
  run_cmd systemctl restart pulsar-boxer.service
fi

echo "[PASS] Hardened systemd units and logrotate installed for role=${ROLE}"
