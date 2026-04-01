// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::prelude::*;
use mvs_manager::mvs::hashing::hash_items;
use predicates::str::contains;
use serde_json::Value;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn contract_fixtures_root() -> PathBuf {
    fixtures_root().join("contracts")
}

fn load_contract_json(name: &str) -> Value {
    let path = contract_fixtures_root().join(name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "failed to read contract fixture {}: {error}",
            path.display()
        )
    }))
    .unwrap_or_else(|error| {
        panic!(
            "contract fixture {} should be valid json: {error}",
            path.display()
        )
    })
}

fn replace_string_field(payload: &mut Value, key: &str, replacement: &str) {
    if let Some(value) = payload.get_mut(key) {
        assert!(value.is_string(), "{key} should be a string field");
        *value = Value::String(replacement.to_string());
    }
}

fn normalize_contract_output(payload: &mut Value) {
    replace_string_field(payload, "manifest_path", "<MANIFEST_PATH>");
    replace_string_field(payload, "root", "<ROOT>");
    replace_string_field(payload, "host_manifest", "<HOST_MANIFEST>");
    replace_string_field(payload, "extension_manifest", "<EXTENSION_MANIFEST>");
}

fn normalize_contract_manifest(manifest: &mut Value) {
    if let Some(history) = manifest.get_mut("history").and_then(Value::as_array_mut) {
        for entry in history {
            let object = entry
                .as_object_mut()
                .expect("history entries should be objects");
            object.insert("changed_at_unix".to_string(), Value::from(0_u64));
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("failed to create destination fixture directory");

    for entry in fs::read_dir(src).expect("failed to read fixture directory") {
        let entry = entry.expect("failed to read fixture entry");
        let file_type = entry.file_type().expect("failed to read file type");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).unwrap_or_else(|error| {
                panic!(
                    "failed to copy fixture file {} -> {}: {error}",
                    src_path.display(),
                    dst_path.display()
                )
            });
        }
    }
}

fn binary_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mvs-manager"))
}

struct TempWorkspace {
    path: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let index = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mvs-integration-test-{}-{}-{}",
            std::process::id(),
            nanos,
            index
        ));
        fs::create_dir_all(&path).expect("failed to create temp test workspace");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn generate_then_lint_passes_for_fixture_project() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");
    let ai_schema_path = project_root.join("tool_schema.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--ai-schema",
            ai_schema_path.to_str().expect("non-utf8 path"),
        ])
        .assert()
        .success()
        .stdout(contains("MVS identity:"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(generated["identity"]["cont"], "cli");
    assert!(generated["identity"]["feat"].as_u64().unwrap_or_default() >= 1);
    assert!(generated["identity"]["prot"].as_u64().unwrap_or_default() >= 1);

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
        "--ai-schema",
        ai_schema_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Lint passed"));
}

#[test]
fn lint_fails_after_public_api_drift_without_regeneration() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
        ])
        .assert()
        .success();

    let api_file = project_root.join("src/api.ts");
    let updated = format!(
        "{}\nexport function rotateToken(token: string): string {{ return token; }}\n",
        fs::read_to_string(&api_file).expect("failed to read API file")
    );
    fs::write(&api_file, updated).expect("failed to write API file drift");

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .failure()
    .stdout(contains("Public API signature drift detected"));
}

#[test]
fn validate_returns_degraded_when_legacy_shim_is_available() {
    let host = fixtures_root().join("manifests/host_with_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
        ])
        .assert()
        .success()
        .stdout(contains("Compatibility: DEGRADED"));
}

#[test]
fn validate_fails_when_protocol_out_of_range_and_no_shim() {
    let host = fixtures_root().join("manifests/host_no_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
        ])
        .assert()
        .failure()
        .stdout(contains("Compatibility: INCOMPATIBLE"));
}

