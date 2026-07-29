#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

manifest_file="${1:-mvs.json}"
cargo_file="${2:-Cargo.toml}"
version_suffix="${CARGO_VERSION_SUFFIX:-}"

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
if [[ "${numeric_version}" == "${mvs_identity}" ]]; then
  echo "identity.mvs must include context suffix (-CONT), found: ${mvs_identity}" >&2
  exit 1
fi

# Prefer explicit identity.fix when present (v2); fall back to parsing.
arch="$(sed -n 's/.*"arch"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"
feat="$(sed -n 's/.*"feat"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"
fix="$(sed -n 's/.*"fix"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "${manifest_file}" | head -n1)"

if [[ -z "${arch}" || -z "${feat}" ]]; then
  echo "unable to parse identity.arch/feat from ${manifest_file}" >&2
  exit 1
fi

if [[ -z "${fix}" ]]; then
  # Legacy three-part A.F.P-CONT: SemVer patch was PROT.
  # Four-part A.F.P.X-CONT without a fix field: use 4th component.
  IFS='.' read -r _a _f _p _x <<< "${numeric_version}"
  dot_count="$(awk -F. '{print NF-1}' <<< "${numeric_version}")"
  if [[ "${dot_count}" -eq 3 && -n "${_x}" ]]; then
    fix="${_x}"
  elif [[ "${dot_count}" -eq 2 && -n "${_p}" ]]; then
    fix="${_p}"
  else
    echo "identity.mvs numeric part must be ARCH.FEAT.PROT.FIX (or legacy ARCH.FEAT.PROT), found: ${numeric_version}" >&2
    exit 1
  fi
fi

# Package SemVer projection: ARCH.FEAT.FIX
semver_version="${arch}.${feat}.${fix}"

version_suffix="${version_suffix#-}"
cargo_version="${semver_version}"
if [[ -n "${version_suffix}" ]]; then
  cargo_version="${semver_version}-${version_suffix}"
fi

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

awk -v new_version="${cargo_version}" '
BEGIN { in_package = 0; replaced = 0 }
/^\[package\]/ { in_package = 1; print; next }
in_package && /^\[/ { in_package = 0 }
in_package && !replaced && /^version[[:space:]]*=/ {
  print "version = \"" new_version "\""
  replaced = 1
  next
}
{ print }
END {
  if (!replaced) {
    exit 2
  }
}
' "${cargo_file}" > "${tmp_file}"

mv "${tmp_file}" "${cargo_file}"
echo "Updated ${cargo_file} version to ${cargo_version} from ${manifest_file} (SemVer arch.feat.fix)."
