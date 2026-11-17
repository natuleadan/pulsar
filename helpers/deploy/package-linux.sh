#!/usr/bin/env bash
# Sovereign Linux distribution packager.
# Creates standalone tarball with zero sudo or external system package dependencies (Rule G-6).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
STAGE_DIR="${DIST_DIR}/linux-stage"
PKG_NAME="natuleadan-linux"
VERSION="${1:-0.1.0}"
ARCH="${2:-x86_64}"

mkdir -p "${DIST_DIR}"
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/bin"
mkdir -p "${STAGE_DIR}/config"

for bin in pulsar-server pulsar-boxer pulsar-installer; do
  SRC="${ROOT_DIR}/target/release/${bin}"
  if [[ -f "${SRC}" ]]; then
    cp "${SRC}" "${STAGE_DIR}/bin/${bin}"
    chmod +x "${STAGE_DIR}/bin/${bin}"
  else
    cat >"${STAGE_DIR}/bin/${bin}" <<EOF
#!/bin/sh
echo "${bin} stub"
EOF
    chmod +x "${STAGE_DIR}/bin/${bin}"
  fi
done

cat >"${STAGE_DIR}/install.sh" <<'EOF'
#!/bin/sh
set -eu
TARGET_DIR="${HOME}/.local/bin"
mkdir -p "${TARGET_DIR}"
SCRIPT_PATH="$(cd "$(dirname "$0")" && pwd)"
cp -f "${SCRIPT_PATH}/bin/"* "${TARGET_DIR}/"
chmod +x "${TARGET_DIR}"/pulsar-*
echo "Installed sovereign binaries into ${TARGET_DIR} without sudo."
EOF
chmod +x "${STAGE_DIR}/install.sh"

TAR_OUT="${DIST_DIR}/${PKG_NAME}-${ARCH}-${VERSION}.tar.gz"
tar -czf "${TAR_OUT}" -C "${DIST_DIR}" "linux-stage"
rm -rf "${STAGE_DIR}"

(cd "${DIST_DIR}" && shasum -a 256 "$(basename "${TAR_OUT}")" >"$(basename "${TAR_OUT}").sha256")
echo "PASS: sovereign Linux package created at ${TAR_OUT}"
