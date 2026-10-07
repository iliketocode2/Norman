#!/usr/bin/env bash
# Build the documentation site's interpreter and check every example on it.
#
#   ./scripts/build-docs.sh
#
# Compiles the crate to WebAssembly, copies it into docs/, then runs every
# code block on the site through it. A broken example fails here rather than
# in front of a reader.
#
# The .wasm is committed, because GitHub Pages serves docs/ straight from the
# branch with no build step. Re-run this whenever src/ changes.
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

echo "building the interpreter for wasm…"
cargo build --release --target wasm32-unknown-unknown --lib

cp target/wasm32-unknown-unknown/release/munorman.wasm docs/munorman.wasm
printf 'interpreter: %s bytes\n' "$(wc -c < docs/munorman.wasm | tr -d ' ')"

echo "checking every example on the site…"
node docs/verify.mjs
