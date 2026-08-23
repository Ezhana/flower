#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"

cargo build -p flower-component --target wasm32-unknown-unknown
mkdir -p target/components
wasm-tools component new \
  target/wasm32-unknown-unknown/debug/flower_component.wasm \
  -o target/components/flower_engine.wasm
