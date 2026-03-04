#!/usr/bin/env bash
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
  (
    cd "${staging_dir}"
    zip -rq "${OLDPWD}/${archive_path}" .
  )
else
  tar -C "${staging_dir}" -czf "${archive_path}" .
fi

if command -v sha256sum >/dev/null 2>&1; then
  checksum="$(sha256sum "${archive_path}" | awk '{print $1}')"
else
  checksum="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
fi

echo "${checksum}  ${archive_name}" > "${output_dir}/${archive_name}.sha256"

checksums_file="${output_dir}/checksums.txt"
if [[ -f "${checksums_file}" ]]; then
  grep -v "  ${archive_name}$" "${checksums_file}" > "${checksums_file}.tmp" || true
  mv "${checksums_file}.tmp" "${checksums_file}"
fi
echo "${checksum}  ${archive_name}" >> "${checksums_file}"

printf 'Created %s\n' "${archive_path}"
printf 'Wrote checksum %s\n' "${output_dir}/${archive_name}.sha256"
