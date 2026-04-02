# 1.x Public Contract

This document defines the public compatibility contract that `mvs-manager` intends to keep stable across `1.x` releases.

The goal is simple:

- manifests remain readable and writable across `1.x`
- command JSON remains safe for automation
- canonical evidence inventories remain stable enough to drive release gates

Golden fixtures under `tests/fixtures/contracts/` are the review gate for this contract. Any intentional contract change should update those fixtures, this document, and the release notes together.

## Stability Rules

- `1.x` favors additive changes over shape changes.
- Existing manifest fields, JSON fields, exit codes, and scan-policy modes should not be removed or redefined within `1.x`.
- New optional fields may be added within `1.x` when they do not change the meaning of existing fields.
- Canonical inventory signature formats are treated as stable `1.x` surface area. Any signature rewrite requires an explicit migration note and regenerated golden fixtures.

## Stable Manifest Surface

The following top-level `mvs.json` sections are part of the stable `1.x` manifest contract:

- `$schema`
- `identity`
- `compatibility`
- `capabilities`
- `ai_contract`
- `environment`
- `evidence`
- `history`
- `scan_policy`

### `identity`

Stable fields:

- `mvs`
- `arch`
- `feat`
- `prot`
- `cont`

### `compatibility`

Stable fields:

- `host_range.min_prot`
- `host_range.max_prot`
- `extension_range.min_prot`
- `extension_range.max_prot`
- `legacy_shims[].from_prot`
- `legacy_shims[].to_prot`
- `legacy_shims[].adapter`

### `ai_contract`

Stable fields:

- `tool_schema_version`
- `tool_schema_hash`
- `required_model_capabilities`
- `provided_model_capabilities`
- `prompt_contract_id`

### `environment`

Stable fields:

- `profiles`
- `runtime_constraints`

### `evidence`

Stable fields:

- `feature_hash`
- `protocol_hash`
- `public_api_hash`
- `feature_inventory`
- `protocol_inventory`
- `public_api_inventory[].file`
- `public_api_inventory[].signature`

Contract rules:

- inventories are sorted and deduplicated
- `public_api_inventory` entries use canonical relative file paths plus canonical signatures
- canonical signatures are stable within `1.x` for supported languages

### `history`

Stable fields:

- `mvs`
- `arch`
- `feat`
- `prot`
- `cont`
- `reasons`
- `changed_at_unix`

`changed_at_unix` is intentionally runtime-derived. Its presence and meaning are stable, but its value is expected to vary across regenerations.

### `scan_policy`

Stable `1.x` keys:

- `public_api_roots`
- `public_api_includes`
- `public_api_excludes`
- `exclude_paths`
- `ts_export_following`
- `go_export_following`
- `rust_export_following`
- `rust_workspace_members`
- `python_export_following`
- `python_module_roots`
- `ruby_export_following`
- `lua_export_following`

Stable mode values:

- `ts_export_following`: `off`, `relative_only`, `workspace_only`
- `go_export_following`: `off`, `package_only`
- `rust_export_following`: `off`, `public_modules`
- `python_export_following`: `off`, `roots_only`, `heuristic`
- `ruby_export_following`: `off`, `heuristic`
- `lua_export_following`: `off`, `returned_root_only`, `heuristic`

## Stable Command JSON

All JSON command outputs include these stable top-level fields:

- `command`
- `status`
- `exit_code`

### `generate --format json`

Stable fields:

- `manifest_path`
- `root`
- `context`
- `dry_run`
- `manifest_written`
- `range_strategy`
- `scan_policy`
- `identity`
- `reasons`
- `evidence`

Stable `identity` fields:

- `previous`
- `current`
- `arch_increment`
- `feat_increment`
- `prot_increment`

Stable `evidence` fields:

- `feature_hash`
- `protocol_hash`
- `public_api_hash`
- `feature_inventory_count`
- `protocol_inventory_count`
- `public_api_inventory_count`
- `diff.features.added`
- `diff.features.removed`
- `diff.protocols.added`
- `diff.protocols.removed`
- `diff.public_api.added`
- `diff.public_api.removed`

### `lint --format json`

Stable fields:

- `manifest_path`
- `root`
- `scan_policy`
- `failure_count`
- `failures`
- `evidence`

Stable `evidence` fields match the `generate` command evidence summary shape.

Optional stable fields when boundary-shaping scan policy is active:

