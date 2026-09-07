#!/usr/bin/env bash
set -euo pipefail

readonly TOOLCHAIN="1.80.0"
readonly RUSTFMT_CONFIG="rustfmt.toml"
readonly LOCKFILE="Cargo.lock"

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to run Scout's Rust preflight." >&2
  exit 1
fi

if [[ ! -f "${RUSTFMT_CONFIG}" ]]; then
  echo "Missing ${RUSTFMT_CONFIG}; Scout requires an explicit Rust 1.80 formatting contract." >&2
  exit 1
fi

if [[ ! -f "${LOCKFILE}" ]]; then
  echo "Missing ${LOCKFILE}; Scout requires a committed dependency lock." >&2
  exit 1
fi

rustup toolchain install "${TOOLCHAIN}" \
  --profile minimal \
  --component rustfmt \
  --component clippy

cargo_180() {
  rustup run "${TOOLCHAIN}" cargo "$@"
}

echo "Scout Rust toolchain:"
rustup run "${TOOLCHAIN}" rustc --version

echo "Scout rustfmt:"
rustup run "${TOOLCHAIN}" rustfmt --version

echo "Checking exact Rust ${TOOLCHAIN} formatting..."
cargo_180 fmt --all -- --check

echo "Validating committed dependency lock..."
cargo_180 metadata --locked --no-deps >/dev/null

echo "Running Clippy against committed dependency lock..."
cargo_180 clippy --locked --workspace --all-targets -- -D warnings

echo "Running workspace tests against committed dependency lock..."
cargo_180 test --locked --workspace --all-targets

echo "Running read-only capability tripwire..."
if grep -RniE \
  --include='*.rs' \
  'Keypair|seed phrase|private.?key|send_transaction|sendTransaction|send_bundle|sendBundle|tpu.?client|jito.?searcher' \
  crates/
then
  echo "Forbidden execution/signing capability marker detected." >&2
  exit 1
fi

echo "Scout Rust ${TOOLCHAIN} preflight passed with committed dependency lock."
