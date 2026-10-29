#!/usr/bin/env bash
# Builds subbit-console's wasm bundle (ui::wasm, gated by target_arch
# -- not a Cargo feature, see lib.rs/ui.rs's own doc) into
# packages/console/www/pkg/. `--no-default-features` just drops the
# native-only `cli` feature.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

wasm-pack build packages/console --target web \
  --out-dir ../../packages/console/www/pkg --out-name subbit_console \
  -- --no-default-features
