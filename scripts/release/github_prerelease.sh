#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
set -euo pipefail

release_remote="${RELEASE_REMOTE:-origin}"
release_branch="${RELEASE_BRANCH:-$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)}"
allow_non_default="${RELEASE_ALLOW_NON_DEFAULT:-false}"
auto_commit="${RELEASE_AUTO_COMMIT:-true}"
do_push="${RELEASE_PUSH:-true}"
tag_suffix="${RELEASE_TAG_SUFFIX:-}"

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

if [[ -z "${tag_suffix}" ]]; then
  echo "Set RELEASE_TAG_SUFFIX=<rc suffix>, for example RELEASE_TAG_SUFFIX=rc1." >&2
  exit 1
fi
tag_suffix="${tag_suffix#-}"

if [[ -z "${release_branch}" || "${release_branch}" == "HEAD" ]]; then
  echo "unable to resolve release branch. Set RELEASE_BRANCH=<branch>." >&2
  exit 1
fi

if [[ "${allow_non_default}" != "true" ]] && [[ "${release_branch}" != "main" && "${release_branch}" != "master" ]]; then
  echo "prerelease branch must be main or master unless RELEASE_ALLOW_NON_DEFAULT=true (current: ${release_branch})." >&2
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

numeric_version="${mvs_identity%%-*}"
release_tag="v${numeric_version}-${tag_suffix}"
EXPECTED_TAG="${release_tag}" scripts/release/check_dogfood.sh

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
    echo "Commit these files, then run make release-rc again." >&2
    exit 1
  fi

  git commit -m "chore(release): prepare ${release_tag}" -- "${version_files[@]}"
  echo "Committed version files for ${release_tag}."
else
  echo "No version file changes to commit for ${release_tag}."
fi

if git rev-parse "${release_tag}" >/dev/null 2>&1; then
  echo "local tag already exists: ${release_tag}" >&2
  exit 1
fi

if git ls-remote --tags "${release_remote}" "refs/tags/${release_tag}" | grep -q .; then
  echo "remote tag already exists: ${release_tag}" >&2
  exit 1
fi

git tag -a "${release_tag}" -m "Release ${release_tag}"
echo "Created local tag ${release_tag}."

if [[ "${do_push}" == "true" ]]; then
  git push "${release_remote}" "${release_branch}"
  git push "${release_remote}" "refs/tags/${release_tag}"
  echo "Pushed ${release_branch} and ${release_tag} to ${release_remote}."
  echo "GitHub Actions will now run Release for ${release_tag}."
else
  echo "RELEASE_PUSH=false, skipping push."
  echo "Push ${release_branch} and ${release_tag} to ${release_remote} to trigger GitHub workflows."
fi
