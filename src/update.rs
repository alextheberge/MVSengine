// SPDX-License-Identifier: AGPL-3.0-only
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_REPO_OWNER: &str = "alextheberge";
const DEFAULT_REPO_NAME: &str = "MVSengine";
const BIN_NAME: &str = "mvs-manager";
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 60 * 60 * 24;
pub const UPDATE_DISABLE_ENV: &str = "MVS_NO_UPDATE_CHECK";
const UPDATE_FORCE_ENV: &str = "MVS_FORCE_UPDATE_CHECK";
const UPDATE_STATE_FILE_ENV: &str = "MVS_UPDATE_STATE_FILE";
const UPDATE_INTERVAL_ENV: &str = "MVS_UPDATE_CHECK_INTERVAL_SECS";
const UPDATE_LATEST_VERSION_ENV: &str = "MVS_UPDATE_LATEST_VERSION";
const UPDATE_LATEST_URL_ENV: &str = "MVS_UPDATE_LATEST_URL";
pub const UPDATE_GITHUB_TOKEN_ENV: &str = "MVS_GITHUB_TOKEN";
/// When set to `1`/`true`/`yes`/`on`, allows `self-update` from Cargo build dirs, Nix store, etc.
pub const ALLOW_UNSAFE_SELF_UPDATE_ENV: &str = "MVS_ALLOW_UNSAFE_SELF_UPDATE";
/// GitHub `owner/name` for releases and install scripts (same as `install.sh` `MVS_REPO`).
pub const MVS_REPO_ENV: &str = "MVS_REPO";
/// Alias for `MVS_REPO` (forks / private mirrors).
pub const MVS_UPDATE_REPO_ENV: &str = "MVS_UPDATE_REPO";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag: String,
    pub release_url: String,
}

