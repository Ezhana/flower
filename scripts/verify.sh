#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repository_root"
flower_cargo_bin="${FLOWER_CARGO_BIN:-cargo}"
flower_go_bin="${FLOWER_GO_BIN:-go}"

"$flower_cargo_bin" fmt --all -- --check
"$flower_cargo_bin" clippy --workspace --all-targets --all-features -- -D warnings
"$flower_cargo_bin" test --workspace --all-features
FLOWER_CARGO_BIN="$flower_cargo_bin" scripts/build-component.sh

flower_fixtures_regenerated="$(mktemp -d target/flower-fixtures-regenerated.XXXXXX)"
"$flower_cargo_bin" run --quiet -p flower-conformance --example generate_v0_2 -- \
  "$flower_fixtures_regenerated/attempt-lifecycle.json"
"$flower_cargo_bin" run --quiet -p flower-conformance --example generate_retry_v0_2 -- \
  "$flower_fixtures_regenerated/retry-lifecycle.json"
diff -ru "$flower_fixtures_regenerated" spec/v0.2/fixtures

flower_generated_bindings="sdk/go/internal/componentabi/flower"
flower_bindings_regenerated="$(mktemp -d target/flower-bindings-regenerated.XXXXXX)"
trap 'rm -rf "$flower_fixtures_regenerated" "$flower_bindings_regenerated"' EXIT
"$flower_go_bin" -C sdk/go/tools/wacogo-witgen run ./cmd/wacogo-witgen generate \
  -w flower:engine/engine-client \
  -o "../../../../$flower_bindings_regenerated" \
  -p flower.dev/sdk/go/internal/componentabi \
  ../../../../wits/engine.wit
diff -ru "$flower_bindings_regenerated/flower" "$flower_generated_bindings"

(cd sdk/go/tools/wacogo-witgen && "$flower_go_bin" test ./...)
(cd sdk/go && "$flower_go_bin" test ./...)
git diff --check
