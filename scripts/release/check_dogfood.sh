#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

manifest_file="${1:-mvs.json}"
cargo_file="${2:-Cargo.toml}"
expected_tag="${EXPECTED_TAG:-}"
require_canonical="${DOGFOOD_REQUIRE_CANONICAL:-false}"

if [[ ! -f "${manifest_file}" ]]; then
  echo "manifest not found: ${manifest_file}" >&2
  exit 1
fi

if [[ ! -f "${cargo_file}" ]]; then
  echo "Cargo manifest not found: ${cargo_file}" >&2
  exit 1
fi

mvs_identity="$(sed -n 's/.*"mvs"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${manifest_file}" | head -n1)"
if [[ -z "${mvs_identity}" ]]; then
  echo "unable to parse identity.mvs from ${manifest_file}" >&2
  exit 1
fi

numeric_version="${mvs_identity%%-*}"
context_suffix="${mvs_identity#*-}"
if [[ "${numeric_version}" == "${mvs_identity}" || -z "${context_suffix}" ]]; then
  echo "identity.mvs must be formatted as ARCH.FEAT.PROT-CONT, found: ${mvs_identity}" >&2
  exit 1
fi

cargo_version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${cargo_file}" | head -n1)"
if [[ -z "${cargo_version}" ]]; then
  echo "unable to parse package version from ${cargo_file}" >&2
  exit 1
fi

cargo_numeric_version="${cargo_version%%-*}"
if [[ "${cargo_numeric_version}" != "${numeric_version}" ]]; then
  echo "dogfood check failed: Cargo.toml version (${cargo_version}) does not share the MVS numeric version (${numeric_version})." >&2
  echo "Run: make dogfood-sync-version" >&2
  exit 1
fi

canonical_tag="v${numeric_version}"
release_tag="v${cargo_version}"

if [[ "${require_canonical}" == "true" && "${cargo_version}" != "${numeric_version}" ]]; then
  echo "dogfood check failed: canonical release flow requires Cargo.toml version ${numeric_version}, found ${cargo_version}." >&2
  exit 1
fi

if [[ -n "${expected_tag}" && "${expected_tag}" != "${release_tag}" ]]; then
  echo "dogfood check failed: expected release tag ${release_tag} from Cargo.toml, got ${expected_tag}." >&2
  exit 1
fi

echo "Dogfood check passed: Cargo ${cargo_version}, MVS ${mvs_identity}, release tag ${release_tag}, canonical tag ${canonical_tag}."
