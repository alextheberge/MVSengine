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
  echo "identity.mvs must be formatted as ARCH.FEAT.PROT.FIX-CONT, found: ${mvs_identity}" >&2
  exit 1
fi

arch="$(sed -n 's/.*"arch"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"
feat="$(sed -n 's/.*"feat"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"
fix="$(sed -n 's/.*"fix"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"

if [[ -z "${arch}" || -z "${feat}" ]]; then
  echo "unable to parse identity.arch/feat from ${manifest_file}" >&2
  exit 1
fi

if [[ -z "${fix}" ]]; then
  IFS='.' read -r _a _f _p _x <<< "${numeric_version}"
  dot_count="$(awk -F. '{print NF-1}' <<< "${numeric_version}")"
  if [[ "${dot_count}" -eq 3 && -n "${_x}" ]]; then
    fix="${_x}"
  elif [[ "${dot_count}" -eq 2 && -n "${_p}" ]]; then
    fix="${_p}"
  else
    echo "identity.mvs must be ARCH.FEAT.PROT.FIX-CONT (or legacy three-part), found: ${mvs_identity}" >&2
    exit 1
  fi
fi

# Package SemVer projection: ARCH.FEAT.FIX
semver_version="${arch}.${feat}.${fix}"

cargo_version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${cargo_file}" | head -n1)"
if [[ -z "${cargo_version}" ]]; then
  echo "unable to parse package version from ${cargo_file}" >&2
  exit 1
fi

cargo_numeric_version="${cargo_version%%-*}"
if [[ "${cargo_numeric_version}" != "${semver_version}" ]]; then
  echo "dogfood check failed: Cargo.toml version (${cargo_version}) does not match SemVer projection arch.feat.fix (${semver_version}) from MVS ${mvs_identity}." >&2
  echo "Run: make dogfood-sync-version" >&2
  exit 1
fi

canonical_tag="v${semver_version}"
release_tag="v${cargo_version}"

if [[ "${require_canonical}" == "true" && "${cargo_version}" != "${semver_version}" ]]; then
  echo "dogfood check failed: canonical release flow requires Cargo.toml version ${semver_version}, found ${cargo_version}." >&2
  exit 1
fi

if [[ -n "${expected_tag}" && "${expected_tag}" != "${release_tag}" ]]; then
  echo "dogfood check failed: expected release tag ${release_tag} from Cargo.toml, got ${expected_tag}." >&2
  exit 1
fi

echo "Dogfood check passed: Cargo ${cargo_version}, MVS ${mvs_identity}, SemVer ${semver_version}, release tag ${release_tag}, canonical tag ${canonical_tag}."
