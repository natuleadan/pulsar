#!/usr/bin/env bash
set -euo pipefail

# Provision PostgreSQL (SCRAM-SHA-256), Valkey/Redis, NATS JetStream, and Traefik TLS proxy.
ROLE="monolithic"
DOMAIN="agents.example.com"
PREVIEW_DOMAIN="preview.example.com"
ACME_EMAIL="ops@example.com"
VLAN_CIDR="10.0.0.0/8"
BIND_IP="127.0.0.1"
ENV_FILE="/etc/agents/agents.env"
DRY_RUN="${AGENTS_DRY_RUN:-0}"

usage() {
  cat <<'EOF'
Usage: 20-deps-server.sh [--role monolithic|server] [--domain host] [--preview-domain host] [--acme-email email] [--vlan-cidr cidr] [--bind-ip ip] [--env-file path] [--dry-run]
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
  monolithic | server) ;;
  *)
    echo "[FAIL] 20-deps-server.sh supports only monolithic or server, got: ${ROLE}" >&2
    exit 2
    ;;
esac

if [[ "${DRY_RUN}" != "1" && "$(id -u)" -ne 0 ]]; then
  echo "[FAIL] 20-deps-server.sh must run as root (use --dry-run to inspect)" >&2
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

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "[DRY-RUN] install postgresql, valkey/redis-server, nats-server, traefik"
else
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -y
  apt-get install -y postgresql postgresql-contrib curl ca-certificates gnupg
  if apt-cache show valkey >/dev/null 2>&1; then
    apt-get install -y valkey
  else
    apt-get install -y redis-server
  fi
fi

# Configure PostgreSQL SCRAM-SHA-256 without logging database credentials.
PG_PASS="${AGENTS_POSTGRES_PASSWORD:-}"
if [[ "${DRY_RUN}" != "1" ]]; then
  if [[ -z "${PG_PASS}" && -r "${ENV_FILE}" ]]; then
    PG_PASS="$(awk -F= '/^AGENTS_POSTGRES_PASSWORD=/{print substr($0, index($0,$2)); exit}' "${ENV_FILE}")"
  fi
  if [[ -z "${PG_PASS}" ]]; then
    echo "[FAIL] AGENTS_POSTGRES_PASSWORD is required in environment or ${ENV_FILE}" >&2
    exit 1
  fi
  su - postgres -c "psql -v ON_ERROR_STOP=1 -c \"DO \\\$\\\$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'agents') THEN CREATE ROLE agents WITH LOGIN PASSWORD '${PG_PASS}'; ELSE ALTER ROLE agents WITH PASSWORD '${PG_PASS}'; END IF; END \\\$\\\$;\""
  su - postgres -c "psql -tc \"SELECT 1 FROM pg_database WHERE datname = 'agents'\" | grep -q 1 || createdb -O agents agents"
  unset PG_PASS
else
  echo "[DRY-RUN] ensure PostgreSQL scram-sha-256 role 'agents' and database 'agents'"
fi

write_file /etc/nats/nats-server.conf <<EOF
server_name: agents-nats-${ROLE}
listen: ${BIND_IP}:4222
http: 127.0.0.1:8222

jetstream {
  store_dir: /var/lib/nats/jetstream
  max_mem_store: 512MB
  max_file_store: 8GB
}
EOF

IS_BARE_IP=0
if [[ -z "${DOMAIN}" || "${DOMAIN}" == "localhost" || "${DOMAIN}" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ || "${DOMAIN}" == *:* ]]; then
  IS_BARE_IP=1
fi

if [[ "${IS_BARE_IP}" == "1" ]]; then
  write_file /etc/traefik/traefik.yml <<EOF
api:
  dashboard: false

entryPoints:
  web:
    address: ":80"
  websecure:
    address: ":443"

providers:
  file:
    directory: /etc/traefik/dynamic
    watch: true
  http:
    endpoint: "http://127.0.0.1:8080/internal/traefik/dynamic"
    pollInterval: "2s"
EOF

  write_file /etc/traefik/dynamic/agents-core.yml <<'EOF'
http:
  middlewares:
    agents-security-headers:
      headers:
        contentTypeNosniff: true
        frameDeny: true
  routers:
    agents-api-http:
      rule: "PathPrefix(`/`)"
      priority: 1
      entryPoints:
        - web
      service: agents-server
      middlewares:
        - agents-security-headers
  services:
    agents-server:
      loadBalancer:
        servers:
          - url: "http://127.0.0.1:8080"
EOF
else
  write_file /etc/traefik/traefik.yml <<EOF
api:
  dashboard: false

entryPoints:
  web:
    address: ":80"
    http:
      redirections:
        entryPoint:
          to: websecure
          scheme: https
  websecure:
    address: ":443"

certificatesResolvers:
  letsencrypt:
    acme:
      email: "${ACME_EMAIL}"
      storage: /var/lib/traefik/acme.json
      tlsChallenge: {}

