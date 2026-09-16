// SPDX-License-Identifier: AGPL-3.0-only
//! Parses legacy version strings from common versioning schemes (SemVer,
//! ZeroVer, PEP 440, Maven/Gradle, .NET four-part, Go modules, CalVer, bare
//! build numbers, Debian/RPM `epoch:version-revision`, or a user-supplied
//! regex) into a normalized [`LegacyVersion`], then maps its components onto
//! MVS axes via a configurable [`AxisMapping`].
//!
//! This module is pure parsing and mapping logic — no file I/O — used by the
//! `convert-version` command today and, later, by `migrate`.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;

// ── Schemes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionScheme {
    /// `MAJOR.MINOR.PATCH[-pre][+build]`, `MAJOR >= 1`.
    Semver,
    /// The same shape as SemVer, but `MAJOR == 0`: minor bumps are commonly
    /// breaking in practice, so this is tagged distinctly for advisories.
    ZeroVer,
    /// PEP 440's common subset: `[N!]N[.N[.N]][{a|b|c|rc}N][.postN][.devN]`.
    Pep440,
    /// `MAJOR.MINOR[.PATCH][-QUALIFIER]` (Maven/Gradle-style, `SNAPSHOT`/`RC`/etc).
    MavenGradle,
    /// `A.B.C.D` (.NET/Windows-style four-part version).
    DotNet4,
    /// SemVer with an optional leading `v`/`V` (Go module tags).
    GoModules,
    /// `YYYY.MM[.patch]` or `YY.MM[.patch]`.
    CalVer,
    /// A bare non-negative integer build/CI counter.
    IntegerBuild,
    /// `[EPOCH:]UPSTREAM_VERSION[-REVISION]` (Debian/RPM packaging versions).
    DebianRpm,
    /// A user-supplied regex with named capture groups.
    Custom,
}

impl VersionScheme {
    pub fn as_str(&self) -> &'static str {
        match self {
            VersionScheme::Semver => "semver",
            VersionScheme::ZeroVer => "zerover",
            VersionScheme::Pep440 => "pep440",
            VersionScheme::MavenGradle => "maven-gradle",
            VersionScheme::DotNet4 => "dotnet4",
            VersionScheme::GoModules => "go-modules",
            VersionScheme::CalVer => "calver",
            VersionScheme::IntegerBuild => "integer",
            VersionScheme::DebianRpm => "debian-rpm",
            VersionScheme::Custom => "custom",
        }
    }

    pub fn parse_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "semver" => Some(Self::Semver),
            "zerover" | "zero-ver" => Some(Self::ZeroVer),
            "pep440" | "pep-440" => Some(Self::Pep440),
            "maven-gradle" | "maven" | "gradle" => Some(Self::MavenGradle),
            "dotnet4" | "dotnet" | "net4" => Some(Self::DotNet4),
            "go-modules" | "go" => Some(Self::GoModules),
            "calver" => Some(Self::CalVer),
            "integer" | "build-number" => Some(Self::IntegerBuild),
            "debian-rpm" | "debian" | "rpm" => Some(Self::DebianRpm),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// A version string normalized into ordered, named numeric components plus
/// the non-axis metadata (`epoch`, `prerelease`, `build_metadata`,
/// `revision`) most schemes also carry.
///
/// Component names are scheme-defined (`["major", "minor", "patch"]` for
/// SemVer, `["year", "month", "patch"]` for CalVer, arbitrary names for
/// `Custom`) and are exactly what `--map <name>=<axis>` refers to.
#[derive(Debug, Clone)]
pub struct LegacyVersion {
    pub raw: String,
    pub scheme: VersionScheme,
    pub components: Vec<(String, u64)>,
    pub epoch: Option<u64>,
    pub prerelease: Option<String>,
    pub build_metadata: Option<String>,
    pub revision: Option<String>,
}