#[derive(Debug, Clone)]
pub enum CheckStatus {
    UpToDate,
    UpdateAvailable(ReleaseInfo),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UpdateState {
    last_checked_unix: Option<u64>,
    latest_version: Option<String>,
    release_url: Option<String>,
    last_notified_version: Option<String>,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Resolved `owner/repo` for GitHub API and install scripts (from `MVS_REPO` / `MVS_UPDATE_REPO` or default).
pub fn repo_slug() -> String {
    env::var(MVS_REPO_ENV)
        .or_else(|_| env::var(MVS_UPDATE_REPO_ENV))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("{DEFAULT_REPO_OWNER}/{DEFAULT_REPO_NAME}"))
}

pub(crate) fn parse_repo_slug(slug: &str) -> Result<(String, String)> {
    let slug = slug.trim();
    let parts: Vec<&str> = slug.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        bail!(
            "invalid repository slug `{slug}`: expected `owner/name` (set {MVS_REPO_ENV} or {MVS_UPDATE_REPO_ENV})"
        );
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

pub fn github_latest_release_api_url() -> Result<String> {
    let (owner, name) = parse_repo_slug(&repo_slug())?;
    Ok(format!(
        "https://api.github.com/repos/{owner}/{name}/releases/latest"
    ))
}

pub fn install_sh_raw_url() -> Result<String> {
    let (owner, name) = parse_repo_slug(&repo_slug())?;
    Ok(format!(
        "https://raw.githubusercontent.com/{owner}/{name}/master/scripts/install.sh"
    ))
}

pub fn install_ps1_raw_url() -> Result<String> {
    let (owner, name) = parse_repo_slug(&repo_slug())?;
    Ok(format!(
        "https://raw.githubusercontent.com/{owner}/{name}/master/scripts/install.ps1"
    ))
}

fn release_url_for(version: &str) -> Result<String> {
    let (owner, name) = parse_repo_slug(&repo_slug())?;
    Ok(format!(
        "https://github.com/{owner}/{name}/releases/tag/v{version}"
    ))
}

pub fn maybe_notify_new_version() {
    if !should_auto_check() {
        return;
    }

    if let Ok(mut state) = load_state() {
        let release = match cached_or_fetch_latest_release(&mut state) {
            Ok(release) => release,
            Err(_) => {
                let _ = save_state(&state);
                return;
            }
        };

        if is_newer_than_current(&release.version).unwrap_or(false)
            && state.last_notified_version.as_deref() != Some(release.version.as_str())
        {
            eprintln!(
                "update available: v{} -> v{}. Run `mvs-manager self-update` to install it.",
                current_version(),
                release.version
            );
            state.last_notified_version = Some(release.version);
        } else if !is_newer_than_current(&release.version).unwrap_or(false) {
            state.last_notified_version = None;
        }

        let _ = save_state(&state);
    }
}

pub fn check_for_update() -> Result<CheckStatus> {
    let mut state = load_state().unwrap_or_default();
    let release = cached_or_fetch_latest_release(&mut state)?;
    save_state(&state)?;

    if is_newer_than_current(&release.version)? {
        Ok(CheckStatus::UpdateAvailable(release))
    } else {
        Ok(CheckStatus::UpToDate)
    }
}

pub fn perform_self_update() -> Result<ReleaseInfo> {
    let release = fetch_latest_release()?;
    if !is_newer_than_current(&release.version)? {
        let mut state = load_state().unwrap_or_default();
        state.last_checked_unix = Some(now_unix());
        state.latest_version = Some(release.version.clone());
        state.release_url = Some(release.release_url.clone());
        state.last_notified_version = None;
        save_state(&state)?;
        return Ok(release);
    }

    ensure_self_update_safe()?;
    run_installer(&release.tag)?;

    let mut state = load_state().unwrap_or_default();
    state.last_checked_unix = Some(now_unix());
    state.latest_version = Some(release.version.clone());
    state.release_url = Some(release.release_url.clone());
    state.last_notified_version = None;
    save_state(&state)?;

    Ok(release)
}

/// Whether `self-update` would refuse to run (without `MVS_ALLOW_UNSAFE_SELF_UPDATE`).
/// When `Some`, `self-update` would refuse to replace the binary (unless `MVS_ALLOW_UNSAFE_SELF_UPDATE` is set).
pub fn self_update_block_reason() -> Option<String> {
    if env_flag(ALLOW_UNSAFE_SELF_UPDATE_ENV) {
        return None;
    }
    self_update_unsafe_reason().err()
}

fn self_update_unsafe_reason() -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let Some(parent) = exe.parent() else {
        return Err("current executable has no parent directory".to_string());
    };

    let lossy = exe.to_string_lossy();
    let lower = lossy.to_ascii_lowercase();

    if lower.contains("/target/debug/")
        || lower.contains("/target/release/")
        || lower.contains("\\target\\debug\\")
        || lower.contains("\\target\\release\\")
    {
        return Err(format!(
            "refusing self-update: binary appears to be a Cargo build (`{}`). \
Install a release build to e.g. $HOME/.local/bin (see scripts/install.sh) or set {}=1 to override.",
            lossy, ALLOW_UNSAFE_SELF_UPDATE_ENV
        ));
    }

    if lower.contains("/.cargo/") || lower.contains("\\.cargo\\") {
        return Err(format!(
            "refusing self-update: binary is under `.cargo` (`{}`). \
Use `cargo install` refresh or a release install path, or set {}=1.",
            lossy, ALLOW_UNSAFE_SELF_UPDATE_ENV
        ));
    }

    if lower.contains("/nix/store/") {
        return Err(format!(
            "refusing self-update: binary is in the Nix store (`{}`), which is read-only. \
Update via your Nix flake or package, or set {}=1.",
            lossy, ALLOW_UNSAFE_SELF_UPDATE_ENV
        ));
    }

    let probe = parent.join(format!(
        ".mvs-manager-writetest-{}-{}",
        std::process::id(),
        now_unix()
    ));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
        }
        Err(error) => {
            return Err(format!(
                "refusing self-update: cannot write next to the binary at `{}` ({error}). \
Set MVS_INSTALL_DIR and run the install script, or set {}=1.",
                parent.display(),
                ALLOW_UNSAFE_SELF_UPDATE_ENV
            ));
        }
    }

    Ok(())
}

fn ensure_self_update_safe() -> Result<()> {
    if env_flag(ALLOW_UNSAFE_SELF_UPDATE_ENV) {
        return Ok(());
    }
    if let Err(msg) = self_update_unsafe_reason() {
        bail!("{msg}");
    }
    Ok(())
}

fn should_auto_check() -> bool {
    if env_flag(UPDATE_DISABLE_ENV) {
        return false;
    }

    env_flag(UPDATE_FORCE_ENV) || std::io::stderr().is_terminal()
}

fn cached_or_fetch_latest_release(state: &mut UpdateState) -> Result<ReleaseInfo> {
    let now = now_unix();
    let interval = env::var(UPDATE_INTERVAL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_CHECK_INTERVAL_SECS);
    let stale = state
        .last_checked_unix
        .map(|checked| now.saturating_sub(checked) >= interval)
        .unwrap_or(true);

    if !stale {
        if let Some(release) = release_from_state(state) {
            return Ok(release);
        }
    }

    match fetch_latest_release() {
        Ok(release) => {
            state.last_checked_unix = Some(now);
            state.latest_version = Some(release.version.clone());
            state.release_url = Some(release.release_url.clone());
            Ok(release)
        }
        Err(error) => release_from_state(state).ok_or(error),
    }
}

