// SPDX-License-Identifier: AGPL-3.0-only
//! WASM bindings for `mvs-core`'s version-scheme translation, so a browser
//! (or Node) can run the exact same conversion `mvs-manager convert-version`
//! does, without shelling out to the CLI. Mirrors
//! `mvs-manager`'s `commands/convert_version.rs` logic against `mvs-core`
//! directly; kept as a thin, independent layer so a bug in one doesn't
//! silently mask a bug in the other — they're cross-checked by the fact that
//! both call the same `mvs-core` functions.

use serde::Serialize;
use wasm_bindgen::prelude::*;

use mvs_core::manifest::Identity;
use mvs_core::schemes::{self, AxisMapping, VersionScheme};

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Converts `input` (a legacy version string) into an MVS identity.
///
/// `scheme` forces a specific scheme (`"semver"`, `"zerover"`, `"pep440"`,
/// `"maven-gradle"`, `"dotnet4"`, `"go-modules"`, `"calver"`, `"integer"`,
/// `"debian-rpm"`, `"custom"`) instead of auto-detecting one; pass `None`/
/// `undefined` to auto-detect. `scheme_regex` is required when `scheme` is
/// `"custom"`. `map_spec` (e.g. `"major=arch,minor=feat,patch=fix"`)
/// replaces the scheme's default component->axis mapping. `prot` overrides
/// the resolved PROT axis. `context` sets the identity's deployment-context
/// suffix (defaults to `"cli"`).
///
/// Returns a JSON string with the same shape as
/// `mvs-manager convert-version --format json`'s success path, or throws a
/// JS `Error` with the same message `mvs-manager` would print on failure.
/// The `#[wasm_bindgen]` boundary is kept as thin as possible — everything
/// that can be tested with a plain native `#[test]` (no JS host required)
/// lives in [`convert_version_inner`], returning a plain `Result<_, String>`.
/// This function only adapts that into wasm-bindgen's `JsError`/JSON-string
/// FFI shape.
#[wasm_bindgen(js_name = convertVersion)]
pub fn convert_version(
    input: &str,
    scheme: Option<String>,
    scheme_regex: Option<String>,
    map_spec: Option<String>,
    prot: Option<u32>,
    context: Option<String>,
) -> Result<String, JsError> {
    let result = convert_version_inner(input, scheme, scheme_regex, map_spec, prot, context)
        .map_err(|message| JsError::new(&message))?;
    serde_json::to_string(&result)
        .map_err(|error| JsError::new(&format!("failed to serialize result: {error}")))
}

fn convert_version_inner(
    input: &str,
    scheme: Option<String>,
    scheme_regex: Option<String>,
    map_spec: Option<String>,
    prot: Option<u32>,
    context: Option<String>,
) -> Result<ConvertVersionResult, String> {
    let context = context.unwrap_or_else(|| "cli".to_string());

    let forced_scheme = match &scheme {
        Some(name) => Some(VersionScheme::parse_name(name).ok_or_else(|| {
            format!(
                "unknown scheme `{name}`; expected one of semver, zerover, pep440, \
                 maven-gradle, dotnet4, go-modules, calver, integer, debian-rpm, custom"
            )
        })?),
        None => None,
    };

    let legacy = match forced_scheme {
        Some(scheme) => schemes::parse(input, scheme, scheme_regex.as_deref())
            .map_err(|error| format!("{error:#}"))?,
        None => schemes::parse_auto(input).map_err(|error| format!("{error:#}"))?,
    };

    let (mapping, mapping_source): (AxisMapping, &'static str) = match &map_spec {
        Some(spec) => (
            AxisMapping::parse_spec(spec).map_err(|error| format!("{error:#}"))?,
            "custom",
        ),
        None => (schemes::default_mapping_for(legacy.scheme), "default"),
    };

    let mut axes = schemes::resolve_axes(&legacy, &mapping);
    if let Some(prot) = prot {
        axes.prot = u64::from(prot);
    }

    let identity = Identity::format_mvs(axes.arch, axes.feat, axes.prot, axes.fix, &context);
    let semver_projection = Identity::semver_projection(axes.arch, axes.feat, axes.fix);

    Ok(ConvertVersionResult {
        input: input.to_string(),
        scheme: legacy.scheme.as_str(),
        scheme_forced: forced_scheme.is_some(),
        components: legacy
            .components
            .iter()
            .map(|(name, value)| ComponentValue {
                name: name.clone(),
                value: *value,
            })
            .collect(),
        epoch: legacy.epoch,
        prerelease: legacy.prerelease.clone(),
        build_metadata: legacy.build_metadata.clone(),
        revision: legacy.revision.clone(),
        mapping_source,
        axes: AxesPayload {
            arch: axes.arch,
            feat: axes.feat,
            prot: axes.prot,
            fix: axes.fix,
        },
        identity,
        semver_projection,
        advisories: schemes::advisories(&legacy, &mapping),
    })
}

#[derive(Debug, Serialize)]
struct ConvertVersionResult {
    input: String,
    scheme: &'static str,
    scheme_forced: bool,
    components: Vec<ComponentValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prerelease: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    build_metadata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    mapping_source: &'static str,
    axes: AxesPayload,
    identity: String,
    semver_projection: String,
    advisories: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ComponentValue {
    name: String,
    value: u64,
}

#[derive(Debug, Serialize)]
struct AxesPayload {
    arch: u64,
    feat: u64,
    prot: u64,
    fix: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_plain_semver_string() {
        let result =
            convert_version_inner("1.4.2", None, None, None, None, Some("cli".to_string()))
                .expect("conversion should succeed");
        assert_eq!(result.scheme, "semver");
        assert_eq!(result.identity, "1.4.0.2-cli");
        assert_eq!(result.semver_projection, "1.4.2");
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"scheme\":\"semver\""));
    }

    #[test]
    fn rejects_an_unknown_scheme_name() {
        let error =
            convert_version_inner("1.0.0", Some("bogus".to_string()), None, None, None, None)
                .unwrap_err();
        assert!(error.contains("unknown scheme"));
    }

    #[test]
    fn calver_map_spec_overrides_the_default_rebase_mapping() {
        let result = convert_version_inner(
            "24.04",
            None,
            None,
            Some("year=arch,month=feat,patch=fix".to_string()),
            None,
            None,
        )
        .expect("conversion should succeed");
        assert_eq!(result.scheme, "calver");
        assert_eq!(result.axes.arch, 24);
        assert_eq!(result.axes.feat, 4);
    }
}
