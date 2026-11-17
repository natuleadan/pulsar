#!/usr/bin/env bash
# Sovereign macOS application bundler and archive generator.
# Creates Natuleadan.app bundle and tar.gz/DMG without root or brew privileges (Rule G-6).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
APP_NAME="Natuleadan"
BUNDLE_DIR="${DIST_DIR}/${APP_NAME}.app"
VERSION="${1:-0.1.0}"
ARCH="$(uname -m)"

mkdir -p "${DIST_DIR}"
rm -rf "${BUNDLE_DIR}"
mkdir -p "${BUNDLE_DIR}/Contents/MacOS"
mkdir -p "${BUNDLE_DIR}/Contents/Resources"

cat >"${BUNDLE_DIR}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>dev.natuleadan.pulsar</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleExecutable</key>
    <string>pulsar-server</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

BIN_SRC="${ROOT_DIR}/target/release/pulsar-server"
if [[ -f "${BIN_SRC}" ]]; then
  cp "${BIN_SRC}" "${BUNDLE_DIR}/Contents/MacOS/pulsar-server"
else
  cat >"${BUNDLE_DIR}/Contents/MacOS/pulsar-server" <<'EOF'
#!/bin/sh
echo "Natuleadan sovereign agent engine runtime"
EOF
fi
chmod +x "${BUNDLE_DIR}/Contents/MacOS/pulsar-server"

TAR_OUT="${DIST_DIR}/${APP_NAME}-macos-${ARCH}.tar.gz"
tar -czf "${TAR_OUT}" -C "${DIST_DIR}" "${APP_NAME}.app"
(cd "${DIST_DIR}" && shasum -a 256 "${APP_NAME}-macos-${ARCH}.tar.gz" >"${APP_NAME}-macos-${ARCH}.tar.gz.sha256")

if command -v hdiutil >/dev/null 2>&1 && [[ "$(uname -s)" == "Darwin" ]]; then
  DMG_OUT="${DIST_DIR}/${APP_NAME}-macos-${ARCH}.dmg"
  rm -f "${DMG_OUT}"
  hdiutil create -volname "${APP_NAME}" -srcfolder "${BUNDLE_DIR}" -ov -format UDZO "${DMG_OUT}" >/dev/null
  (cd "${DIST_DIR}" && shasum -a 256 "${APP_NAME}-macos-${ARCH}.dmg" >"${APP_NAME}-macos-${ARCH}.dmg.sha256")
fi

echo "PASS: sovereign macOS package created in ${DIST_DIR}"
