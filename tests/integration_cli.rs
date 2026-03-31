// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use assert_cmd::prelude::*;
use predicates::str::contains;
use serde_json::Value;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
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
