#!/usr/bin/env bash
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to run Scout's Rust formatter." >&2
  exit 1
fi

rustup show active-toolchain

echo "Scout Rust formatter toolchain:"
rustc --version

echo "Scout rustfmt:"
rustfmt --version

echo "Formatting Scout with the repository-pinned Rust toolchain..."
cargo fmt --all

echo "Verifying repository-pinned Rust formatting..."
cargo fmt --all -- --check

echo "Scout repository-pinned Rust formatting passed."