#[test]
fn generate_json_reports_semantic_evidence_snapshot_counts() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    let assert = generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--format",
            "json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("generate output should be valid utf8");
    let payload: Value = serde_json::from_str(&stdout).expect("generate json output should parse");

    assert_eq!(payload["command"], "generate");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["exit_code"], 0);
    assert!(
        payload["evidence"]["feature_inventory_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );
    assert!(
        payload["evidence"]["protocol_inventory_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );
    assert!(
        payload["evidence"]["public_api_inventory_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );
}

#[test]
fn generate_json_matches_golden_contract_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");
    let ai_schema_path = project_root.join("tool_schema.json");

    let mut generate = binary_cmd();
    let assert = generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--ai-schema",
            ai_schema_path.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("generate output should be valid utf8");
    let mut payload: Value =
        serde_json::from_str(&stdout).expect("generate json output should parse");
    normalize_contract_output(&mut payload);

    assert_eq!(payload, load_contract_json("generate_cli.json"));
}

#[test]
fn generated_manifest_matches_golden_contract_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");
    let ai_schema_path = project_root.join("tool_schema.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--ai-schema",
            ai_schema_path.to_str().expect("non-utf8 path"),
        ])
        .assert()
        .success();

    let mut manifest: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    normalize_contract_manifest(&mut manifest);

    assert_eq!(manifest, load_contract_json("generator_manifest_cli.json"));
}

#[test]
fn lint_json_failure_uses_stable_exit_code() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
        ])
        .assert()
        .success();

    let api_file = project_root.join("src/api.ts");
    let updated = format!(
        "{}\nexport function rotateToken(token: string): string {{ return token; }}\n",
        fs::read_to_string(&api_file).expect("failed to read API file")
    );
    fs::write(&api_file, updated).expect("failed to write API file drift");

    let mut lint = binary_cmd();
    let assert = lint
        .args([
            "lint",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .code(20);

    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("lint output should be utf8");
    let payload: Value = serde_json::from_str(&stdout).expect("lint json output should parse");

    assert_eq!(payload["command"], "lint");
    assert_eq!(payload["status"], "failed");
    assert_eq!(payload["exit_code"], 20);
    assert!(payload["failure_count"].as_u64().unwrap_or_default() >= 1);
}

#[test]
fn lint_json_failure_matches_golden_contract_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
        ])
        .assert()
        .success();

    let api_file = project_root.join("src/api.ts");
    let updated = format!(
        "{}\nexport function rotateToken(token: string): string {{ return token; }}\n",
        fs::read_to_string(&api_file).expect("failed to read API file")
    );
    fs::write(&api_file, updated).expect("failed to write API file drift");

    let mut lint = binary_cmd();
    let assert = lint
        .args([
            "lint",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .code(20);

    let stdout =
        String::from_utf8(assert.get_output().stdout.clone()).expect("lint output should be utf8");
    let mut payload: Value = serde_json::from_str(&stdout).expect("lint json output should parse");
    normalize_contract_output(&mut payload);

    assert_eq!(payload, load_contract_json("lint_public_api_drift.json"));
}

#[test]
fn validate_json_failure_uses_stable_exit_code() {
    let host = fixtures_root().join("manifests/host_no_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    let assert = validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .code(30);

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("validate output should be valid utf8");
    let payload: Value = serde_json::from_str(&stdout).expect("validate json output should parse");

    assert_eq!(payload["command"], "validate");
    assert_eq!(payload["status"], "incompatible");
    assert_eq!(payload["exit_code"], 30);
    assert_eq!(payload["compatible"], false);
    assert_eq!(payload["failure_count"], 1);
    assert_eq!(payload["degraded_count"], 0);
    let checks = payload["checks"]
        .as_array()
        .expect("validate checks should be an array");
    assert!(checks.iter().any(|check| {
        check["axis"].as_str().expect("axis should be a string") == "protocol"
            && check["status"].as_str().expect("status should be a string") == "fail"
            && check["code"].as_str().expect("code should be a string") == "protocol_range_mismatch"
    }));
}

#[test]
fn validate_json_incompatible_matches_golden_contract_fixture() {
    let host = fixtures_root().join("manifests/host_no_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    let assert = validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .code(30);

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("validate output should be valid utf8");
    let mut payload: Value =
        serde_json::from_str(&stdout).expect("validate json output should parse");
    normalize_contract_output(&mut payload);

    assert_eq!(payload, load_contract_json("validate_incompatible.json"));
}

#[test]
fn validate_json_degraded_reports_shimmed_protocol_axis() {
    let host = fixtures_root().join("manifests/host_with_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    let assert = validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("validate output should be valid utf8");
    let payload: Value = serde_json::from_str(&stdout).expect("validate json output should parse");

    assert_eq!(payload["command"], "validate");
    assert_eq!(payload["status"], "degraded");
    assert_eq!(payload["exit_code"], 0);
    assert_eq!(payload["compatible"], true);
    assert_eq!(payload["degraded"], true);
    assert_eq!(payload["failure_count"], 0);
    assert_eq!(payload["degraded_count"], 1);
    let checks = payload["checks"]
        .as_array()
        .expect("validate checks should be an array");
    assert!(checks.iter().any(|check| {
        check["axis"].as_str().expect("axis should be a string") == "protocol"
            && check["status"].as_str().expect("status should be a string") == "degraded"
            && check["code"].as_str().expect("code should be a string") == "protocol_range_shimmed"
    }));
}

#[test]
fn validate_json_degraded_matches_golden_contract_fixture() {
    let host = fixtures_root().join("manifests/host_with_shim.json");
    let extension = fixtures_root().join("manifests/extension_out_of_range.json");

    let mut validate = binary_cmd();
    let assert = validate
        .args([
            "validate",
            "--host-manifest",
            host.to_str().expect("non-utf8 path"),
            "--extension-manifest",
            extension.to_str().expect("non-utf8 path"),
            "--format",
            "json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("validate output should be valid utf8");
    let mut payload: Value =
        serde_json::from_str(&stdout).expect("validate json output should parse");
    normalize_contract_output(&mut payload);

    assert_eq!(payload, load_contract_json("validate_degraded.json"));
}

#[test]
fn scoped_public_api_root_ignores_internal_public_api_drift() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/api.ts",
        ])
        .assert()
        .success();

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["public_api_roots"][0],
        "src/api.ts"
    );

    let lib_file = project_root.join("src/lib.rs");
    let updated = format!(
        "{}\npub fn internal_probe(seed: u64) -> u64 {{ seed + 1 }}\n",
        fs::read_to_string(&lib_file).expect("failed to read lib file")
    );
    fs::write(&lib_file, updated).expect("failed to write internal api drift");

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Lint passed"));
}

#[test]
fn public_api_include_filters_persist_and_ignore_non_contract_drift() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/api.ts",
            "--public-api-include",
            "ts/js:function login*",
        ])
        .assert()
        .success();

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["public_api_includes"][0],
        "ts/js:function login*"
    );
    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert_eq!(inventory.len(), 1);
    assert!(inventory[0]["signature"]
        .as_str()
        .expect("signature should be a string")
        .starts_with("ts/js:function login("));

    let api_file = project_root.join("src/api.ts");
    let updated = fs::read_to_string(&api_file)
        .expect("failed to read api file")
        .replace(
            "export interface Session",
            "export interface SessionPayload",
        )
        .replace("export const buildSession", "export const buildSessionV2");
    fs::write(&api_file, updated).expect("failed to write non-contract drift");

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Lint passed"))
    .stdout(contains("Public API includes"));
}