/// Parses `raw` under an explicitly chosen `scheme`. For `VersionScheme::Custom`,
/// `custom_regex` must be `Some` and contain at least one named capture group.
pub fn parse(
    raw: &str,
    scheme: VersionScheme,
    custom_regex: Option<&str>,
) -> Result<LegacyVersion> {
    if scheme == VersionScheme::Custom {
        let pattern = custom_regex
            .ok_or_else(|| anyhow!("--scheme custom requires --scheme-regex <pattern>"))?;
        return parse_custom(raw, pattern);
    }

    let parsed = match scheme {
        VersionScheme::Semver | VersionScheme::ZeroVer => semver_shaped(raw, scheme),
        VersionScheme::Pep440 => parse_pep440(raw),
        VersionScheme::MavenGradle => parse_maven_gradle(raw),
        VersionScheme::DotNet4 => parse_dotnet4(raw),
        VersionScheme::GoModules => parse_go_modules(raw),
        VersionScheme::CalVer => parse_calver(raw),
        VersionScheme::IntegerBuild => parse_integer_build(raw),
        VersionScheme::DebianRpm => parse_debian_rpm(raw),
        VersionScheme::Custom => unreachable!("handled above"),
    };

    parsed.ok_or_else(|| {
        anyhow!(
            "`{raw}` does not match the `{}` scheme shape",
            scheme.as_str()
        )
    })
}

/// Auto-detects a scheme and parses `raw` under it. Detection tries the most
/// structurally distinctive shapes first (an epoch marker `:`, an exact
/// four-part version, PEP 440 pre/post/dev suffixes, a plausible CalVer
/// `year.month`, a Maven/Gradle qualifier) before falling back to plain
/// SemVer/ZeroVer.
pub fn parse_auto(raw: &str) -> Result<LegacyVersion> {
    detect(raw).ok_or_else(|| {
        anyhow!(
            "could not auto-detect a version scheme for `{raw}`; pass --scheme explicitly, \
             or --scheme custom --scheme-regex '(?P<arch>\\d+)\\.(?P<feat>\\d+)...'"
        )
    })
}

fn detect(raw: &str) -> Option<LegacyVersion> {
    let trimmed = raw.trim();

    if trimmed.contains(':') {
        if let Some(v) = parse_debian_rpm(trimmed) {
            return Some(v);
        }
    }
    if let Some(v) = parse_dotnet4(trimmed) {
        return Some(v);
    }
    if is_pep440_distinctive(trimmed) {
        if let Some(v) = parse_pep440(trimmed) {
            return Some(v);
        }
    }
    if let Some(v) = parse_calver(trimmed) {
        let year = v.components[0].1;
        let month = v.components[1].1;
        let has_patch = trimmed.matches('.').count() >= 2;
        if is_plausible_calver_autodetect(year, month, has_patch) {
            return Some(v);
        }
    }
    if let Some(v) = detect_maven_gradle_distinctive(trimmed) {
        return Some(v);
    }
    if let Some(v) = parse_integer_build(trimmed) {
        return Some(v);
    }
    if let Some(v) = semver_shaped(trimmed, VersionScheme::Semver) {
        let scheme = if v.components.first().map(|(_, value)| *value) == Some(0) {
            VersionScheme::ZeroVer
        } else {
            VersionScheme::Semver
        };
        return Some(LegacyVersion { scheme, ..v });
    }
    None
}

// ── Per-scheme parsers ───────────────────────────────────────────────────────

fn semver_shaped(raw: &str, scheme: VersionScheme) -> Option<LegacyVersion> {
    let re =
        Regex::new(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$").unwrap();
    let caps = re.captures(raw.trim())?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme,
        components: vec![
            ("major".to_string(), caps[1].parse().ok()?),
            ("minor".to_string(), caps[2].parse().ok()?),
            ("patch".to_string(), caps[3].parse().ok()?),
        ],
        epoch: None,
        prerelease: caps.get(4).map(|m| m.as_str().to_string()),
        build_metadata: caps.get(5).map(|m| m.as_str().to_string()),
        revision: None,
    })
}

