// SPDX-License-Identifier: AGPL-3.0-only
//! Download a GitHub release archive, verify SHA-256 against `checksums.txt`, extract, and install
//! the `mvs-manager` binary without executing a remote shell script.

use std::env;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use minisign::{PublicKeyBox, SignatureBox};

use crate::github_http::{
    download_to_file_sha256_limited, get_github_bytes_optional, MAX_ARCHIVE_DOWNLOAD_BYTES,
    MAX_SMALL_DOWNLOAD_BYTES,
};
use crate::update::{parse_repo_slug, repo_slug};

/// Override embedded [`MINISIGN_PUBLIC_KEY_EMBED`]: full minisign public key file contents (two lines).
pub const MVS_MINISIGN_PUBLIC_KEY_ENV: &str = "MVS_MINISIGN_PUBLIC_KEY";

const MINISIGN_PUBLIC_KEY_EMBED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/packaging/minisign.pub"
));

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

fn load_minisign_public_key() -> Result<minisign::PublicKey> {
    if let Ok(raw) = env::var(MVS_MINISIGN_PUBLIC_KEY_ENV) {
        let t = raw.trim();
        if !t.is_empty() {
            return PublicKeyBox::from_string(t)
                .and_then(|b| b.into_public_key())
                .map_err(|e| anyhow!("{e}"));
        }
    }
    let t = MINISIGN_PUBLIC_KEY_EMBED.trim();
    if t.is_empty() || t.starts_with('#') {
        bail!(
            "release includes checksums.txt.minisig but no minisign public key is configured; set {} to the public key file contents (see packaging/minisign.pub in this repository)",
            MVS_MINISIGN_PUBLIC_KEY_ENV
        );
    }
    PublicKeyBox::from_string(t)
        .and_then(|b| b.into_public_key())
        .map_err(|e| anyhow!("{e}"))
}

/// When `checksums.txt.minisig` exists on the release, verify it before trusting `checksums.txt`.
fn verify_checksums_minisig_if_present(base: &str, checksums_bytes: &[u8]) -> Result<()> {
    let sig_url = format!("{base}/checksums.txt.minisig");
    let Some(sig_raw) = get_github_bytes_optional(&sig_url)? else {
        return Ok(());
    };
    let sig_text =
        std::str::from_utf8(&sig_raw).context("checksums.txt.minisig is not valid UTF-8")?;
    let pk = load_minisign_public_key()?;
    let sig_box = SignatureBox::from_string(sig_text.trim()).map_err(|e| anyhow!("{e}"))?;
    let mut cursor = Cursor::new(checksums_bytes.to_vec());
    minisign::verify(&pk, &sig_box, &mut cursor, true, false, false).map_err(|e| anyhow!("{e}"))
}

fn is_safe_root_archive_path(name: &str, expected_file: &str) -> bool {
    let normalized = name.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    if trimmed.is_empty() || trimmed.contains("..") {
        return false;
    }
    let pb = Path::new(trimmed);
    if pb
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
    {
        return false;
    }
    if pb.components().count() != 1 {
        return false;
    }
    trimmed == expected_file
}

/// Parse a 64-character hex SHA-256 token (used by tests and fuzz targets).
pub fn parse_hex_sha256(word: &str) -> Result<[u8; 32]> {
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
    let mut found: Option<PathBuf> = None;
    for entry in archive.entries().context("read tar entries")? {
        let mut entry = entry.context("tar entry")?;
        let path = entry
            .path()
            .context("tar entry path")?
            .to_string_lossy()
            .into_owned();
        if !is_safe_root_archive_path(&path, BIN_NAME) {
            continue;
        }
        if entry.header().entry_type().is_symlink() {
            bail!(
                "refusing symlink entry masquerading as `{BIN_NAME}` in {}",
                archive_path.display()
            );
        }
        if !entry.header().entry_type().is_file() {
            continue;
        }
        if found.is_some() {
            bail!(
                "archive {} contains multiple `{BIN_NAME}` file entries",
                archive_path.display()
            );
        }
        let dest = work_dir.join(BIN_NAME);
        entry.unpack(&dest).with_context(|| {
            format!(
                "failed to unpack `{}` from {}",
                dest.display(),
                archive_path.display()
            )
        })?;
        found = Some(dest);
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
    let mut found: Option<PathBuf> = None;
    for i in 0..archive.len() {
        let mut inner = archive.by_index(i).context("zip index")?;
        let name = inner.name().replace('\\', "/");
        if !is_safe_root_archive_path(&name, &exe_name) {
            continue;
        }
        if inner.is_symlink() {
            bail!(
                "refusing symlink entry masquerading as `{exe_name}` in {}",
                archive_path.display()
            );
        }
        if inner.is_dir() {
            continue;
        }
        if found.is_some() {
            bail!(
                "archive {} contains multiple `{exe_name}` file entries",
                archive_path.display()
            );
        }
        let dest = work_dir.join(&exe_name);
        let mut out = File::create(&dest).context("create extracted exe")?;
        io::copy(&mut inner, &mut out).context("copy zip member")?;
        found = Some(dest);
    }
    found.ok_or_else(|| {
        anyhow!(
            "archive {} did not contain `{exe_name}` at zip root",
            archive_path.display()
        )
    })
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

        download_to_file_sha256_limited(&checksums_url, &checksums_path, MAX_SMALL_DOWNLOAD_BYTES)
            .with_context(|| format!("download checksums from {checksums_url}"))?;
        let checksums_bytes = fs::read(&checksums_path).context("read checksums.txt")?;
        verify_checksums_minisig_if_present(&base, &checksums_bytes)?;
        let checksums_txt =
            String::from_utf8(checksums_bytes).context("checksums.txt is not valid UTF-8")?;
        let expected = expected_sha256_from_checksums(&checksums_txt, &basename)?;

        let actual = download_to_file_sha256_limited(
            &archive_url,
            &archive_path,
            MAX_ARCHIVE_DOWNLOAD_BYTES,
        )
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
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
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

    #[test]
    fn fixture_checksums_minisig_verifies_with_embedded_pubkey() -> Result<()> {
        let dir = fixture_dir();
        let sums = fs::read(dir.join("checksums.txt"))?;
        let sig_txt = fs::read_to_string(dir.join("checksums.txt.minisig"))?;
        let pk = load_minisign_public_key()?;
        let sig_box = SignatureBox::from_string(sig_txt.trim()).map_err(|e| anyhow!("{e}"))?;
        let mut cursor = Cursor::new(sums);
        minisign::verify(&pk, &sig_box, &mut cursor, true, false, false).map_err(|e| anyhow!("{e}"))
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parse_hex_sha256_never_panics(s in ".{0,256}") {
            let _ = parse_hex_sha256(&s);
        }
        #[test]
        fn expected_sha256_from_checksums_never_panics(content in ".{0,512}", basename in "[a-z0-9._-]{1,64}") {
            let _ = expected_sha256_from_checksums(&content, &basename);
        }
    }
}
