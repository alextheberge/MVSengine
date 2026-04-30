#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Portable CI gate (bash + cargo) for Linux, macOS, and Windows GitHub runners.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

CARGO="${CARGO:-cargo}"

echo "==> fmt --check"
"$CARGO" fmt -- --check

echo "==> check --all-targets"
"$CARGO" check --all-targets

echo "==> clippy -D warnings"
"$CARGO" clippy --all-targets --all-features -- -D warnings

echo "==> test --all-targets"
"$CARGO" test --all-targets

echo "==> fixture-smoke (generate + lint on fixture project)"
tmp_dir="$(mktemp -d)"
cleanup() {
	rm -rf "${tmp_dir}"
}
trap cleanup EXIT
cp -R tests/fixtures/generator_project "${tmp_dir}/project"
manifest_path="${tmp_dir}/mvs.json"
"$CARGO" run -- generate --root "${tmp_dir}/project" --manifest "${manifest_path}" --context cli --ai-schema "${tmp_dir}/project/tool_schema.json"
"$CARGO" run -- lint --root "${tmp_dir}/project" --manifest "${manifest_path}" --ai-schema "${tmp_dir}/project/tool_schema.json"

echo "==> lint-manifest (repo dogfood)"
"$CARGO" run -- lint --root . --manifest mvs.json

echo "==> dogfood-check"
EXPECTED_TAG="${EXPECTED_TAG:-}"
DOGFOOD_REQUIRE_CANONICAL="${DOGFOOD_REQUIRE_CANONICAL:-false}"
export EXPECTED_TAG DOGFOOD_REQUIRE_CANONICAL
scripts/release/check_dogfood.sh

echo "CI checks passed."
