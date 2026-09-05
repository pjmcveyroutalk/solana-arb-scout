#!/usr/bin/env bash
set -euo pipefail

readonly TOOLCHAIN="1.80.0"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to run Scout's Rust formatter." >&2
  exit 1
fi

rustup toolchain install "${TOOLCHAIN}" \
  --profile minimal \
  --component rustfmt

cargo_180() {
  rustup run "${TOOLCHAIN}" cargo "$@"
}

echo "Scout Rust formatter toolchain:"
rustup run "${TOOLCHAIN}" rustc --version

echo "Scout rustfmt:"
rustup run "${TOOLCHAIN}" rustfmt --version

echo "Formatting Scout with exact Rust ${TOOLCHAIN} rustfmt..."
cargo_180 fmt --all

echo "Verifying exact Rust ${TOOLCHAIN} formatting..."
cargo_180 fmt --all -- --check

echo "Scout Rust ${TOOLCHAIN} formatting passed."