#[test]
fn ts_export_following_persists_and_resolves_relative_barrels() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create src");

    fs::write(
        project_root.join("src/auth.ts"),
        r#"
        export function login(username: string): string {
          return username;
        }

        export interface Session {
          token: string;
        }
    "#,
    )
    .expect("failed to write auth fixture");

    fs::write(
        project_root.join("src/index.ts"),
        r#"
        export { login as authenticate, Session as ActiveSession } from "./auth";
        export * from "./auth";
    "#,
    )
    .expect("failed to write barrel fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/index.ts",
            "--ts-export-following",
            "relative-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["ts_export_following"],
        "relative_only"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:function authenticate(username: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:interface ActiveSession"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export * from \"./auth\""
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("TS/JS export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn ts_export_following_workspace_only_persists_and_resolves_workspace_facades() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create src");

    fs::write(
        project_root.join("package.json"),
        r#"
        {
          "name": "@demo/sdk",
          "exports": {
            "./auth": "./src/auth.ts"
          }
        }
    "#,
    )
    .expect("failed to write package.json");

    fs::write(
        project_root.join("tsconfig.json"),
        r#"
        {
          "compilerOptions": {
            "baseUrl": ".",
            "paths": {
              "@/*": ["src/*"]
            }
          }
        }
    "#,
    )
    .expect("failed to write tsconfig.json");

    fs::write(
        project_root.join("src/auth.ts"),
        r#"
        export function login(username: string): string {
          return username;
        }
    "#,
    )
    .expect("failed to write auth fixture");

    fs::write(
        project_root.join("src/session.ts"),
        r#"
        export interface Session {
          token: string;
        }
    "#,
    )
    .expect("failed to write session fixture");

    fs::write(
        project_root.join("src/index.ts"),
        r#"
        export { login as authenticate } from "@demo/sdk/auth";
        export * from "@/session";
    "#,
    )
    .expect("failed to write barrel fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/index.ts",
            "--ts-export-following",
            "workspace-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["ts_export_following"],
        "workspace_only"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:function authenticate(username: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:interface Session"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export * from \"@/session\""
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("TS/JS export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn ts_export_following_workspace_only_prefers_conditioned_root_and_wildcard_exports() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src/features")).expect("failed to create src/features");
    fs::create_dir_all(project_root.join("dist/features")).expect("failed to create dist/features");

    fs::write(
        project_root.join("package.json"),
        r#"
        {
          "name": "@demo/sdk",
          "exports": {
            ".": {
              "default": "./dist/index.js",
              "import": "./src/root.ts"
            },
            "./features/*": {
              "default": "./dist/features/*.js",
              "import": "./src/features/*.ts"
            }
          }
        }
    "#,
    )
    .expect("failed to write package.json");

    fs::write(
        project_root.join("src/root.ts"),
        r#"
        export function createKit(name: string): string {
          return name;
        }
    "#,
    )
    .expect("failed to write root fixture");

    fs::write(
        project_root.join("src/features/session.ts"),
        r#"
        export interface SessionFeature {
          token: string;
        }
    "#,
    )
    .expect("failed to write feature fixture");

    fs::write(
        project_root.join("src/index.ts"),
        r#"
        export { createKit } from "@demo/sdk";
        export * from "@demo/sdk/features/session";
    "#,
    )
    .expect("failed to write barrel fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/index.ts",
            "--ts-export-following",
            "workspace-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:function createKit(name: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:interface SessionFeature"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("dist/index.js")
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("dist/features")
    }));
}

