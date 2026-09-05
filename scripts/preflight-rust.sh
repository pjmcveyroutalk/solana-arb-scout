#!/usr/bin/env bash
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to run Scout's Rust preflight." >&2
  exit 1
fi

rustup show active-toolchain

echo "Scout Rust preflight toolchain:"
rustc --version

echo "Scout rustfmt:"
rustfmt --version

echo "Scout Clippy:"
cargo clippy --version

echo "Resolving repository-pinned Rust dependencies..."
cargo generate-lockfile
cargo update -p tinyvec --precise 1.12.0

echo "Checking repository-pinned Rust formatting..."
cargo fmt --all -- --check

echo "Running Clippy..."
cargo clippy --locked --workspace --all-targets -- -D warnings

echo "Running workspace tests..."
cargo test --locked --workspace --all-targets

echo "Running read-only capability tripwire..."
if grep -RniE \
  --include='*.rs' \
  'Keypair|seed phrase|private.?key|send_transaction|sendTransaction|send_bundle|sendBundle|tpu.?client|jito.?searcher' \
  crates/
then
  echo "Forbidden execution/signing capability marker detected." >&2
  exit 1
fi

echo "Scout repository-pinned Rust preflight passed."
