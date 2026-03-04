#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 4 ]]; then
  echo "Usage: $0 <archive_path> <checksums_file> [signature_file] [gpg_public_key]" >&2
  exit 1
fi

archive_path="$1"
checksums_file="$2"
signature_file="${3:-}"
public_key_file="${4:-}"
archive_name="$(basename "${archive_path}")"

if [[ ! -f "${archive_path}" ]]; then
  echo "Archive not found: ${archive_path}" >&2
  exit 1
fi

if [[ ! -f "${checksums_file}" ]]; then
  echo "Checksums file not found: ${checksums_file}" >&2
  exit 1
fi

expected_line="$(grep -E "[[:space:]]${archive_name}$" "${checksums_file}" || true)"
if [[ -z "${expected_line}" ]]; then
  echo "No checksum entry found for ${archive_name}" >&2
  exit 1
fi

expected_checksum="$(echo "${expected_line}" | awk '{print $1}')"

if command -v sha256sum >/dev/null 2>&1; then
  actual_checksum="$(sha256sum "${archive_path}" | awk '{print $1}')"
else
  actual_checksum="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
fi

if [[ "${expected_checksum}" != "${actual_checksum}" ]]; then
  echo "Checksum mismatch for ${archive_name}" >&2
  echo "Expected: ${expected_checksum}" >&2
  echo "Actual:   ${actual_checksum}" >&2
  exit 1
fi

printf 'Checksum verified for %s\n' "${archive_name}"

if [[ -n "${signature_file}" || -n "${public_key_file}" ]]; then
  if [[ -z "${signature_file}" || -z "${public_key_file}" ]]; then
    echo "Provide both signature_file and gpg_public_key to verify signatures" >&2
    exit 1
  fi

  if ! command -v gpg >/dev/null 2>&1; then
    echo "gpg is required for signature verification" >&2
    exit 1
  fi

  gpg --batch --import "${public_key_file}" >/dev/null 2>&1
  gpg --batch --verify "${signature_file}" "${checksums_file}" >/dev/null 2>&1
  echo "GPG signature verified for checksums file"
fi