- `boundary_debug.included_count`
- `boundary_debug.excluded_count`
- `boundary_debug.included[].file`
- `boundary_debug.included[].signature`
- `boundary_debug.included[].included`
- `boundary_debug.included[].file_reason`
- `boundary_debug.included[].file_rule`
- `boundary_debug.included[].item_reason`
- `boundary_debug.included[].item_rule`
- `boundary_debug.excluded_path_count`
- `boundary_debug.excluded_paths[].path`
- `boundary_debug.excluded_paths[].kind`
- `boundary_debug.excluded_paths[].reason`
- `boundary_debug.excluded_paths[].rule`
- `boundary_debug.excluded[]` with the same item shape

`boundary_debug` is additive `1.x` surface area for explaining why declarations were included or excluded by `public_api_roots`, include/exclude selectors, boundary-following policy, explicit `exclude_paths`, and default ignored directories.

### `validate --format json`

Stable fields:

- `compatible`
- `degraded`
- `failure_count`
- `degraded_count`
- `host_manifest`
- `extension_manifest`
- `target_context`
- `reasons`
- `checks`

Stable `checks[]` fields:

- `axis`
- `status`
- `code`
- `message`
- `details`

Stable `axis` values:

- `protocol`
- `context`
- `runtime_profile`
- `capabilities`
- `ai_schema`
- `ai_model_capabilities`

Stable `status` values:

- `pass`
- `degraded`
- `fail`

### `report --format json`

Stable fields:

- `base_manifest`
- `target_manifest`
- `change_count`
- `changed_sections`
- `comparison`

Stable `status` values:

- `changed`
- `unchanged`

Stable `comparison.identity` fields:

- `base`
- `target`
- `arch_delta`
- `feat_delta`
- `prot_delta`
- `context_changed`

Stable `comparison.compatibility` fields:

- `host_range_changed`
- `extension_range_changed`
- `base_host_range`
- `target_host_range`
- `base_extension_range`
- `target_extension_range`
- `added_legacy_shims`
- `removed_legacy_shims`

Stable `comparison.capabilities.changes[]` fields:

- `field`
- `base`
- `target`

Stable `comparison.ai_contract` fields:

- `tool_schema_version_changed`
- `tool_schema_hash_changed`
- `prompt_contract_id_changed`
- `base_tool_schema_version`
- `target_tool_schema_version`
- `base_tool_schema_hash`
- `target_tool_schema_hash`
- `base_prompt_contract_id`
- `target_prompt_contract_id`
- `required_model_capabilities`
- `provided_model_capabilities`

Stable `comparison.environment` fields:

- `profiles`
- `runtime_constraints[].field`
- `runtime_constraints[].base`
- `runtime_constraints[].target`

Stable `comparison.scan_policy.changes[]` fields:

- `field`
- `base`
- `target`

Stable `comparison.evidence` fields:

- `feature_hash_changed`
- `protocol_hash_changed`
- `public_api_hash_changed`
- `diff`

Current stable `code` values include:

- `protocol_range_ok`
- `protocol_range_shimmed`
- `protocol_range_mismatch`
- `context_ok`
- `context_mismatch`
- `runtime_profile_ok`
- `runtime_profile_missing`
- `required_capabilities_ok`
- `required_capabilities_missing`
- `ai_schema_version_ok`
- `ai_schema_version_missing`
- `ai_model_capabilities_ok`
- `ai_model_capabilities_missing`

Automation should key off `exit_code`, `checks[].axis`, `checks[].status`, and `checks[].code`. Human-readable strings such as `reasons`, `failures`, and `checks[].message` are also tracked in golden fixtures and should be treated as stable unless the project deliberately announces a contract revision.

## Exit Codes

Stable `1.x` exit codes:

- `0`: success
- `10`: `generate` execution failure
- `20`: `lint` detected drift
- `21`: `lint` execution failure
- `30`: `validate` incompatibility
- `40`: manifest read, parse, write, or validation failure
- `70`: output rendering failure

## Intentionally Variable Data

These values are expected to vary by machine or run and should be normalized in contract tests:

- absolute filesystem paths in command output such as `manifest_path`, `root`, `host_manifest`, and `extension_manifest`
- `history[].changed_at_unix`

Their presence and meaning are stable even though the raw values vary.

## Review Gate

The public contract is guarded by golden fixtures in:

- `tests/fixtures/contracts/generate_cli.json`
- `tests/fixtures/contracts/generator_manifest_cli.json`
- `tests/fixtures/contracts/lint_public_api_drift.json`
- `tests/fixtures/contracts/validate_incompatible.json`
- `tests/fixtures/contracts/validate_degraded.json`

When one of these fixtures changes intentionally:

1. update the implementation
2. update the golden fixture
3. update this document if semantics changed
4. call the change out in release notes or upgrade guidance
