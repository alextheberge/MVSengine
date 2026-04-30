// SPDX-License-Identifier: AGPL-3.0-only
//! Download a GitHub release archive, verify SHA-256 against `checksums.txt`, extract, and install
//! the `mvs-manager` binary without executing a remote shell script.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::update::{github_token, parse_repo_slug, repo_slug};

const BIN_NAME: &str = "mvs-manager";

/// When `1`/`true`/`yes`/`on`, `self-update` uses the legacy `curl | bash` / `irm | iex` installer instead of in-process verification.
pub const LEGACY_SHELL_INSTALL_ENV: &str = "MVS_LEGACY_SHELL_INSTALL";

/// Default in-process path; legacy shell path is opt-in via [`LEGACY_SHELL_INSTALL_ENV`].
pub fn self_update_install_mode() -> &'static str {
    if legacy_shell_install_enabled() {
        "legacy_shell"
    } else {
        "verified_in_process"
    }
}

fn legacy_shell_install_enabled() -> bool {
    env::var(LEGACY_SHELL_INSTALL_ENV)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Host triple for published release archives (aligned with `scripts/install.sh` `detect_target`).
pub fn release_target_triple() -> Result<String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok("x86_64-pc-windows-msvc".to_string())
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok("aarch64-apple-darwin".to_string())
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok("x86_64-apple-darwin".to_string())
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok("x86_64-unknown-linux-gnu".to_string())
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    {
        bail!(
            "no prebuilt GitHub release for this host ({}-{}). Build from source or set {}=1 to use the legacy remote-shell installer (not recommended for production).",
            env::consts::OS,
            env::consts::ARCH,
            LEGACY_SHELL_INSTALL_ENV
        );
    }
}

fn version_label_from_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('v').to_string()
}

fn archive_basename(version_label: &str, target_triple: &str) -> String {
    let ext = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("{BIN_NAME}-{version_label}-{target_triple}.{ext}")
}

fn release_download_base(tag: &str) -> Result<String> {
    let (owner, name) = parse_repo_slug(&repo_slug())?;
    let tag = tag.trim();
    Ok(format!(
        "https://github.com/{owner}/{name}/releases/download/{tag}"
    ))
}

fn ureq_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(&format!(
            "mvs-manager/{} self-update",
            crate::update::current_version()
        ))
        .build()
}

fn apply_github_auth(mut request: ureq::Request) -> ureq::Request {
    if let Some(token) = github_token() {
        let value = format!("Bearer {token}");
        request = request.set("Authorization", value.as_str());
    }
    request
}

/// Stream `GET url` to `dest_path` and return SHA-256 of the bytes (streaming hash).
fn download_and_sha256(url: &str, dest_path: &Path) -> Result<[u8; 32]> {
    let request = apply_github_auth(ureq_agent().get(url));
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
    loop {
        let n = reader
            .read(&mut buf)
            .context("failed reading download stream")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .with_context(|| format!("failed writing `{}`", dest_path.display()))?;
    }
    let digest = hasher.finalize();
    Ok(digest.into())
}

fn parse_hex_sha256(word: &str) -> Result<[u8; 32]> {
    let s = word.trim();
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid SHA-256 hex token (expected 64 hex chars): `{s}`");
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("invalid SHA-256 hex at position {i}"))?;
    }
    Ok(out)
}

/// Find the `sha256sum`-style line for `basename` in `checksums.txt` (same contract as `scripts/install.sh`).
pub fn expected_sha256_from_checksums(content: &str, basename: &str) -> Result<[u8; 32]> {
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let hash = parts[0];
        let name_field = parts[parts.len() - 1];
        let name = name_field.strip_prefix('*').unwrap_or(name_field);
        if name == basename {
            return parse_hex_sha256(hash);
        }
    }
    bail!("missing checksum line for `{basename}` in checksums.txt")
}

fn extract_binary(archive_path: &Path, work_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(work_dir).context("failed to create extract dir")?;
    if archive_path.extension().and_then(|e| e.to_str()) == Some("zip")
        || archive_path.to_string_lossy().ends_with(".zip")
    {
        extract_zip_binary(archive_path, work_dir)
    } else {
        extract_tar_gz_binary(archive_path, work_dir)
    }
}

fn extract_tar_gz_binary(archive_path: &Path, work_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(work_dir).context("failed to create extract dir")?;
    let file = File::open(archive_path).context("open archive")?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    let mut found = None;
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .context("tar entry path")?
            .to_string_lossy()
            .into_owned();
        if path == BIN_NAME || path == format!("./{BIN_NAME}") {
            let dest = work_dir.join(BIN_NAME);
            entry.unpack(&dest).with_context(|| {
                format!(
                    "failed to unpack `{}` from {}",
                    dest.display(),
                    archive_path.display()
                )
            })?;
            found = Some(dest);
            break;
        }
    }
    found.ok_or_else(|| {
        anyhow!(
            "archive {} did not contain `{BIN_NAME}` at tarball root",
            archive_path.display()
        )
    })
}

