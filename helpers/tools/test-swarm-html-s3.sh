#!/usr/bin/env bash
set -euo pipefail

# Wrapper to execute the multi-agent HTML synthesis and S3 publishing test.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_URL="${TARGET_URL:-https://test.nla.run}"

echo "=========================================================="
echo "Running Multi-Agent HTML & S3 Publishing Verification"
echo "Target URL: ${TARGET_URL}"
echo "=========================================================="

TARGET_URL="${TARGET_URL}" node "${SCRIPT_DIR}/test-swarm-html-s3.mjs"