#[test]
fn ts_export_following_workspace_only_resolves_package_imports_maps() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src/internal/features"))
        .expect("failed to create src/internal/features");
    fs::create_dir_all(project_root.join("dist/internal/features"))
        .expect("failed to create dist/internal/features");

    fs::write(
        project_root.join("package.json"),
        r##"
        {
          "name": "@demo/sdk",
          "imports": {
            "#core": {
              "default": "./dist/internal/core.js",
              "import": "./src/internal/core.ts"
            },
            "#features/*": {
              "default": "./dist/internal/features/*.js",
              "import": "./src/internal/features/*.ts"
            }
          }
        }
    "##,
    )
    .expect("failed to write package.json");

    fs::write(
        project_root.join("src/internal/core.ts"),
        r#"
        export function boot(target: string): string {
          return target;
        }
    "#,
    )
    .expect("failed to write core fixture");

    fs::write(
        project_root.join("src/internal/features/session.ts"),
        r#"
        export interface InternalSession {
          token: string;
        }
    "#,
    )
    .expect("failed to write feature fixture");

    fs::write(
        project_root.join("src/index.ts"),
        r##"
        export { boot } from "#core";
        export * from "#features/session";
    "##,
    )
    .expect("failed to write barrel fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/index.ts",
            "--ts-export-following",
            "workspace-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:function boot(target: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:interface InternalSession"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export { boot } from \"#core\""
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export * from \"#features/session\""
    }));
}

