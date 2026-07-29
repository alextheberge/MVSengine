#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Print package SemVer (ARCH.FEAT.FIX) derived from mvs.json identity.
set -euo pipefail

manifest_file="${1:-mvs.json}"

if [[ ! -f "${manifest_file}" ]]; then
  echo "manifest not found: ${manifest_file}" >&2
  exit 1
fi

mvs_identity="$(sed -n 's/.*"mvs"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${manifest_file}" | head -n1)"
if [[ -z "${mvs_identity}" ]]; then
  echo "unable to parse identity.mvs from ${manifest_file}" >&2
  exit 1
fi

numeric_version="${mvs_identity%%-*}"
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
    echo "identity.mvs must be ARCH.FEAT.PROT.FIX-CONT, found: ${mvs_identity}" >&2
    exit 1
  fi
fi

echo "${arch}.${feat}.${fix}"
