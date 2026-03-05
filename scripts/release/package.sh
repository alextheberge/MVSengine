#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

BIN_NAME="${BIN_NAME:-mvs-manager}"
TARGET="${TARGET:-}"
VERSION_TAG="${VERSION_TAG:-}"
DIST_ROOT="${DIST_ROOT:-dist}"
MANIFEST_FILE="${MANIFEST_FILE:-mvs.json}"

if [[ -z "${TARGET}" ]]; then
  TARGET="$(rustc -vV | awk '/host:/ {print $2}')"
fi

if [[ -z "${VERSION_TAG}" ]]; then
  pkg_version="$(cargo metadata --no-deps --format-version=1 | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -n1)"
  VERSION_TAG="v${pkg_version}"
fi

release_label="${VERSION_TAG#v}"
archive_ext="tar.gz"
binary_ext=""
if [[ "${TARGET}" == *"windows"* ]]; then
  archive_ext="zip"
  binary_ext=".exe"
fi

create_zip_archive() {
  local source_dir="$1"
  local destination="$2"

  if command -v zip >/dev/null 2>&1; then
    (
      cd "${source_dir}"
      zip -rq "${destination}" .
    )
    return
  fi

  if command -v 7z >/dev/null 2>&1; then
    (
      cd "${source_dir}"
      7z a -bd -tzip "${destination}" . >/dev/null
    )
    return
  fi

  local py_bin=""
  if command -v python3 >/dev/null 2>&1; then
    py_bin="python3"
  elif command -v python >/dev/null 2>&1; then
    py_bin="python"
  fi

  if [[ -n "${py_bin}" ]]; then
    "${py_bin}" - "${source_dir}" "${destination}" <<'PY'
import os
import pathlib
import sys
import zipfile

source = pathlib.Path(sys.argv[1]).resolve()
destination = pathlib.Path(sys.argv[2]).resolve()

with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as archive:
    for path in source.rglob("*"):
        if path.is_file():
            archive.write(path, path.relative_to(source))
PY
    return
  fi

  echo "unable to create zip archive: missing zip, 7z, and python" >&2
  exit 1
}

compute_sha256() {
  local artifact="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${artifact}" | awk '{print $1}'
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${artifact}" | awk '{print $1}'
    return
  fi

  if command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 "${artifact}" | awk '{print $2}'
    return
  fi

  local py_bin=""
  if command -v python3 >/dev/null 2>&1; then
    py_bin="python3"
  elif command -v python >/dev/null 2>&1; then
    py_bin="python"
  fi

  if [[ -n "${py_bin}" ]]; then
    "${py_bin}" - "${artifact}" <<'PY'
import hashlib
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
digest = hashlib.sha256(path.read_bytes()).hexdigest()
print(digest)
PY
    return
  fi

  echo "unable to compute sha256 checksum: missing sha256sum, shasum, openssl, and python" >&2
  exit 1
}

cargo build --release --target "${TARGET}"

binary_path="target/${TARGET}/release/${BIN_NAME}${binary_ext}"
if [[ ! -f "${binary_path}" ]]; then
  echo "expected binary not found: ${binary_path}" >&2
  exit 1
fi

output_dir="${DIST_ROOT}/${VERSION_TAG}"
mkdir -p "${output_dir}"

staging_dir="$(mktemp -d)"
trap 'rm -rf "${staging_dir}"' EXIT

cp "${binary_path}" "${staging_dir}/${BIN_NAME}${binary_ext}"
if [[ -f "${MANIFEST_FILE}" ]]; then
  cp "${MANIFEST_FILE}" "${staging_dir}/mvs.json"
fi
if [[ -f "README.md" ]]; then
  cp "README.md" "${staging_dir}/README.md"
fi
if [[ -d "docs" ]]; then
  cp -R "docs" "${staging_dir}/docs"
fi

archive_name="${BIN_NAME}-${release_label}-${TARGET}.${archive_ext}"
archive_path="${output_dir}/${archive_name}"

if [[ "${archive_ext}" == "zip" ]]; then
  create_zip_archive "${staging_dir}" "${PWD}/${archive_path}"
else
  tar -C "${staging_dir}" -czf "${archive_path}" .
fi

checksum="$(compute_sha256 "${archive_path}")"

echo "${checksum}  ${archive_name}" > "${output_dir}/${archive_name}.sha256"

checksums_file="${output_dir}/checksums.txt"
if [[ -f "${checksums_file}" ]]; then
  grep -v "  ${archive_name}$" "${checksums_file}" > "${checksums_file}.tmp" || true
  mv "${checksums_file}.tmp" "${checksums_file}"
fi
echo "${checksum}  ${archive_name}" >> "${checksums_file}"

printf 'Created %s\n' "${archive_path}"
printf 'Wrote checksum %s\n' "${output_dir}/${archive_name}.sha256"