fn extract_zip_binary(archive_path: &Path, work_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(work_dir).context("failed to create extract dir")?;
    let file = File::open(archive_path).context("open zip")?;
    let mut archive = zip::ZipArchive::new(file).context("read zip")?;
    let exe_name = format!("{BIN_NAME}.exe");
    for i in 0..archive.len() {
        let mut inner = archive.by_index(i).context("zip index")?;
        let name = inner.name().replace('\\', "/");
        if name == exe_name || name == format!("./{exe_name}") {
            let dest = work_dir.join(&exe_name);
            let mut out = File::create(&dest).context("create extracted exe")?;
            io::copy(&mut inner, &mut out).context("copy zip member")?;
            return Ok(dest);
        }
    }
    bail!(
        "archive {} did not contain `{exe_name}` at zip root",
        archive_path.display()
    )
}

#[cfg(unix)]
fn set_executable_unix(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(unix)]
fn install_binary_unix(src: &Path, dest_dir: &Path) -> Result<()> {
    let dest = dest_dir.join(BIN_NAME);
    let tmp = dest_dir.join(format!("{BIN_NAME}.new"));
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }
    fs::copy(src, &tmp).with_context(|| format!("copy to {}", tmp.display()))?;
    set_executable_unix(&tmp)?;
    fs::rename(&tmp, &dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}

#[cfg(windows)]
fn install_binary_windows(src: &Path, dest_dir: &Path) -> Result<()> {
    let dest = dest_dir.join(format!("{BIN_NAME}.exe"));
    let tmp = dest_dir.join(format!("{BIN_NAME}.exe.new"));
    if tmp.exists() {
        fs::remove_file(&tmp).ok();
    }
    fs::copy(src, &tmp).with_context(|| format!("copy new binary to {}", tmp.display()))?;
    if dest.exists() {
        fs::remove_file(&dest).with_context(|| {
            format!(
                "cannot replace `{}`: file in use or permission denied (exit other instances of mvs-manager and retry)",
                dest.display()
            )
        })?;
    }
    fs::rename(&tmp, &dest)
        .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn install_binary(src: &Path, dest_dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        install_binary_unix(src, dest_dir)
    }
    #[cfg(windows)]
    {
        install_binary_windows(src, dest_dir)
    }
    #[cfg(not(any(unix, windows)))]
    {
        bail!("unsupported OS for in-process install");
    }
}

/// Download release `tag`, verify checksums, extract, and install into `dest_dir`.
pub fn install_verified_release(tag: &str, dest_dir: &Path) -> Result<()> {
    let target = release_target_triple()?;
    let version_label = version_label_from_tag(tag);
    let basename = archive_basename(&version_label, &target);
    let base = release_download_base(tag)?;
    let checksums_url = format!("{base}/checksums.txt");
    let archive_url = format!("{base}/{basename}");

    let work_root = env::temp_dir().join(format!(
        "mvs-manager-install-{}-{}",
        std::process::id(),
        crate::update::now_unix()
    ));
    fs::create_dir_all(&work_root).context("temp install dir")?;

    let outcome: Result<()> = (|| {
        let checksums_path = work_root.join("checksums.txt");
        let archive_path = work_root.join(&basename);
        let extract_dir = work_root.join("extract");

        download_and_sha256(&checksums_url, &checksums_path)
            .with_context(|| format!("download checksums from {checksums_url}"))?;
        let checksums_txt = fs::read_to_string(&checksums_path).context("read checksums.txt")?;
        let expected = expected_sha256_from_checksums(&checksums_txt, &basename)?;

        let actual = download_and_sha256(&archive_url, &archive_path)
            .with_context(|| format!("download archive from {archive_url}"))?;
        if actual != expected {
            bail!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                basename,
                hex_lower(&expected),
                hex_lower(&actual)
            );
        }

        let binary = extract_binary(&archive_path, &extract_dir)?;
        install_binary(&binary, dest_dir)?;
        Ok(())
    })();

    let _ = fs::remove_dir_all(&work_root);
    outcome
}

