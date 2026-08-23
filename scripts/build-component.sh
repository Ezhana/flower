#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"
flower_cargo_bin="${FLOWER_CARGO_BIN:-cargo}"

"$flower_cargo_bin" build -p flower-component --target wasm32-unknown-unknown
mkdir -p target/components
"$flower_cargo_bin" run -p flower-component-tools -- new \
  target/wasm32-unknown-unknown/debug/flower_component.wasm \
  target/components/flower_engine.wasm
"$flower_cargo_bin" run -p flower-component-tools -- validate target/components/flower_engine.wasm

if [[ -n "${FLOWER_WASM_TOOLS_BIN:-}" ]]; then
  "$FLOWER_WASM_TOOLS_BIN" validate target/components/flower_engine.wasm
elif command -v wasm-tools >/dev/null 2>&1; then
  wasm-tools validate target/components/flower_engine.wasm
fi
