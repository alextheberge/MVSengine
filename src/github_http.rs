// SPDX-License-Identifier: AGPL-3.0-only
//! Shared HTTPS client for GitHub API and release asset downloads (ureq + rustls).

use std::io::{Read, Write};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use ureq::Agent;

use crate::update::{current_version, github_token};

/// Cap for small artifacts (checksums, detached signatures).
pub(crate) const MAX_SMALL_DOWNLOAD_BYTES: u64 = 1024 * 1024;

/// Cap for release archives (binary tarballs / zips).
pub(crate) const MAX_ARCHIVE_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const OVERALL_TIMEOUT: Duration = Duration::from_secs(300);
const READ_TIMEOUT: Duration = Duration::from_secs(120);

static GITHUB_AGENT: OnceLock<Agent> = OnceLock::new();

fn build_agent() -> Agent {
    ureq::AgentBuilder::new()
        .user_agent(&format!("mvs-manager/{}", current_version()))
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(OVERALL_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .try_proxy_from_env(true)
        .build()
}

/// Shared agent for GitHub `api.github.com` and `github.com` release URLs.
pub(crate) fn github_agent() -> &'static Agent {
    GITHUB_AGENT.get_or_init(build_agent)
}

fn apply_github_auth(mut request: ureq::Request) -> ureq::Request {
    if let Some(token) = github_token() {
        let value = format!("Bearer {token}");
        request = request.set("Authorization", value.as_str());
    }
    request
}

/// `GET` JSON from a GitHub API URL (no curl/powershell).
pub(crate) fn fetch_github_json(url: &str) -> Result<Value> {
    let request = apply_github_auth(github_agent().get(url));
    let response = request
        .set("Accept", "application/vnd.github+json")
        .call()
        .with_context(|| format!("HTTP GET failed for {url}"))?;
    let status = response.status();
    if !(200..300).contains(&status) {
        bail!(
            "GitHub API returned HTTP {status} for {url}: {}",
            response.status_text()
        );
    }
    let body = response
        .into_string()
        .with_context(|| format!("failed to read response body from {url}"))?;
    serde_json::from_str(&body).with_context(|| format!("invalid JSON from {url}"))
}

/// Download full body when small (e.g. optional signature file). Returns `None` on HTTP 404.
pub(crate) fn get_github_bytes_optional(url: &str) -> Result<Option<Vec<u8>>> {
    let request = apply_github_auth(github_agent().get(url));
    let response = match request.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("HTTP GET failed for {url}")),
    };
    match response.status() {
        404 => Ok(None),
        200..300 => {
            let mut body = Vec::new();
            response
                .into_reader()
                .take(MAX_SMALL_DOWNLOAD_BYTES)
                .read_to_end(&mut body)
                .context("read response body")?;
            Ok(Some(body))
        }
        status => bail!(
            "unexpected HTTP {status} for {url}: {}",
            response.status_text()
        ),
    }
}

/// Stream response body to `dest_path`, hashing with SHA-256, enforcing a byte cap.
pub(crate) fn download_to_file_sha256_limited(
    url: &str,
    dest_path: &std::path::Path,
    max_bytes: u64,
) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::fs::File;

    let request = apply_github_auth(github_agent().get(url));
    let response = request
        .call()
        .with_context(|| format!("HTTP GET failed for {url}"))?;
    if !(200..300).contains(&response.status()) {
        bail!(
            "HTTP {} for {url}: {}",
            response.status(),
            response.status_text()
        );
    }

    let mut reader = response.into_reader();
    let mut file = File::create(dest_path)
        .with_context(|| format!("failed to create `{}`", dest_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        let n = reader
            .read(&mut buf)
            .context("failed reading download stream")?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_bytes {
            bail!(
                "download exceeded maximum size ({max_bytes} bytes); refusing to continue for {url}"
            );
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .with_context(|| format!("failed writing `{}`", dest_path.display()))?;
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[test]
    fn fetch_github_json_parses_release() {
        let mut server = Server::new();
        let path = "/repos/o/r/releases/latest";
        let _m = server
            .mock("GET", path)
            .match_header("accept", "application/vnd.github+json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"tag_name":"v1.2.3","html_url":"https://example/release"}"#)
            .create();

        let url = format!("{}{path}", server.url());
        let v = fetch_github_json(&url).expect("json");
        assert_eq!(v["tag_name"], "v1.2.3");
    }

    #[test]
    fn fetch_github_json_errors_on_non_2xx() {
        let mut server = Server::new();
        let path = "/repos/x/y/releases/latest";
        let _m = server
            .mock("GET", path)
            .with_status(403)
            .with_body("rate limited")
            .create();

        let url = format!("{}{path}", server.url());
        assert!(fetch_github_json(&url).is_err());
    }

    #[test]
    fn get_github_bytes_optional_returns_none_on_404() {
        let mut server = Server::new();
        let path = "/missing.minisig";
        let _m = server.mock("GET", path).with_status(404).create();
        let url = format!("{}{path}", server.url());
        assert_eq!(get_github_bytes_optional(&url).unwrap(), None);
    }
}
