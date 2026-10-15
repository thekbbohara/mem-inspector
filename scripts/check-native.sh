#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(pwd)/target}
for mem_inspector_manifest in Cargo.toml crates/mem-core/Cargo.toml crates/mem-ai/Cargo.toml; do
  cargo fmt --manifest-path "$mem_inspector_manifest" --all -- --check
  cargo test --manifest-path "$mem_inspector_manifest" --workspace --locked
  cargo clippy --manifest-path "$mem_inspector_manifest" --workspace --all-targets --locked -- -D warnings
  cargo build --manifest-path "$mem_inspector_manifest" --workspace --locked
 done
