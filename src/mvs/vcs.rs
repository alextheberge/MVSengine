// SPDX-License-Identifier: AGPL-3.0-only
//! Minimal git plumbing for `migrate`: checking whether `git` is usable,
//! listing tags in chronological order, and checking out historical
//! snapshots into disposable worktrees so the crawler can be run against
//! them without disturbing the caller's working tree.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};

/// True when a `git` binary is on `PATH` and runs successfully.
pub fn is_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// True when `root` is inside a git working tree.
pub fn is_repo(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        })
        .unwrap_or(false)
}

/// Lists tags oldest-first, ordered by the creation date of what each tag
/// points at (falls back to the pointed-at commit's date for lightweight
/// tags).
pub fn list_tags_chronological(root: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "for-each-ref",
            "--sort=creatordate",
            "--format=%(refname:short)",
            "refs/tags",
        ])
        .output()
        .context("failed to run `git for-each-ref`")?;

    if !output.status.success() {
        bail!(
            "git for-each-ref failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// A disposable, detached `git worktree` checked out at a tag. Removed via
/// `git worktree remove` (and the directory deleted as a fallback) when
/// dropped, so a panicking or early-returning caller never leaks it.
pub struct Worktree {
    repo_root: PathBuf,
    pub path: PathBuf,
}

impl Worktree {
    pub fn create(repo_root: &Path, tag: &str) -> Result<Self> {
        let dir_name = format!(
            "mvs-backfill-{}-{}-{}",
            std::process::id(),
            sanitize_for_path(tag),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        let path = std::env::temp_dir().join(dir_name);

        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(["worktree", "add", "--detach", "--force"])
            .arg(&path)
            .arg(tag)
            .output()
            .with_context(|| format!("failed to run `git worktree add` for tag `{tag}`"))?;

        if !output.status.success() {
            bail!(
                "git worktree add failed for tag `{tag}`: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            path,
        })
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output();
        // Best-effort fallback in case `git worktree remove` didn't run
        // (e.g. the repo root itself vanished) or left files behind.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn sanitize_for_path(tag: &str) -> String {
    tag.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "mvs-vcs-test-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or_default()
            ));
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo_with_tags(dir: &Path) {
        run_git(dir, &["init", "-q", "-b", "main"]);
        for (tag, content) in [("v1.0.0", "one"), ("v1.1.0", "two"), ("v2.0.0", "three")] {
            fs::write(dir.join("marker.txt"), content).unwrap();
            run_git(dir, &["add", "-A"]);
            run_git(dir, &["commit", "-q", "-m", tag]);
            run_git(dir, &["tag", tag]);
        }
    }

    #[test]
    fn detects_repo_and_lists_tags_chronologically() {
        if !is_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let dir = TempDir::new("repo");
        init_repo_with_tags(dir.path());

        assert!(is_repo(dir.path()));
        let tags = list_tags_chronological(dir.path()).unwrap();
        assert_eq!(tags, vec!["v1.0.0", "v1.1.0", "v2.0.0"]);
    }

    #[test]
    fn non_repo_is_reported_as_such() {
        if !is_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let dir = TempDir::new("non-repo");
        assert!(!is_repo(dir.path()));
    }

    #[test]
    fn worktree_checks_out_tag_content_and_cleans_up_on_drop() {
        if !is_available() {
            eprintln!("skipping: git not available");
            return;
        }
        let dir = TempDir::new("worktree-repo");
        init_repo_with_tags(dir.path());

        let worktree_path = {
            let worktree = Worktree::create(dir.path(), "v1.1.0").unwrap();
            let content = fs::read_to_string(worktree.path.join("marker.txt")).unwrap();
            assert_eq!(content, "two");
            worktree.path.clone()
        };
        // Dropped: the worktree directory should be gone.
        assert!(!worktree_path.exists());
    }
}
