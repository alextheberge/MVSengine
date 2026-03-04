#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <version_dir> [output_file]" >&2
  exit 1
fi

version_dir="$1"
output_file="${2:-${version_dir}/checksums.txt}"

if [[ ! -d "${version_dir}" ]]; then
  echo "Directory not found: ${version_dir}" >&2
  exit 1
fi

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

found=0
while IFS= read -r -d '' checksum_file; do
  found=1
  cat "${checksum_file}" >> "${tmp_file}"
  printf '\n' >> "${tmp_file}"
done < <(find "${version_dir}" -type f -name '*.sha256' -print0 | sort -z)

if [[ "${found}" -eq 0 ]]; then
  echo "No .sha256 files found under ${version_dir}" >&2
  exit 1
fi

awk 'NF == 2 {print $1 "  " $2}' "${tmp_file}" | sort -u > "${output_file}"

printf 'Merged checksums into %s\n' "${output_file}"
