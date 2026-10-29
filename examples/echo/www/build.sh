#!/usr/bin/env bash
# Builds the browser demo's wasm bundle from packages/stamp's `wasm`
# feature (vanilla dialect only) into examples/echo/www/pkg/.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

wasm-pack build packages/stamp --target web \
  --out-dir ../../examples/echo/www/pkg --out-name subbit_stamp \
  -- --no-default-features --features wasm