fn parse_go_modules(raw: &str) -> Option<LegacyVersion> {
    let stripped = raw.trim().trim_start_matches(['v', 'V']);
    let inner = semver_shaped(stripped, VersionScheme::GoModules)?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        ..inner
    })
}

fn parse_dotnet4(raw: &str) -> Option<LegacyVersion> {
    let re = Regex::new(r"^(\d+)\.(\d+)\.(\d+)\.(\d+)$").unwrap();
    let caps = re.captures(raw.trim())?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::DotNet4,
        components: vec![
            ("major".to_string(), caps[1].parse().ok()?),
            ("minor".to_string(), caps[2].parse().ok()?),
            ("patch".to_string(), caps[3].parse().ok()?),
            ("revision".to_string(), caps[4].parse().ok()?),
        ],
        epoch: None,
        prerelease: None,
        build_metadata: None,
        revision: None,
    })
}

fn parse_maven_gradle(raw: &str) -> Option<LegacyVersion> {
    let re = Regex::new(r"^(\d+)\.(\d+)(?:\.(\d+))?(?:-([0-9A-Za-z.]+))?$").unwrap();
    let caps = re.captures(raw.trim())?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::MavenGradle,
        components: vec![
            ("major".to_string(), caps[1].parse().ok()?),
            ("minor".to_string(), caps[2].parse().ok()?),
            (
                "patch".to_string(),
                caps.get(3)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
            ),
        ],
        epoch: None,
        prerelease: caps.get(4).map(|m| m.as_str().to_string()),
        build_metadata: None,
        revision: None,
    })
}

/// Only accepted during auto-detection when the shape is unambiguous:
/// missing patch (`1.4`) or carrying an explicit qualifier (`1.4.0-SNAPSHOT`).
/// A plain `1.4.0` is left to fall through to the SemVer default.
fn detect_maven_gradle_distinctive(raw: &str) -> Option<LegacyVersion> {
    let v = parse_maven_gradle(raw)?;
    let has_patch = Regex::new(r"^\d+\.\d+\.\d+").unwrap().is_match(raw.trim());
    if !has_patch || v.prerelease.is_some() {
        Some(v)
    } else {
        None
    }
}

fn parse_calver(raw: &str) -> Option<LegacyVersion> {
    let re = Regex::new(r"^(\d{2,4})\.(\d{1,2})(?:\.(\d+))?$").unwrap();
    let caps = re.captures(raw.trim())?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::CalVer,
        components: vec![
            ("year".to_string(), caps[1].parse().ok()?),
            ("month".to_string(), caps[2].parse().ok()?),
            (
                "patch".to_string(),
                caps.get(3)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
            ),
        ],
        epoch: None,
        prerelease: None,
        build_metadata: None,
        revision: None,
    })
}

/// A `year.month[.patch]` shape is inherently ambiguous with an ordinary
/// `major.minor.patch` SemVer version whose major happens to look like a
/// two-digit year (e.g. `12.4.2`). A 4-digit year is unambiguous regardless
/// of whether a patch is present. A 2-digit year is only trusted during
/// auto-detection when there's no patch component — matching common `YY.MM`
/// tools (Ubuntu, PyCharm, ...) — since a genuine SemVer version practically
/// always carries a patch. Pass `--scheme calver` / `--scheme semver`
/// explicitly whenever this heuristic guesses wrong.
fn is_plausible_calver_autodetect(year: u64, month: u64, has_patch: bool) -> bool {
    if !(1..=12).contains(&month) {
        return false;
    }
    if year >= 1900 {
        return true;
    }
    !has_patch && (10..=39).contains(&year)
}

