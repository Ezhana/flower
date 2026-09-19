set shell := ["bash", "-cu"]

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

check:
    cargo check --workspace --all-targets

test:
    cargo test --workspace

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

build:
    cargo build --workspace

release:
    cargo build --workspace --release

build-ffi:
    cargo build -p my-lib-ffi

build-ffi-release:
    cargo build -p my-lib-ffi --release

clean:
    cargo clean
    rm -rf dist

ci: fmt-check check test clippy