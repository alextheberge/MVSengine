#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

manifest_file="${1:-mvs.json}"
cargo_file="${2:-Cargo.toml}"

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

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

awk -v new_version="${numeric_version}" '
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
echo "Updated ${cargo_file} version to ${numeric_version} from ${manifest_file}."
