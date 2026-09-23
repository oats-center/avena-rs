#!/usr/bin/env bash
# Builds the documentation site into target/docs-site.
#
# The guides are an mdBook under docs/. The API reference is generated from the
# code: rustdoc for rust-ljm and TypeDoc for the webapp library. Both are copied
# under api/ in the book output, which is what GitHub Pages publishes.
#
# Requires: mdbook, cargo, and pnpm with webapp dependencies installed.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SITE="${ROOT}/target/docs-site"

rm -rf "${SITE}"
mdbook build "${ROOT}/docs"

cargo doc \
  --manifest-path "${ROOT}/rust-ljm/Cargo.toml" \
  --target-dir "${ROOT}/target/docs-rust" \
  --no-deps \
  --document-private-items
mkdir -p "${SITE}/api"
cp -a "${ROOT}/target/docs-rust/doc" "${SITE}/api/rust"

(
  cd "${ROOT}/webapp"
  pnpm exec typedoc --options typedoc.json --out "${SITE}/api/webapp"
)

printf 'Docs site built at %s\n' "${SITE}"