providers:
  file:
    directory: /etc/traefik/dynamic
    watch: true
  http:
    endpoint: "http://127.0.0.1:8080/internal/traefik/dynamic"
    pollInterval: "2s"
EOF

  write_file /etc/traefik/dynamic/agents-core.yml <<EOF
http:
  middlewares:
    agents-security-headers:
      headers:
        stsSeconds: 31536000
        stsIncludeSubdomains: true
        contentTypeNosniff: true
        frameDeny: true
  routers:
    agents-api:
      rule: "Host(\`${DOMAIN}\`)"
      entryPoints:
        - websecure
      service: agents-server
      middlewares:
        - agents-security-headers
      tls:
        certResolver: letsencrypt
  services:
    agents-server:
      loadBalancer:
        servers:
          - url: "http://127.0.0.1:8080"
EOF
fi

echo "[PASS] Server dependencies configured (domain=${DOMAIN}, bare_ip=${IS_BARE_IP}, preview_domain=${PREVIEW_DOMAIN}, vlan=${VLAN_CIDR})"

if [[ "${DRY_RUN}" != "1" ]]; then
  ARCH="$(uname -m)"
  case "${ARCH}" in
    x86_64|amd64) GO_ARCH="amd64" ;;
    aarch64|arm64) GO_ARCH="arm64" ;;
    *) GO_ARCH="amd64" ;;
  esac

  if ! command -v nats-server >/dev/null 2>&1; then
    NATS_VER="v2.11.1"
    curl -fsSL "https://github.com/nats-io/nats-server/releases/download/${NATS_VER}/nats-server-${NATS_VER}-linux-${GO_ARCH}.tar.gz" -o /tmp/nats-server.tgz
    tar -xzf /tmp/nats-server.tgz -C /tmp
    install -m 0755 "/tmp/nats-server-${NATS_VER}-linux-${GO_ARCH}/nats-server" /usr/local/bin/nats-server
    rm -rf /tmp/nats-server.tgz "/tmp/nats-server-${NATS_VER}-linux-${GO_ARCH}"
  fi

  if ! command -v traefik >/dev/null 2>&1; then
    TRAEFIK_VER="v3.3.4"
    curl -fsSL "https://github.com/traefik/traefik/releases/download/${TRAEFIK_VER}/traefik_${TRAEFIK_VER}_linux_${GO_ARCH}.tar.gz" -o /tmp/traefik.tgz
    tar -xzf /tmp/traefik.tgz -C /tmp traefik
    install -m 0755 /tmp/traefik /usr/local/bin/traefik
    rm -f /tmp/traefik.tgz /tmp/traefik
  fi

  if [[ -r "${ENV_FILE}" ]]; then
    VALKEY_URL_VAL="$(awk -F= '/^VALKEY_URL=/{print substr($0, index($0,$2)); exit}' "${ENV_FILE}")"
    if [[ "${VALKEY_URL_VAL}" =~ ://[^:]*:([^@]+)@ ]]; then
      VALKEY_PASS="${BASH_REMATCH[1]}"
      for cfg in /etc/valkey/valkey.conf /etc/redis/redis.conf; do
        if [[ -f "${cfg}" ]]; then
          sed -i '/^requirepass /d' "${cfg}"
          echo "requirepass ${VALKEY_PASS}" >>"${cfg}"
        fi
      done
    fi
  fi

  if command -v docker >/dev/null 2>&1; then
    mkdir -p /var/lib/rustfs
    S3_ACCESS="${AGENTS_S3_ACCESS_KEY:-rustfsadmin}"
    S3_SECRET="${AGENTS_S3_SECRET_KEY:-rustfsadmin}"
    if [[ -r "${ENV_FILE}" ]]; then
      ENV_ACCESS="$(awk -F= '/^S3_ACCESS_KEY=/{print substr($0, index($0,$2)); exit}' "${ENV_FILE}")"
      ENV_SECRET="$(awk -F= '/^S3_SECRET_KEY=/{print substr($0, index($0,$2)); exit}' "${ENV_FILE}")"
      [[ -n "${ENV_ACCESS}" ]] && S3_ACCESS="${ENV_ACCESS}"
      [[ -n "${ENV_SECRET}" ]] && S3_SECRET="${ENV_SECRET}"
    fi
    docker rm -f agents-rustfs >/dev/null 2>&1 || true
    docker run -d --name agents-rustfs --restart unless-stopped \
      -p 127.0.0.1:9000:9000 -p 127.0.0.1:9001:9001 \
      -e "RUSTFS_ACCESS_KEY=${S3_ACCESS}" \
      -e "RUSTFS_SECRET_KEY=${S3_SECRET}" \
      -v /var/lib/rustfs:/data \
      rustfs/rustfs:latest >/dev/null
    echo "[INFO] Started agents-rustfs S3 service on 127.0.0.1:9000"
  fi
fi