#[test]
fn ts_export_following_workspace_only_resolves_monorepo_package_self_references() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("packages/sdk/src"))
        .expect("failed to create packages/sdk/src");

    fs::write(
        project_root.join("package.json"),
        r#"
        {
          "private": true,
          "workspaces": ["packages/*"]
        }
    "#,
    )
    .expect("failed to write monorepo package.json");

    fs::write(
        project_root.join("packages/sdk/package.json"),
        r#"
        {
          "name": "@demo/sdk",
          "exports": {
            "./auth": "./src/auth.ts"
          }
        }
    "#,
    )
    .expect("failed to write package fixture");

    fs::write(
        project_root.join("packages/sdk/src/auth.ts"),
        r#"
        export function login(username: string): string {
          return username;
        }
    "#,
    )
    .expect("failed to write auth fixture");

    fs::write(
        project_root.join("packages/sdk/src/index.ts"),
        r#"
        export { login as authenticate } from "@demo/sdk/auth";
    "#,
    )
    .expect("failed to write index fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "packages/sdk/src/index.ts",
            "--ts-export-following",
            "workspace-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["ts_export_following"],
        "workspace_only"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "packages/sdk/src/index.ts"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "ts/js:function authenticate(username: string): string"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("TS/JS export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn ts_export_following_workspace_only_covers_release_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("release_ts_workspace");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "packages/sdk/src/index.ts",
            "--ts-export-following",
            "workspace-only",
        ])
        .assert()
        .success()
        .stdout(contains("TS/JS export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["ts_export_following"],
        "workspace_only"
    );
    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "packages/sdk/src/index.ts"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "ts/js:function authenticate(username: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "packages/sdk/src/index.ts"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "ts/js:interface Session"
    }));
    assert!(!inventory.iter().any(|entry| {
        let signature = entry["signature"]
            .as_str()
            .expect("signature should be a string");
        signature == "ts/js:export { login as authenticate } from \"@demo/sdk/auth\""
            || signature == "ts/js:export * from \"#session\""
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("TS/JS export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn go_export_following_package_only_persists_and_expands_package_siblings() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create src");

    fs::write(
        project_root.join("src/api.go"),
        r#"
        package demo

        func Connect(target string) error {
            return nil
        }
    "#,
    )
    .expect("failed to write go api fixture");

    fs::write(
        project_root.join("src/types.go"),
        r#"
        package demo

        type Session struct {
            Token string
        }
    "#,
    )
    .expect("failed to write go types fixture");

    fs::write(
        project_root.join("src/api_test.go"),
        r#"
        package demo

        const TestHelper string = "ignored"
    "#,
    )
    .expect("failed to write go test fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/api.go",
            "--go-export-following",
            "package-only",
        ])
        .assert()
        .success()
        .stdout(contains("Go export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["go_export_following"],
        "package_only"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/types.go"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "go:type Session struct"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("TestHelper")
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Go export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn rust_export_following_public_modules_persists_and_expands_rooted_lib_rs_modules() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src/facade")).expect("failed to create rust facade dir");

    fs::write(
        project_root.join("src/lib.rs"),
        r#"
        pub fn handshake(version: u32) -> bool { version > 0 }

        pub mod api;
        mod internal;

        pub mod facade {
            pub mod http;
        }
    "#,
    )
    .expect("failed to write rust root fixture");

    fs::write(
        project_root.join("src/api.rs"),
        r#"
        pub struct Session;
    "#,
    )
    .expect("failed to write rust api fixture");

    fs::write(
        project_root.join("src/internal.rs"),
        r#"
        pub struct Hidden;
    "#,
    )
    .expect("failed to write rust internal fixture");

    fs::write(
        project_root.join("src/facade/http.rs"),
        r#"
        pub fn respond(status: u16) -> bool { status > 0 }
    "#,
    )
    .expect("failed to write rust nested fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/lib.rs",
            "--rust-export-following",
            "public-modules",
        ])
        .assert()
        .success()
        .stdout(contains("Rust export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["rust_export_following"],
        "public_modules"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/api.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct Session"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/facade/http.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn respond(status: u16) -> bool"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/internal.rs"
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Rust export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn rust_export_following_public_modules_resolves_direct_pub_use_facades() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create rust src dir");

    fs::write(
        project_root.join("src/lib.rs"),
        r#"
        pub use internal::{Hidden as Visible, connect as open};

        mod internal;
    "#,
    )
    .expect("failed to write rust root fixture");

    fs::write(
        project_root.join("src/internal.rs"),
        r#"
        pub struct Hidden;

        impl Hidden {
            pub fn ping(&self) -> bool { true }
        }

        pub fn connect(target: u32) -> bool { target > 0 }
    "#,
    )
    .expect("failed to write rust internal fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/lib.rs",
            "--rust-export-following",
            "public-modules",
        ])
        .assert()
        .success()
        .stdout(contains("Rust export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct Visible"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:impl-fn Visible::ping(&self) -> bool"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn open(target: u32) -> bool"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("Hidden")
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/internal.rs"
    }));
}

#[test]
fn rust_export_following_public_modules_resolves_chained_pub_use_facades() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create rust src dir");

    fs::write(
        project_root.join("src/lib.rs"),
        r#"
        pub use facade::{PublicSession as Session, open as connect};
        pub use facade::*;

        pub mod facade;
        mod internal;
    "#,
    )
    .expect("failed to write rust root fixture");

    fs::write(
        project_root.join("src/facade.rs"),
        r#"
        pub use crate::internal::{Hidden as PublicSession, start as open};
    "#,
    )
    .expect("failed to write rust facade fixture");

    fs::write(
        project_root.join("src/internal.rs"),
        r#"
        pub struct Hidden;

        impl Hidden {
            pub fn ping(&self) -> bool { true }
        }

        pub fn start(target: u32) -> bool { target > 0 }
    "#,
    )
    .expect("failed to write rust internal fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/lib.rs",
            "--rust-export-following",
            "public-modules",
        ])
        .assert()
        .success()
        .stdout(contains("Rust export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct Session"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:impl-fn Session::ping(&self) -> bool"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn connect(target: u32) -> bool"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct PublicSession"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn open(target: u32) -> bool"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("Hidden")
    }));
}