fn parse_integer_build(raw: &str) -> Option<LegacyVersion> {
    let trimmed = raw.trim();
    let value: u64 = trimmed.parse().ok()?;
    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::IntegerBuild,
        components: vec![("build".to_string(), value)],
        epoch: None,
        prerelease: None,
        build_metadata: None,
        revision: None,
    })
}

fn pep440_regex() -> Regex {
    Regex::new(
        r"(?x)
        ^
        (?:(?P<epoch>\d+)!)?
        (?P<major>\d+)
        (?:\.(?P<minor>\d+))?
        (?:\.(?P<patch>\d+))?
        (?:[-._]?(?P<pre_label>a|b|c|rc)(?P<pre_num>\d+))?
        (?:[-._]?post(?P<post_num>\d+))?
        (?:[-._]?dev(?P<dev_num>\d+))?
        $
        ",
    )
    .expect("valid regex")
}

fn parse_pep440(raw: &str) -> Option<LegacyVersion> {
    let caps = pep440_regex().captures(raw.trim())?;
    let epoch = caps.name("epoch").and_then(|m| m.as_str().parse().ok());
    let major: u64 = caps.name("major")?.as_str().parse().ok()?;
    let minor: u64 = caps
        .name("minor")
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    let patch: u64 = caps
        .name("patch")
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);

    let mut pre_parts = Vec::new();
    if let (Some(label), Some(num)) = (caps.name("pre_label"), caps.name("pre_num")) {
        pre_parts.push(format!("{}{}", label.as_str(), num.as_str()));
    }
    if let Some(num) = caps.name("post_num") {
        pre_parts.push(format!("post{}", num.as_str()));
    }
    if let Some(num) = caps.name("dev_num") {
        pre_parts.push(format!("dev{}", num.as_str()));
    }

    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::Pep440,
        components: vec![
            ("major".to_string(), major),
            ("minor".to_string(), minor),
            ("patch".to_string(), patch),
        ],
        epoch,
        prerelease: if pre_parts.is_empty() {
            None
        } else {
            Some(pre_parts.join("."))
        },
        build_metadata: None,
        revision: None,
    })
}

/// A bare release segment (`1.2.3`) is indistinguishable from SemVer/Maven,
/// so auto-detection only claims PEP 440 when an epoch or an explicit
/// pre/post/dev marker is present.
fn is_pep440_distinctive(raw: &str) -> bool {
    if raw.contains('!') {
        return true;
    }
    Regex::new(r"(?:^|[.\-_])(?:a|b|c|rc)\d+|post\d+|dev\d+")
        .unwrap()
        .is_match(raw)
}

fn parse_debian_rpm(raw: &str) -> Option<LegacyVersion> {
    let re = Regex::new(r"^(?:(\d+):)?([0-9][0-9A-Za-z.+~]*)(?:-([0-9A-Za-z.+~]+))?$").unwrap();
    let caps = re.captures(raw.trim())?;
    let epoch = caps.get(1).and_then(|m| m.as_str().parse().ok());
    let upstream = caps.get(2)?.as_str();
    let revision = caps.get(3).map(|m| m.as_str().to_string());

    let numeric_re = Regex::new(r"^(\d+)(?:\.(\d+))?(?:\.(\d+))?").unwrap();
    let numeric_caps = numeric_re.captures(upstream)?;

    Some(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::DebianRpm,
        components: vec![
            ("major".to_string(), numeric_caps[1].parse().ok()?),
            (
                "minor".to_string(),
                numeric_caps
                    .get(2)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
            ),
            (
                "patch".to_string(),
                numeric_caps
                    .get(3)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
            ),
        ],
        epoch,
        prerelease: None,
        build_metadata: None,
        revision,
    })
}

