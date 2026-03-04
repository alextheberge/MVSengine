#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="mvs-manager"
REPO="${MVS_REPO:-alextheberge/MVSengine}"
VERSION="${MVS_VERSION:-latest}"
INSTALL_DIR="${MVS_INSTALL_DIR:-$HOME/.local/bin}"

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi

if ! command -v tar >/dev/null 2>&1; then
  echo "tar is required" >&2
  exit 1
fi

detect_target() {
  local os
  local arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}" in
    Darwin)
      case "${arch}" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "unsupported macOS architecture: ${arch}" >&2; exit 1 ;;
      esac
      ;;
    Linux)
      case "${arch}" in
        x86_64) echo "x86_64-unknown-linux-gnu" ;;
        *)
          echo "unsupported Linux architecture for prebuilt releases: ${arch}" >&2
          echo "Build from source with: cargo build --release" >&2
          exit 1
          ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      echo "x86_64-pc-windows-msvc"
      ;;
    *)
      echo "unsupported OS: ${os}" >&2
      exit 1
      ;;
  esac
}

resolve_version() {
  if [[ "${VERSION}" != "latest" ]]; then
    echo "${VERSION}"
    return
  fi

  local latest
  latest="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  if [[ -z "${latest}" ]]; then
    echo "failed to resolve latest release tag from GitHub API" >&2
    exit 1
  fi
  echo "${latest}"
}

verify_archive() {
  local archive="$1"
  local checksums="$2"
  local archive_name
  archive_name="$(basename "${archive}")"

  local expected
  expected="$(grep -E "[[:space:]]${archive_name}$" "${checksums}" | awk '{print $1}')"
  if [[ -z "${expected}" ]]; then
    echo "missing checksum for ${archive_name}" >&2
    exit 1
  fi

  local actual
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${archive}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${archive}" | awk '{print $1}')"
  fi

  if [[ "${expected}" != "${actual}" ]]; then
    echo "checksum mismatch for ${archive_name}" >&2
    exit 1
  fi
}

target="$(detect_target)"
version_tag="$(resolve_version)"
version_label="${version_tag#v}"
archive_ext="tar.gz"
if [[ "${target}" == *"windows"* ]]; then
  archive_ext="zip"
fi

archive_name="${BIN_NAME}-${version_label}-${target}.${archive_ext}"
release_base="https://github.com/${REPO}/releases/download/${version_tag}"
archive_url="${release_base}/${archive_name}"
checksums_url="${release_base}/checksums.txt"

work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

archive_path="${work_dir}/${archive_name}"
checksums_path="${work_dir}/checksums.txt"

printf 'Downloading %s\n' "${archive_url}"
curl -fL "${archive_url}" -o "${archive_path}"
curl -fL "${checksums_url}" -o "${checksums_path}"

verify_archive "${archive_path}" "${checksums_path}"

mkdir -p "${INSTALL_DIR}"
if [[ "${archive_ext}" == "zip" ]]; then
  if ! command -v unzip >/dev/null 2>&1; then
    echo "unzip is required to install windows archive" >&2
    exit 1
  fi
  unzip -q "${archive_path}" -d "${work_dir}/extract"
else
  tar -xzf "${archive_path}" -C "${work_dir}/extract"
fi

binary_path="${work_dir}/extract/${BIN_NAME}"
if [[ "${target}" == *"windows"* ]]; then
  binary_path="${work_dir}/extract/${BIN_NAME}.exe"
fi

if [[ ! -f "${binary_path}" ]]; then
  echo "binary not found in archive: ${binary_path}" >&2
  exit 1
fi

cp "${binary_path}" "${INSTALL_DIR}/"
chmod +x "${INSTALL_DIR}/${BIN_NAME}" 2>/dev/null || true

printf 'Installed %s to %s\n' "${BIN_NAME}" "${INSTALL_DIR}"
printf 'Run `%s --help` to verify installation.\n' "${BIN_NAME}"
