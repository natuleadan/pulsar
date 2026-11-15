#!/usr/bin/env bash
set -euo pipefail

# Zero-touch, provider-agnostic VPS bootstrap orchestrator.
# Always enforces HTTPS (ACME when --domain is provided, self-signed TLS on IP otherwise).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ROLE="monolithic"
DOMAIN=""
PREVIEW_DOMAIN=""
ACME_EMAIL="ops@example.com"
VLAN_CIDR="10.0.0.0/8"
BIND_IP="127.0.0.1"
ENV_FILE="/etc/agents/agents.env"
BIN_DIR="/usr/local/bin"
ENABLE_OTEL="1"
INFERENCE_PROVIDER="${AGENTS_LLM_PROVIDER:-ollama}"
INFERENCE_ENDPOINT="${AGENTS_LLM_ENDPOINT:-http://127.0.0.1:11434/v1}"
INFERENCE_MODEL="${AGENTS_LLM_MODEL:-llama3.2}"
ADMIN_EMAIL="${AGENTS_ADMIN_EMAIL:-}"
ADMIN_PASSWORD="${AGENTS_ADMIN_INITIAL_PASSWORD:-}"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: setup.sh [--role monolithic|server|boxer] [--domain host] [--preview-domain host] [--acme-email email] [--vlan-cidr cidr] [--bind-ip ip] [--env-file path] [--bin-dir dir] [--enable-otel 0|1] [--inference-provider name] [--inference-endpoint url] [--inference-model model] [--admin-email email] [--admin-password pass] [--dry-run]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --role)
      ROLE="${2:?missing value for --role}"
      shift 2
      ;;
    --domain)
      DOMAIN="${2:?missing value for --domain}"
      shift 2
      ;;
    --preview-domain)
      PREVIEW_DOMAIN="${2:?missing value for --preview-domain}"
      shift 2
      ;;
    --acme-email)
      ACME_EMAIL="${2:?missing value for --acme-email}"
      shift 2
      ;;
    --vlan-cidr)
      VLAN_CIDR="${2:?missing value for --vlan-cidr}"
      shift 2
      ;;
    --bind-ip)
      BIND_IP="${2:?missing value for --bind-ip}"
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
    --enable-otel)
      ENABLE_OTEL="${2:?missing value for --enable-otel}"
      shift 2
      ;;
    --inference-provider)
      INFERENCE_PROVIDER="${2:?missing value for --inference-provider}"
      shift 2
      ;;
    --inference-endpoint)
      INFERENCE_ENDPOINT="${2:?missing value for --inference-endpoint}"
      shift 2
      ;;
    --inference-model)
      INFERENCE_MODEL="${2:?missing value for --inference-model}"
      shift 2
      ;;
    --admin-email)
      ADMIN_EMAIL="${2:?missing value for --admin-email}"
      shift 2
      ;;
    --admin-password)
      ADMIN_PASSWORD="${2:?missing value for --admin-password}"
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

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] setup.sh must run as root (use --dry-run to inspect)" >&2
  exit 1
fi

gen_hex() {
  openssl rand -hex "${1:-32}"
}

ensure_env_file() {
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[DRY-RUN] ensure ${ENV_FILE} with mode 0600 and default inference=${INFERENCE_PROVIDER} (${INFERENCE_ENDPOINT})"
    return 0
  fi

  mkdir -p "$(dirname "${ENV_FILE}")"
  if [[ ! -f "${ENV_FILE}" ]]; then
    local pg_pass valkey_pass jwt_secret master_key s3_ak s3_sk public_url admin_email admin_pass
    pg_pass="$(gen_hex 18)"
    valkey_pass="$(gen_hex 18)"
    jwt_secret="$(gen_hex 32)"
    master_key="$(gen_hex 32)"
    s3_ak="ag_$(gen_hex 10)"
    s3_sk="$(gen_hex 20)"
    admin_email="${ADMIN_EMAIL:-admin@${DOMAIN:-example.com}}"
    admin_pass="${ADMIN_PASSWORD:-$(gen_hex 16)}"
    if [[ -n "${DOMAIN}" ]]; then
      public_url="https://${DOMAIN}"
    else
      public_url="https://127.0.0.1"
    fi

    cat >"${ENV_FILE}" <<EOF
AGENTS_ENV=production
AGENTS_BIND_ADDR=127.0.0.1:8080
AGENTS_PUBLIC_URL=${public_url}
AGENTS_TRUSTED_ORIGINS=${public_url}
AGENTS_JWT_SECRET=${jwt_secret}
AGENTS_CREDENTIAL_MASTER_KEY=${master_key}
AGENTS_POSTGRES_PASSWORD=${pg_pass}
DATABASE_URL=postgres://agents:${pg_pass}@127.0.0.1:5432/agents?sslmode=disable
VALKEY_PASSWORD=${valkey_pass}
VALKEY_URL=redis://:${valkey_pass}@127.0.0.1:6379
NATS_URL=nats://127.0.0.1:4222
S3_ENDPOINT=http://127.0.0.1:9000
S3_BUCKET=agents-artifacts
S3_REGION=us-east-1
S3_ACCESS_KEY=${s3_ak}
S3_SECRET_KEY=${s3_sk}
AGENTS_LLM_PROVIDER=${INFERENCE_PROVIDER}
AGENTS_LLM_ENDPOINT=${INFERENCE_ENDPOINT}
AGENTS_LLM_MODEL=${INFERENCE_MODEL}
AGENTS_ADMIN_EMAIL=${admin_email}
AGENTS_ADMIN_INITIAL_PASSWORD=${admin_pass}
EOF
    chmod 600 "${ENV_FILE}"
  fi
}

DRY_ARG=()
if [[ "${DRY_RUN}" == "1" ]]; then
  DRY_ARG=(--dry-run)
fi

ensure_env_file

bash "${SCRIPT_DIR}/00-preflight.sh" --role "${ROLE}" "${DRY_ARG[@]}"
bash "${SCRIPT_DIR}/10-hardening.sh" --role "${ROLE}" "${DRY_ARG[@]}"

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "server" ]]; then
  SERVER_ARGS=(--role "${ROLE}" --acme-email "${ACME_EMAIL}" --vlan-cidr "${VLAN_CIDR}" --bind-ip "${BIND_IP}" --env-file "${ENV_FILE}")
  [[ -n "${DOMAIN}" ]] && SERVER_ARGS+=(--domain "${DOMAIN}")
  [[ -n "${PREVIEW_DOMAIN}" ]] && SERVER_ARGS+=(--preview-domain "${PREVIEW_DOMAIN}")
  bash "${SCRIPT_DIR}/20-deps-server.sh" "${SERVER_ARGS[@]}" "${DRY_ARG[@]}"
  bash "${SCRIPT_DIR}/25-setup-observability.sh" --enable-otel "${ENABLE_OTEL}" "${DRY_ARG[@]}"
fi

if [[ "${ROLE}" == "monolithic" || "${ROLE}" == "boxer" ]]; then
  bash "${SCRIPT_DIR}/30-deps-boxer.sh" "${DRY_ARG[@]}"
fi

bash "${SCRIPT_DIR}/40-systemd-units.sh" --role "${ROLE}" --env-file "${ENV_FILE}" --bin-dir "${BIN_DIR}" "${DRY_ARG[@]}"
bash "${SCRIPT_DIR}/90-doctor.sh" --role "${ROLE}" --env-file "${ENV_FILE}" "${DRY_ARG[@]}"

echo "[PASS] Zero-touch bootstrap finished (role=${ROLE}, https=always, default_inference=${INFERENCE_PROVIDER})"
