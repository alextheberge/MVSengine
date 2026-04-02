// SPDX-License-Identifier: AGPL-3.0-only
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const REPO_OWNER: &str = "alextheberge";
const REPO_NAME: &str = "MVSengine";
const REPO_SLUG: &str = "alextheberge/MVSengine";
const BIN_NAME: &str = "mvs-manager";
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 60 * 60 * 24;
const UPDATE_DISABLE_ENV: &str = "MVS_NO_UPDATE_CHECK";
const UPDATE_FORCE_ENV: &str = "MVS_FORCE_UPDATE_CHECK";
const UPDATE_STATE_FILE_ENV: &str = "MVS_UPDATE_STATE_FILE";
const UPDATE_INTERVAL_ENV: &str = "MVS_UPDATE_CHECK_INTERVAL_SECS";
const UPDATE_LATEST_VERSION_ENV: &str = "MVS_UPDATE_LATEST_VERSION";
const UPDATE_LATEST_URL_ENV: &str = "MVS_UPDATE_LATEST_URL";
const UPDATE_GITHUB_TOKEN_ENV: &str = "MVS_GITHUB_TOKEN";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/alextheberge/MVSengine/releases/latest";
#[cfg(not(target_os = "windows"))]
const INSTALL_SH_URL: &str =
    "https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.sh";
#[cfg(target_os = "windows")]
const INSTALL_PS1_URL: &str =
    "https://raw.githubusercontent.com/alextheberge/MVSengine/master/scripts/install.ps1";

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

    run_installer(&release.tag)?;

    let mut state = load_state().unwrap_or_default();
    state.last_checked_unix = Some(now_unix());
    state.latest_version = Some(release.version.clone());
    state.release_url = Some(release.release_url.clone());
    state.last_notified_version = None;
    save_state(&state)?;

    Ok(release)
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
            .unwrap_or_else(|| release_url_for(&version));
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
        .unwrap_or_else(|| release_url_for(&version));

    Ok(ReleaseInfo {
        tag,
        version,
        release_url,
    })
}

fn fetch_latest_release_payload() -> Result<Value> {
    fetch_json(GITHUB_LATEST_RELEASE_URL)
}

fn github_token() -> Option<String> {
    env::var(UPDATE_GITHUB_TOKEN_ENV)
        .ok()
        .or_else(|| env::var("GITHUB_TOKEN").ok())
        .filter(|value| !value.trim().is_empty())
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

    #[cfg(target_os = "windows")]
    {
        let install_dir = escape_powershell_single_quoted(&install_dir.to_string_lossy());
        let tag = escape_powershell_single_quoted(tag);
        let script = format!(
            "$env:MVS_REPO = '{REPO_SLUG}'; $env:MVS_VERSION = '{tag}'; $env:MVS_INSTALL_DIR = '{install_dir}'; irm '{INSTALL_PS1_URL}' | iex"
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
        let output = Command::new("bash")
            .args(["-lc", &format!("curl -fsSL {INSTALL_SH_URL} | bash")])
            .env("MVS_REPO", REPO_SLUG)
            .env("MVS_VERSION", tag)
            .env("MVS_INSTALL_DIR", &install_dir)
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
        .unwrap_or_else(|| release_url_for(&version));
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

fn default_state_file_path() -> Option<PathBuf> {
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

fn release_url_for(version: &str) -> String {
    format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/v{version}")
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

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
