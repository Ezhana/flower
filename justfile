cargo := env_var_or_default("FLOWER_CARGO_BIN", "cargo")
go := env_var_or_default("FLOWER_GO_BIN", "go")
export GOCACHE := env_var_or_default("GOCACHE", justfile_directory() / "target/go-build-cache")
export GOPATH := env_var_or_default("GOPATH", justfile_directory() / "target/go")

default:
    @just --list

# Format Rust and Go sources.
format:
    {{cargo}} fmt --all
    {{go}} -C sdk/go fmt ./...
    {{go}} -C sdk/go/tools/wacogo-witgen fmt ./...

# Check Rust formatting without changing files.
format-check:
    {{cargo}} fmt --all -- --check

# Run Rust lints with warnings denied.
lint:
    {{cargo}} clippy --workspace --all-targets --all-features -- -D warnings

# Run every Rust test.
rust-test:
    {{cargo}} test --workspace --all-features

# Build and validate the WebAssembly Component.
component:
    {{cargo}} build -p flower-component --target wasm32-unknown-unknown
    {{cargo}} run -p flower-component-tools -- new target/wasm32-unknown-unknown/debug/flower_component.wasm target/components/flower_engine.wasm
    {{cargo}} run -p flower-component-tools -- validate target/components/flower_engine.wasm

# Regenerate shared engine regression fixtures.
fixtures:
    {{cargo}} run --quiet -p flower-engine-tests --example generate_attempt_lifecycle -- fixtures/cases/attempt-lifecycle.json
    {{cargo}} run --quiet -p flower-engine-tests --example generate_retry_lifecycle -- fixtures/cases/retry-lifecycle.json

# Regenerate checked-in Go Component bindings.
bindings:
    {{go}} -C sdk/go/tools/wacogo-witgen run ./cmd/wacogo-witgen generate -w flower:engine/engine-client -o ../../../../sdk/go/internal/componentabi -p flower.dev/sdk/go/internal/componentabi ../../../../wits/engine.wit

# Verify generated files are current without changing checked-in outputs.
generated-check:
    {{cargo}} run --quiet -p flower-engine-tests --example generate_attempt_lifecycle -- target/generated-fixtures/attempt-lifecycle.json
    {{cargo}} run --quiet -p flower-engine-tests --example generate_retry_lifecycle -- target/generated-fixtures/retry-lifecycle.json
    git diff --no-index --exit-code fixtures/cases target/generated-fixtures
    {{go}} -C sdk/go/tools/wacogo-witgen run ./cmd/wacogo-witgen generate -w flower:engine/engine-client -o ../../../../target/generated-bindings -p flower.dev/sdk/go/internal/componentabi ../../../../wits/engine.wit
    git diff --no-index --exit-code sdk/go/internal/componentabi/flower target/generated-bindings/flower

# Run the Go SDK and binding-generator tests against the built Component.
go-test: component
    {{go}} -C sdk/go/tools/wacogo-witgen test ./...
    {{go}} -C sdk/go test ./...

# Run the complete repository verification pipeline.
verify: format-check lint rust-test generated-check go-test
    git diff --check
