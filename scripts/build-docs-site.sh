#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
SITE_DIR="${ROOT}/target/docs-site"
RUST_TARGET="${ROOT}/target/docs-rust"
FRONTEND_DOCS="${ROOT}/target/docs-frontend"

rm -rf "${SITE_DIR}" "${FRONTEND_DOCS}"
mkdir -p "${SITE_DIR}/api"

(
  cd "${ROOT}/rust-ljm"
  cargo doc \
    --target-dir "${RUST_TARGET}" \
    --no-deps \
    --document-private-items
)

cp -a "${RUST_TARGET}/doc" "${SITE_DIR}/api/rust"

(
  cd "${ROOT}/webapp"
  pnpm exec typedoc --options typedoc.json --out "${FRONTEND_DOCS}"
)

cp -a "${FRONTEND_DOCS}" "${SITE_DIR}/api/frontend"

python3 "${ROOT}/scripts/build-docs-site.py" --root "${ROOT}" --site "${SITE_DIR}"

printf 'Docs site built at %s\n' "${SITE_DIR}"