fn fetch_latest_release() -> Result<ReleaseInfo> {
    if let Ok(version) = env::var(UPDATE_LATEST_VERSION_ENV) {
        let version = version.trim().trim_start_matches('v').to_string();
        validate_version(&version)?;
        let release_url = env::var(UPDATE_LATEST_URL_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| release_url_for(&version).unwrap_or_default());
        return Ok(ReleaseInfo {
            tag: format!("v{version}"),
            version,
            release_url,
        });
    }

    let payload = fetch_latest_release_payload()?;
    let tag = payload["tag_name"]
        .as_str()
        .context("latest release payload is missing `tag_name`")?
        .trim()
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    validate_version(&version)?;
    let release_url = payload["html_url"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| release_url_for(&version).unwrap_or_default());

    Ok(ReleaseInfo {
        tag,
        version,
        release_url,
    })
}

fn fetch_latest_release_payload() -> Result<Value> {
    let url = github_latest_release_api_url()?;
    fetch_json(&url)
}

pub(crate) fn github_token() -> Option<String> {
    env::var(UPDATE_GITHUB_TOKEN_ENV)
        .ok()
        .or_else(|| env::var("GITHUB_TOKEN").ok())
        .filter(|value| !value.trim().is_empty())
}

pub fn github_token_configured() -> bool {
    github_token().is_some()
}

fn is_newer_than_current(version: &str) -> Result<bool> {
    let latest = validate_version(version)?;
    let current = validate_version(current_version())?;
    Ok(latest > current)
}

fn validate_version(version: &str) -> Result<Version> {
    Version::parse(version)
        .with_context(|| format!("release version `{version}` is not valid semver"))
}

fn fetch_json(url: &str) -> Result<Value> {
    #[cfg(target_os = "windows")]
    {
        fetch_json_with_powershell(url).or_else(|_| fetch_json_with_curl(url))
    }

    #[cfg(not(target_os = "windows"))]
    {
        fetch_json_with_curl(url).or_else(|_| fetch_json_with_wget(url))
    }
}

#[cfg(target_os = "windows")]
fn fetch_json_with_powershell(url: &str) -> Result<Value> {
    let auth = github_token()
        .map(|token| {
            format!(
                "$headers.Authorization = 'Bearer {}'; ",
                escape_powershell_single_quoted(&token)
            )
        })
        .unwrap_or_default();
    let script = format!(
        "$headers = @{{ 'User-Agent' = 'mvs-manager'; 'Accept' = 'application/vnd.github+json' }}; {auth}Invoke-RestMethod -Method Get -Headers $headers -Uri '{url}' | ConvertTo-Json -Depth 100"
    );
    run_json_command("powershell", &["-NoProfile", "-Command", &script])
}

fn fetch_json_with_curl(url: &str) -> Result<Value> {
    let mut owned_args = vec![
        "-fsSL".to_string(),
        "-H".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        "-H".to_string(),
        "User-Agent: mvs-manager".to_string(),
    ];
    if let Some(token) = github_token() {
        owned_args.push("-H".to_string());
        owned_args.push(format!("Authorization: Bearer {token}"));
    }
    owned_args.push(url.to_string());
    let borrowed = owned_args.iter().map(String::as_str).collect::<Vec<_>>();
    run_json_command("curl", &borrowed)
}

#[cfg(not(target_os = "windows"))]
fn fetch_json_with_wget(url: &str) -> Result<Value> {
    let mut owned_args = vec![
        "--quiet".to_string(),
        "-O".to_string(),
        "-".to_string(),
        "--header".to_string(),
        "Accept: application/vnd.github+json".to_string(),
        "--header".to_string(),
        "User-Agent: mvs-manager".to_string(),
    ];
    if let Some(token) = github_token() {
        owned_args.push("--header".to_string());
        owned_args.push(format!("Authorization: Bearer {token}"));
    }
    owned_args.push(url.to_string());
    let borrowed = owned_args.iter().map(String::as_str).collect::<Vec<_>>();
    run_json_command("wget", &borrowed)
}

