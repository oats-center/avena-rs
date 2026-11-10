#!/usr/bin/env bash
# Builds the documentation site into target/docs-site, or into the folder given
# as the first argument.
#
# The guides are an mdBook under docs/. The API reference is generated from the
# code: rustdoc for rust-ljm and TypeDoc for the webapp library. Both are copied
# under api/ in the book output, which is what GitHub Pages publishes.
#
# Requires: mdbook, cargo, and pnpm with webapp dependencies installed.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SITE="${1:-${ROOT}/target/docs-site}"
mkdir -p "${SITE}"
SITE="$(cd -- "${SITE}" && pwd)"
case "${SITE}" in
  / | "${HOME:-/}" | "${ROOT}")
    echo "Refusing to use ${SITE} as the output folder; it is emptied first." >&2
    exit 1
    ;;
esac

rm -rf "${SITE}"
mdbook build --dest-dir "${SITE}" "${ROOT}/docs"

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