fn parse_custom(raw: &str, pattern: &str) -> Result<LegacyVersion> {
    let re = Regex::new(pattern).with_context(|| format!("invalid --scheme-regex `{pattern}`"))?;
    let caps = re
        .captures(raw.trim())
        .ok_or_else(|| anyhow!("--scheme-regex `{pattern}` did not match `{raw}`"))?;

    let mut components = Vec::new();
    for name in re.capture_names().flatten() {
        if let Some(m) = caps.name(name) {
            let value: u64 = m.as_str().parse().with_context(|| {
                format!(
                    "named group `{name}` matched `{}`, which is not a non-negative integer",
                    m.as_str()
                )
            })?;
            components.push((name.to_string(), value));
        }
    }
    if components.is_empty() {
        bail!("--scheme-regex `{pattern}` has no named capture groups; use (?P<arch>...) etc.");
    }

    Ok(LegacyVersion {
        raw: raw.to_string(),
        scheme: VersionScheme::Custom,
        components,
        epoch: None,
        prerelease: None,
        build_metadata: None,
        revision: None,
    })
}

// ── Axis mapping ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MvsAxis {
    Arch,
    Feat,
    Prot,
    Fix,
}

impl MvsAxis {
    pub fn as_str(&self) -> &'static str {
        match self {
            MvsAxis::Arch => "arch",
            MvsAxis::Feat => "feat",
            MvsAxis::Prot => "prot",
            MvsAxis::Fix => "fix",
        }
    }

    pub fn parse_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "arch" => Some(Self::Arch),
            "feat" => Some(Self::Feat),
            "prot" => Some(Self::Prot),
            "fix" => Some(Self::Fix),
            _ => None,
        }
    }
}

/// A `component name -> MVS axis` table, either a scheme's default (see
/// [`default_mapping_for`]) or built from a `--map` spec via [`AxisMapping::parse_spec`].
#[derive(Debug, Clone, Default)]
pub struct AxisMapping {
    entries: BTreeMap<String, MvsAxis>,
}

impl AxisMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, component: impl Into<String>, axis: MvsAxis) {
        self.entries.insert(component.into(), axis);
    }

    pub fn get(&self, component: &str) -> Option<MvsAxis> {
        self.entries.get(component).copied()
    }

    /// Parses a comma-separated `component=axis` spec, e.g.
    /// `"major=arch,minor=feat,patch=fix"`. Axis names are case-insensitive.
    pub fn parse_spec(spec: &str) -> Result<Self> {
        let mut mapping = Self::new();
        for pair in spec.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let (name, axis) = pair.split_once('=').ok_or_else(|| {
                anyhow!("invalid --map entry `{pair}`; expected `component=axis`")
            })?;
            let axis = MvsAxis::parse_name(axis.trim()).ok_or_else(|| {
                anyhow!(
                    "invalid --map axis `{}` in `{pair}`; expected one of arch, feat, prot, fix",
                    axis.trim()
                )
            })?;
            mapping.insert(name.trim().to_string(), axis);
        }
        Ok(mapping)
    }
}

/// The default `component -> axis` table for a scheme. Components not
/// listed here (DotNet4's `revision`, CalVer's `year`/`month` under the
/// default "rebase" mode) are intentionally left unmapped; override with
/// `--map`. See [`advisories`] for what each scheme's default drops.
pub fn default_mapping_for(scheme: VersionScheme) -> AxisMapping {
    let mut mapping = AxisMapping::new();
    match scheme {
        VersionScheme::Semver
        | VersionScheme::ZeroVer
        | VersionScheme::Pep440
        | VersionScheme::MavenGradle
        | VersionScheme::GoModules
        | VersionScheme::DebianRpm
        | VersionScheme::DotNet4 => {
            mapping.insert("major", MvsAxis::Arch);
            mapping.insert("minor", MvsAxis::Feat);
            mapping.insert("patch", MvsAxis::Fix);
        }
        VersionScheme::CalVer => {
            // Rebase mode (default): year/month are dropped; ARCH/FEAT stay
            // at their constant defaults from `constant_axis_defaults`.
            // Pass `--map year=arch,month=feat,patch=fix` for a
            // date-passthrough mapping instead.
            mapping.insert("patch", MvsAxis::Fix);
        }
        VersionScheme::IntegerBuild => {
            mapping.insert("build", MvsAxis::Fix);
        }
        VersionScheme::Custom => {
            for name in ["arch", "feat", "prot", "fix"] {
                if let Some(axis) = MvsAxis::parse_name(name) {
                    mapping.insert(name, axis);
                }
            }
        }
    }
    mapping
}