fn hex_lower(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::Header;

    fn fixture_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install_release")
    }

    fn sha256_file(path: &Path) -> Result<[u8; 32]> {
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().into())
    }

    #[test]
    fn parses_checksums_line() {
        let txt = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  mvs-manager-1.0.0-x86_64-unknown-linux-gnu.tar.gz\n";
        let h = expected_sha256_from_checksums(
            txt,
            "mvs-manager-1.0.0-x86_64-unknown-linux-gnu.tar.gz",
        )
        .unwrap();
        assert_eq!(
            hex_lower(&h),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn rejects_missing_checksum_line() {
        let txt = "deadbeef00000000000000000000000000000000000000000000000000000000  other.tgz\n";
        assert!(expected_sha256_from_checksums(txt, "missing.tar.gz").is_err());
    }

    #[test]
    fn extract_tar_gz_finds_binary() -> Result<()> {
        let work = env::temp_dir().join(format!(
            "mvs-tar-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(&work)?;
        let archive_path = work.join("dist.tar.gz");
        let f = File::create(&archive_path)?;
        let enc = GzEncoder::new(f, Compression::default());
        let mut ar = tar::Builder::new(enc);
        let data: &[u8] = b"fake-binary";
        let mut header = Header::new_gnu();
        header.set_path(BIN_NAME)?;
        header.set_size(data.len() as u64);
        #[cfg(unix)]
        header.set_mode(0o755);
        header.set_cksum();
        ar.append(&header, data)?;
        let mut gz_inner = ar.into_inner()?;
        gz_inner.flush()?;
        let _file = gz_inner.finish()?;

        let extract_dir = work.join("ex");
        let got = extract_tar_gz_binary(&archive_path, &extract_dir)?;
        assert_eq!(fs::read_to_string(&got)?, "fake-binary");
        let _ = fs::remove_dir_all(&work);
        Ok(())
    }

    #[test]
    fn fixture_tar_checksum_matches_file() -> Result<()> {
        let dir = fixture_dir();
        let sums = fs::read_to_string(dir.join("checksums.txt"))?;
        let name = "mvs-manager-9.9.9-fixture-x86_64-unknown-linux-gnu.tar.gz";
        let expected = expected_sha256_from_checksums(&sums, name)?;
        let actual = sha256_file(&dir.join(name))?;
        assert_eq!(expected, actual);
        Ok(())
    }

    #[test]
    fn fixture_zip_checksum_matches_file() -> Result<()> {
        let dir = fixture_dir();
        let sums = fs::read_to_string(dir.join("checksums.txt"))?;
        let name = "mvs-manager-9.9.9-fixture-x86_64-pc-windows-msvc.zip";
        let expected = expected_sha256_from_checksums(&sums, name)?;
        let actual = sha256_file(&dir.join(name))?;
        assert_eq!(expected, actual);
        Ok(())
    }

    #[test]
    fn wrong_checksum_line_does_not_match_fixture_tar_bytes() -> Result<()> {
        let dir = fixture_dir();
        let name = "mvs-manager-9.9.9-fixture-x86_64-unknown-linux-gnu.tar.gz";
        let actual = sha256_file(&dir.join(name))?;
        let fake_sums = format!("{}  {name}\n", "0".repeat(64));
        let bogus_expected = expected_sha256_from_checksums(&fake_sums, name)?;
        assert_ne!(bogus_expected, actual);
        Ok(())
    }

    #[test]
    fn fixture_extract_tar_reads_root_binary() -> Result<()> {
        let dir = fixture_dir();
        let extract_dir =
            env::temp_dir().join(format!("mvs-fixture-tar-{}", crate::update::now_unix()));
        let _ = fs::remove_dir_all(&extract_dir);
        let got = extract_tar_gz_binary(
            &dir.join("mvs-manager-9.9.9-fixture-x86_64-unknown-linux-gnu.tar.gz"),
            &extract_dir,
        )?;
        assert_eq!(fs::read_to_string(&got)?, "fixture-payload");
        let _ = fs::remove_dir_all(&extract_dir);
        Ok(())
    }

    #[test]
    fn fixture_extract_zip_reads_root_exe() -> Result<()> {
        let dir = fixture_dir();
        let extract_dir =
            env::temp_dir().join(format!("mvs-fixture-zip-{}", crate::update::now_unix()));
        let _ = fs::remove_dir_all(&extract_dir);
        let got = extract_zip_binary(
            &dir.join("mvs-manager-9.9.9-fixture-x86_64-pc-windows-msvc.zip"),
            &extract_dir,
        )?;
        assert_eq!(fs::read_to_string(&got)?, "fixture-exe");
        let _ = fs::remove_dir_all(&extract_dir);
        Ok(())
    }

    #[test]
    fn rejects_invalid_sha256_hex_token() {
        assert!(parse_hex_sha256("not-hex").is_err());
        assert!(parse_hex_sha256("abcd").is_err());
    }
}