#[test]
fn rust_workspace_members_persist_and_resolve_cross_crate_reexports() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("app/src")).expect("failed to create app/src");
    fs::create_dir_all(project_root.join("shared/src")).expect("failed to create shared/src");

    fs::write(
        project_root.join("app/Cargo.toml"),
        r#"
        [package]
        name = "app"
    "#,
    )
    .expect("failed to write app Cargo.toml");

    fs::write(
        project_root.join("shared/Cargo.toml"),
        r#"
        [package]
        name = "shared-contract"
    "#,
    )
    .expect("failed to write shared Cargo.toml");

    fs::write(
        project_root.join("app/src/lib.rs"),
        r#"
        pub use shared_contract::{Hidden as Session, connect as open};
    "#,
    )
    .expect("failed to write app rust root fixture");

    fs::write(
        project_root.join("shared/src/lib.rs"),
        r#"
        pub struct Hidden;

        impl Hidden {
            pub fn ping(&self) -> bool { true }
        }

        pub fn connect(target: u32) -> bool { target > 0 }
    "#,
    )
    .expect("failed to write shared rust fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "app/src/lib.rs",
            "--rust-export-following",
            "public-modules",
            "--rust-workspace-member",
            "shared",
        ])
        .assert()
        .success()
        .stdout(contains("Rust export following"))
        .stdout(contains("Rust workspace members"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["rust_workspace_members"][0],
        "shared"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct Session"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:impl-fn Session::ping(&self) -> bool"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn open(target: u32) -> bool"
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Rust export following"))
    .stdout(contains("Rust workspace members"))
    .stdout(contains("Lint passed"));
}

#[test]
fn rust_workspace_members_cover_deeper_workspace_facade_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("release_rust_workspace");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "app/src/lib.rs",
            "--rust-export-following",
            "public-modules",
            "--rust-workspace-member",
            "sdk",
            "--rust-workspace-member",
            "shared",
        ])
        .assert()
        .success()
        .stdout(contains("Rust export following"))
        .stdout(contains("Rust workspace members"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    let members = generated["scan_policy"]["rust_workspace_members"]
        .as_array()
        .expect("workspace members should be an array");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0], "sdk");
    assert_eq!(members[1], "shared");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:struct PublicSession"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:impl-fn PublicSession::ping(&self) -> bool"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/src/lib.rs"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "rust:fn open(target: u32) -> bool"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("Hidden")
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Rust export following"))
    .stdout(contains("Rust workspace members"))
    .stdout(contains("Lint passed"));
}