fn run_json_command(program: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute `{program}`"))?;
    if !output.status.success() {
        bail!(
            "`{program}` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("`{program}` returned invalid JSON"))
}

fn run_installer(tag: &str) -> Result<()> {
    let install_dir = current_install_dir()?;
    if crate::install_release::self_update_install_mode() == "legacy_shell" {
        return run_installer_shell(tag, &install_dir);
    }
    crate::install_release::install_verified_release(tag, &install_dir)
        .context("in-process verified install failed")
}

fn run_installer_shell(tag: &str, install_dir: &Path) -> Result<()> {
    let repo = repo_slug();

    #[cfg(target_os = "windows")]
    {
        let install_dir_esc = escape_powershell_single_quoted(&install_dir.to_string_lossy());
        let tag_esc = escape_powershell_single_quoted(tag);
        let repo_esc = escape_powershell_single_quoted(&repo);
        let script_url = install_ps1_raw_url()?;
        let script_url_esc = escape_powershell_single_quoted(&script_url);
        let script = format!(
            "$env:MVS_REPO = '{repo_esc}'; $env:MVS_VERSION = '{tag_esc}'; $env:MVS_INSTALL_DIR = '{install_dir_esc}'; irm '{script_url_esc}' | iex"
        );
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .context("failed to launch PowerShell installer")?;
        if !output.status.success() {
            bail!(
                "installer failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let install_sh_url = install_sh_raw_url()?;
        let output = Command::new("bash")
            .args(["-lc", &format!("curl -fsSL {install_sh_url} | bash")])
            .env(MVS_REPO_ENV, &repo)
            .env("MVS_VERSION", tag)
            .env("MVS_INSTALL_DIR", install_dir)
            .output()
            .context("failed to launch installer shell")?;
        if !output.status.success() {
            bail!(
                "installer failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

fn current_install_dir() -> Result<PathBuf> {
    let exe = env::current_exe().context("failed to resolve current executable path")?;
    exe.parent()
        .map(PathBuf::from)
        .context("current executable has no parent directory")
}

#[cfg(target_os = "windows")]
fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn release_from_state(state: &UpdateState) -> Option<ReleaseInfo> {
    let version = state.latest_version.as_ref()?.trim().to_string();
    if validate_version(&version).is_err() {
        return None;
    }

    let release_url = state
        .release_url
        .clone()
        .or_else(|| release_url_for(&version).ok())?;
    Some(ReleaseInfo {
        tag: format!("v{version}"),
        version,
        release_url,
    })
}

fn load_state() -> Result<UpdateState> {
    let path = state_file_path()?;
    if !path.exists() {
        return Ok(UpdateState::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read update state `{}`", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse update state `{}`", path.display()))
}

fn save_state(state: &UpdateState) -> Result<()> {
    let path = state_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create update state directory `{}`",
                parent.display()
            )
        })?;
    }

    let payload = serde_json::to_string_pretty(state)?;
    fs::write(&path, payload)
        .with_context(|| format!("failed to write update state `{}`", path.display()))
}

fn state_file_path() -> Result<PathBuf> {
    if let Ok(path) = env::var(UPDATE_STATE_FILE_ENV) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    default_state_file_path().context("unable to determine update state file path")
}

/// Path used for update-check cache (for diagnostics).
pub fn default_state_file_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var("LOCALAPPDATA")
            .ok()
            .map(PathBuf::from)
            .map(|path| path.join(BIN_NAME).join("update-state.json"))
    }

    #[cfg(target_os = "macos")]
    {
        env::var("HOME").ok().map(PathBuf::from).map(|path| {
            path.join("Library/Caches")
                .join(BIN_NAME)
                .join("update-state.json")
        })
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        env::var("XDG_CACHE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                env::var("HOME")
                    .ok()
                    .map(PathBuf::from)
                    .map(|path| path.join(".cache"))
            })
            .map(|path| path.join(BIN_NAME).join("update-state.json"))
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Best-effort: first `mvs-manager` on `PATH` (may differ from `current_exe` when multiple installs exist).
pub fn which_mvs_manager() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let output = Command::new("where").arg(BIN_NAME).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().find(|l| !l.trim().is_empty())?;
        let path = PathBuf::from(line.trim());
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = Command::new("which").arg(BIN_NAME).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().next()?;
        let path = PathBuf::from(line.trim());
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
}

/// True if `which mvs-manager` resolves to the same file as `current_exe` (when both exist).
pub fn path_matches_primary_install() -> bool {
    match (env::current_exe().ok(), which_mvs_manager()) {
        (Some(a), Some(b)) => same_file(&a, &b),
        _ => false,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_urls_reflect_mvs_repo_env() {
        let prev = env::var(MVS_REPO_ENV).ok();
        env::set_var(MVS_REPO_ENV, "myfork/MVSengine");
        assert!(install_sh_raw_url().unwrap().contains("myfork/MVSengine"));
        assert!(github_latest_release_api_url()
            .unwrap()
            .contains("myfork/MVSengine"));
        match prev {
            Some(v) => env::set_var(MVS_REPO_ENV, v),
            None => env::remove_var(MVS_REPO_ENV),
        }
    }
}