/// Constant axis values that hold regardless of the mapping, for schemes
/// whose source numbering doesn't carry three independent axes worth of
/// signal (a bare build counter, or CalVer's default "rebase" mode).
pub fn constant_axis_defaults(scheme: VersionScheme) -> BTreeMap<MvsAxis, u64> {
    let mut defaults = BTreeMap::new();
    if matches!(scheme, VersionScheme::IntegerBuild | VersionScheme::CalVer) {
        defaults.insert(MvsAxis::Arch, 1);
        defaults.insert(MvsAxis::Feat, 0);
    }
    defaults
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MvsAxes {
    pub arch: u64,
    pub feat: u64,
    pub prot: u64,
    pub fix: u64,
}

/// Resolves `legacy`'s components into MVS axes via `mapping`, applying the
/// scheme's constant defaults first (so an explicit mapping entry always
/// wins) and folding any PEP 440 / Debian epoch additively into `arch`.
pub fn resolve_axes(legacy: &LegacyVersion, mapping: &AxisMapping) -> MvsAxes {
    let mut axes = MvsAxes::default();
    for (axis, value) in constant_axis_defaults(legacy.scheme) {
        set_axis(&mut axes, axis, value);
    }
    for (name, value) in &legacy.components {
        if let Some(axis) = mapping.get(name) {
            set_axis(&mut axes, axis, *value);
        }
    }
    if let Some(epoch) = legacy.epoch {
        axes.arch += epoch;
    }
    axes
}

fn set_axis(axes: &mut MvsAxes, axis: MvsAxis, value: u64) {
    match axis {
        MvsAxis::Arch => axes.arch = value,
        MvsAxis::Feat => axes.feat = value,
        MvsAxis::Prot => axes.prot = value,
        MvsAxis::Fix => axes.fix = value,
    }
}

/// Human-readable notes about what a scheme's *default* mapping drops or
/// folds, so a CLI can surface them instead of silently discarding signal.
pub fn advisories(legacy: &LegacyVersion, mapping: &AxisMapping) -> Vec<String> {
    let mut notes = Vec::new();
    match legacy.scheme {
        VersionScheme::ZeroVer => notes.push(
            "ZeroVer: 0.x minor bumps are often breaking in practice; review whether PROT \
             should move too."
                .to_string(),
        ),
        VersionScheme::DotNet4 => {
            let revision_mapped = mapping.get("revision").is_some();
            if let Some((_, revision)) = legacy
                .components
                .iter()
                .find(|(name, _)| name == "revision")
            {
                if *revision != 0 && !revision_mapped {
                    notes.push(format!(
                        "DotNet4: the 4th component (`revision` = {revision}) is not mapped to \
                         any axis and was dropped; pass --map revision=<axis> if it carries meaning."
                    ));
                }
            }
        }
        VersionScheme::GoModules => notes.push(
            "GoModules: when ARCH >= 2, the module's import path must carry a matching /vN \
             suffix."
                .to_string(),
        ),
        VersionScheme::CalVer => {
            let year_or_month_mapped =
                mapping.get("year").is_some() || mapping.get("month").is_some();
            if !year_or_month_mapped {
                notes.push(
                    "CalVer: year/month are dropped under the default rebase mapping (ARCH=1, \
                     FEAT=0); pass --map year=arch,month=feat,patch=fix for a date-passthrough \
                     mapping instead."
                        .to_string(),
                );
            }
        }
        VersionScheme::Pep440 => {
            if let Some(epoch) = legacy.epoch {
                notes.push(format!(
                    "PEP 440: epoch {epoch} was folded additively into ARCH."
                ));
            }
        }
        VersionScheme::DebianRpm => {
            if let Some(epoch) = legacy.epoch {
                notes.push(format!(
                    "Debian/RPM: epoch {epoch} was folded additively into ARCH."
                ));
            }
            if let Some(revision) = &legacy.revision {
                notes.push(format!(
                    "Debian/RPM: packaging revision `-{revision}` is metadata, not a version \
                     axis, and was dropped."
                ));
            }
        }
        _ => {}
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parses_and_maps() {
        let v = parse("1.4.2-rc1+build5", VersionScheme::Semver, None).unwrap();
        assert_eq!(v.scheme, VersionScheme::Semver);
        assert_eq!(v.prerelease.as_deref(), Some("rc1"));
        assert_eq!(v.build_metadata.as_deref(), Some("build5"));
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.prot, axes.fix), (1, 4, 0, 2));
    }

    #[test]
    fn zerover_is_auto_detected_from_major_zero() {
        let v = parse_auto("0.3.1").unwrap();
        assert_eq!(v.scheme, VersionScheme::ZeroVer);
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.prot, axes.fix), (0, 3, 0, 1));
        assert!(advisories(&v, &default_mapping_for(v.scheme))[0].contains("0.x minor bumps"));
    }

    #[test]
    fn dotnet4_default_mapping_drops_revision() {
        let v = parse_auto("2.5.1.42").unwrap();
        assert_eq!(v.scheme, VersionScheme::DotNet4);
        let default_mapping = default_mapping_for(v.scheme);
        let axes = resolve_axes(&v, &default_mapping);
        assert_eq!((axes.arch, axes.feat, axes.prot, axes.fix), (2, 5, 0, 1));
        let notes = advisories(&v, &default_mapping);
        assert!(notes[0].contains("revision"));

        // Explicit override recovers it onto PROT, and the advisory no
        // longer claims it was dropped.
        let mut mapping = default_mapping_for(v.scheme);
        mapping.insert("revision", MvsAxis::Prot);
        let axes = resolve_axes(&v, &mapping);
        assert_eq!(axes.prot, 42);
        assert!(advisories(&v, &mapping).is_empty());
    }

    #[test]
    fn pep440_folds_epoch_and_captures_prerelease() {
        let v = parse_auto("1!2.3.4rc1").unwrap();
        assert_eq!(v.scheme, VersionScheme::Pep440);
        assert_eq!(v.epoch, Some(1));
        assert_eq!(v.prerelease.as_deref(), Some("rc1"));
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        // major=2 + epoch=1 folded into arch.
        assert_eq!((axes.arch, axes.feat, axes.prot, axes.fix), (3, 3, 0, 4));
    }

    #[test]
    fn pep440_bare_release_is_not_auto_detected_over_semver() {
        let v = parse_auto("2.3.4").unwrap();
        assert_eq!(v.scheme, VersionScheme::Semver);
    }

    #[test]
    fn maven_gradle_qualifier_and_two_part_are_distinctive() {
        let v = parse_auto("1.4.0-SNAPSHOT").unwrap();
        assert_eq!(v.scheme, VersionScheme::MavenGradle);
        assert_eq!(v.prerelease.as_deref(), Some("SNAPSHOT"));

        let v2 = parse_auto("1.4").unwrap();
        assert_eq!(v2.scheme, VersionScheme::MavenGradle);
        let axes = resolve_axes(&v2, &default_mapping_for(v2.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (1, 4, 0));
    }

    #[test]
    fn calver_default_mapping_rebases_and_passthrough_overrides() {
        let v = parse_auto("2024.9.3").unwrap();
        assert_eq!(v.scheme, VersionScheme::CalVer);
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (1, 0, 3));

        let mapping = AxisMapping::parse_spec("year=arch,month=feat,patch=fix").unwrap();
        let axes = resolve_axes(&v, &mapping);
        assert_eq!((axes.arch, axes.feat, axes.fix), (2024, 9, 3));
        // The "dropped under the default mapping" advisory no longer
        // applies once year/month are explicitly mapped.
        assert!(advisories(&v, &mapping).is_empty());
    }

    #[test]
    fn calver_autodetect_does_not_shadow_ordinary_semver_majors() {
        // `12.4.2` looks like a plausible 2-digit-year CalVer date (year 12,
        // month 4), but it also has a patch component, which is
        // characteristic of an ordinary SemVer version. The heuristic must
        // prefer SemVer here rather than silently rebasing a real major
        // version 12 down to ARCH=1.
        let v = parse_auto("12.4.2").unwrap();
        assert_eq!(v.scheme, VersionScheme::Semver);
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (12, 4, 2));
    }

    #[test]
    fn calver_two_digit_year_without_patch_is_detected() {
        // Ubuntu-style `YY.MM` releases are exactly the common case the
        // 2-digit-year heuristic targets.
        let v = parse_auto("24.04").unwrap();
        assert_eq!(v.scheme, VersionScheme::CalVer);
        assert_eq!(v.components[0], ("year".to_string(), 24));
        assert_eq!(v.components[1], ("month".to_string(), 4));
    }

    #[test]
    fn integer_build_maps_to_fix_with_constant_arch() {
        let v = parse_auto("1024").unwrap();
        assert_eq!(v.scheme, VersionScheme::IntegerBuild);
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (1, 0, 1024));
    }

    #[test]
    fn debian_rpm_parses_epoch_upstream_and_revision() {
        let v = parse_auto("2:1.4.2-3ubuntu1").unwrap();
        assert_eq!(v.scheme, VersionScheme::DebianRpm);
        assert_eq!(v.epoch, Some(2));
        assert_eq!(v.revision.as_deref(), Some("3ubuntu1"));
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (3, 4, 2));
        let notes = advisories(&v, &default_mapping_for(v.scheme));
        assert!(notes.iter().any(|n| n.contains("epoch 2")));
        assert!(notes.iter().any(|n| n.contains("packaging revision")));
    }

    #[test]
    fn go_modules_strips_leading_v() {
        let v = parse("v3.1.0", VersionScheme::GoModules, None).unwrap();
        assert_eq!(v.raw, "v3.1.0");
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.fix), (3, 1, 0));
        assert!(advisories(&v, &default_mapping_for(v.scheme))[0].contains("/vN"));
    }

    #[test]
    fn custom_regex_maps_named_groups_directly_to_axes() {
        let v = parse(
            "R7-F12-P3",
            VersionScheme::Custom,
            Some(r"R(?P<arch>\d+)-F(?P<feat>\d+)-P(?P<prot>\d+)"),
        )
        .unwrap();
        let axes = resolve_axes(&v, &default_mapping_for(v.scheme));
        assert_eq!((axes.arch, axes.feat, axes.prot, axes.fix), (7, 12, 3, 0));
    }

    #[test]
    fn custom_regex_without_named_groups_is_rejected() {
        let err = parse("1.2.3", VersionScheme::Custom, Some(r"(\d+)\.(\d+)\.(\d+)")).unwrap_err();
        assert!(err.to_string().contains("no named capture groups"));
    }

    #[test]
    fn axis_mapping_parse_spec_rejects_bad_axis_names() {
        let err = AxisMapping::parse_spec("major=bogus").unwrap_err();
        assert!(err.to_string().contains("invalid --map axis"));
    }

    #[test]
    fn unparseable_string_reports_a_clear_error() {
        let err = parse_auto("not-a-version").unwrap_err();
        assert!(err.to_string().contains("could not auto-detect"));
    }
}