#[test]
fn python_module_roots_persist_and_enable_cross_module_python_exports() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("app/pkg")).expect("failed to create python package");

    fs::write(
        project_root.join("app/pkg/core.py"),
        r#"
        __all__ = ("login",)

        def login(username: str) -> str:
            return username
    "#,
    )
    .expect("failed to write python core fixture");

    fs::write(
        project_root.join("app/api.py"),
        r#"
        from pkg.core import __all__ as CORE_EXPORTS
        from pkg.core import *

        __all__ = CORE_EXPORTS
    "#,
    )
    .expect("failed to write python facade fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--python-export-following",
            "roots-only",
            "--python-module-root",
            "app",
        ])
        .assert()
        .success()
        .stdout(contains("Python export following"))
        .stdout(contains("Python module roots"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(
        generated["scan_policy"]["python_export_following"],
        "roots_only"
    );
    assert_eq!(generated["scan_policy"]["python_module_roots"][0], "app");
    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "python:from pkg.core import login"
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Python export following"))
    .stdout(contains("Python module roots"))
    .stdout(contains("Lint passed"));
}

#[test]
fn python_module_roots_cover_multi_root_release_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("release_python_multi_root");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "app/api.py",
            "--python-export-following",
            "roots-only",
            "--python-module-root",
            "app",
            "--python-module-root",
            "services",
        ])
        .assert()
        .success()
        .stdout(contains("Python export following"))
        .stdout(contains("Python module roots"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    let roots = generated["scan_policy"]["python_module_roots"]
        .as_array()
        .expect("python module roots should be an array");
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], "app");
    assert_eq!(roots[1], "services");

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/api.py"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "python:from service_api import authorize"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/api.py"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "python:from service_api import SessionToken"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "app/api.py"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "python:def login(username: str) -> str"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "services/internal/auth.py"
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Python export following"))
    .stdout(contains("Python module roots"))
    .stdout(contains("Lint passed"));
}

