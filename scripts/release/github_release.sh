#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

release_remote="${RELEASE_REMOTE:-origin}"
release_branch="${RELEASE_BRANCH:-$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)}"
allow_non_default="${RELEASE_ALLOW_NON_DEFAULT:-false}"
auto_commit="${RELEASE_AUTO_COMMIT:-true}"
do_push="${RELEASE_PUSH:-true}"

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

if [[ -z "${release_branch}" || "${release_branch}" == "HEAD" ]]; then
  echo "unable to resolve release branch. Set RELEASE_BRANCH=<main|master>." >&2
  exit 1
fi

if [[ "${allow_non_default}" != "true" ]] && [[ "${release_branch}" != "main" && "${release_branch}" != "master" ]]; then
  echo "release branch must be main or master (current: ${release_branch})." >&2
  echo "Set RELEASE_ALLOW_NON_DEFAULT=true to override." >&2
  exit 1
fi

if ! git remote get-url "${release_remote}" >/dev/null 2>&1; then
  echo "git remote not found: ${release_remote}" >&2
  exit 1
fi

mvs_identity="$(sed -n 's/.*"mvs"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' mvs.json | head -n1)"
if [[ -z "${mvs_identity}" ]]; then
  echo "unable to parse identity.mvs from mvs.json" >&2
  exit 1
fi

canonical_tag="v${mvs_identity%%-*}"
EXPECTED_TAG="${canonical_tag}" scripts/release/check_dogfood.sh

version_files=()
for file in mvs.json Cargo.toml Cargo.lock; do
  if [[ -f "${file}" ]]; then
    version_files+=("${file}")
  fi
done

git add "${version_files[@]}"

if ! git diff --cached --quiet -- "${version_files[@]}"; then
  if [[ "${auto_commit}" != "true" ]]; then
    echo "version files changed but RELEASE_AUTO_COMMIT is false." >&2
    echo "Commit these files, then run make release-github again." >&2
    exit 1
  fi

  git commit -m "chore(release): prepare ${canonical_tag}" -- "${version_files[@]}"
  echo "Committed version files for ${canonical_tag}."
else
  echo "No version file changes to commit for ${canonical_tag}."
fi

if [[ "${do_push}" == "true" ]]; then
  git push "${release_remote}" "${release_branch}"
  echo "Pushed ${release_branch} to ${release_remote}."
  echo "GitHub Actions will now run Auto Tag Version and Release for ${canonical_tag}."
else
  echo "RELEASE_PUSH=false, skipping push."
  echo "Push ${release_branch} to ${release_remote} to trigger GitHub workflows."
fi
