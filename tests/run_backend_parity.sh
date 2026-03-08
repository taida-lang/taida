#!/usr/bin/env bash
set -euo pipefail

if ! command -v node >/dev/null 2>&1; then
  echo "error: node is required for backend parity tests" >&2
  exit 1
fi

if ! command -v cc >/dev/null 2>&1; then
  echo "error: cc is required for native backend parity tests" >&2
  exit 1
fi

echo "[backend-parity] running native compile parity tests"
cargo test --test native_compile -- --nocapture

echo "[backend-parity] running crash regression corpus"
cargo test --test crash_regression -- --nocapture

echo "[backend-parity] running three-way parity tests"
cargo test --test parity -- --nocapture