#[test]
fn ruby_and_lua_export_following_modes_persist_and_shape_runtime_exports() {
    let temp = TempWorkspace::new();
    let project_root = temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).expect("failed to create src");

    fs::write(
        project_root.join("src/api.rb"),
        r#"
        module Demo
          SECRET = "hidden"
          private_constant :SECRET

          module_function

          def build(token)
            token
          end
        end
    "#,
    )
    .expect("failed to write ruby fixture");

    fs::write(
        project_root.join("src/api.luau"),
        r#"
        export type Session = {
            token: string,
        }

        function connect(target: string): boolean
            return target ~= ""
        end
    "#,
    )
    .expect("failed to write luau fixture");

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--ruby-export-following",
            "off",
            "--lua-export-following",
            "returned-root-only",
        ])
        .assert()
        .success()
        .stdout(contains("Lua export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert_eq!(generated["scan_policy"]["ruby_export_following"], "off");
    assert_eq!(
        generated["scan_policy"]["lua_export_following"],
        "returned_root_only"
    );

    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ruby:const Demo::SECRET"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ruby:def Demo#build(token)"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "ruby:def Demo.build(token)"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "luau:export type Session={ token: string }"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            == "luau:function connect(target: string): boolean"
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Ruby export following"))
    .stdout(contains("Lua export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn ruby_and_lua_export_following_cover_release_fixture() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("release_ruby_luau");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--ruby-export-following",
            "heuristic",
            "--lua-export-following",
            "returned-root-only",
        ])
        .assert()
        .success()
        .stdout(contains("Lua export following"));

    let generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    assert!(generated["scan_policy"]["ruby_export_following"].is_null());
    assert_eq!(
        generated["scan_policy"]["lua_export_following"],
        "returned_root_only"
    );
    let inventory = generated["evidence"]["public_api_inventory"]
        .as_array()
        .expect("public api inventory should be an array");
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/api.rb"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "ruby:def Demo#build(token)"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("SECRET")
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/runtime.luau"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "luau:function Runtime.connect(target: string): boolean"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/runtime.luau"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "luau:function Runtime:refresh(token: string): string"
    }));
    assert!(inventory.iter().any(|entry| {
        entry["file"].as_str().expect("file should be a string") == "src/runtime.luau"
            && entry["signature"]
                .as_str()
                .expect("signature should be a string")
                == "luau:export type Session={ token: string }"
    }));
    assert!(!inventory.iter().any(|entry| {
        entry["signature"]
            .as_str()
            .expect("signature should be a string")
            .contains("leak")
    }));

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Lua export following"))
    .stdout(contains("Lint passed"));
}

#[test]
fn lint_accepts_legacy_rust_signature_format_and_generate_rewrites_it() {
    let temp = TempWorkspace::new();
    let fixture_project = fixtures_root().join("generator_project");
    let project_root = temp.path().join("project");
    copy_dir_recursive(&fixture_project, &project_root);

    let manifest_path = temp.path().join("mvs.json");

    let mut generate = binary_cmd();
    generate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/lib.rs",
            "--public-api-include",
            "rust:fn *",
        ])
        .assert()
        .success();

    let mut generated: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read generated manifest"),
    )
    .expect("generated manifest should be valid JSON");
    let legacy_signature = "rust:fn fn handshake(version: u32) -> bool";
    generated["evidence"]["public_api_inventory"] = serde_json::json!([
        {
            "file": "src/lib.rs",
            "signature": legacy_signature
        }
    ]);
    generated["evidence"]["public_api_hash"] =
        Value::String(hash_items([format!("src/lib.rs|{legacy_signature}")]));

    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&generated).expect("failed to serialize manifest")
        ),
    )
    .expect("failed to write legacy-format manifest");

    let mut lint = binary_cmd();
    lint.args([
        "lint",
        "--root",
        project_root.to_str().expect("non-utf8 path"),
        "--manifest",
        manifest_path.to_str().expect("non-utf8 path"),
    ])
    .assert()
    .success()
    .stdout(contains("Lint passed"));

    let mut regenerate = binary_cmd();
    regenerate
        .args([
            "generate",
            "--root",
            project_root.to_str().expect("non-utf8 path"),
            "--manifest",
            manifest_path.to_str().expect("non-utf8 path"),
            "--context",
            "cli",
            "--public-api-root",
            "src/lib.rs",
            "--public-api-include",
            "rust:fn *",
        ])
        .assert()
        .success();

    let rewritten: Value = serde_json::from_str(
        &fs::read_to_string(&manifest_path).expect("failed to read rewritten manifest"),
    )
    .expect("rewritten manifest should be valid JSON");
    assert_eq!(rewritten["identity"]["mvs"], "0.1.1-cli");
    assert_eq!(
        rewritten["evidence"]["public_api_inventory"][0]["signature"],
        "rust:fn handshake(version: u32) -> bool"
    );
}
