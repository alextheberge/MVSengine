// SPDX-License-Identifier: AGPL-3.0-only
use crate::cli::{SchemaArgs, EXIT_OUTPUT_ERROR, EXIT_SUCCESS};

/// @mvs-feature("manifest_schema_output")
/// @mvs-protocol("cli_schema_command")
pub fn run(args: SchemaArgs) -> i32 {
    let schema = MVS_JSON_SCHEMA_V1;

    match args.output {
        None => {
            println!("{schema}");
            EXIT_SUCCESS
        }
        Some(path) => match std::fs::write(&path, format!("{schema}\n")) {
            Ok(()) => {
                eprintln!("Schema written to: {}", path.display());
                EXIT_SUCCESS
            }
            Err(error) => {
                eprintln!(
                    "error: failed to write schema to `{}`: {error}",
                    path.display()
                );
                EXIT_OUTPUT_ERROR
            }
        },
    }
}

/// Canonical JSON Schema for mvs.json (schema version 1).
///
/// Mirrors the `Manifest` struct in `src/mvs/manifest.rs`.  Keep in sync
/// whenever new top-level fields are added (additive changes only in 1.x).
pub const MVS_JSON_SCHEMA_V1: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://mvs.dev/schema/v1",
  "title": "MVS Manifest",
  "description": "Multidimensional versioning manifest (mvs.json).  Tracks ARCH.FEAT.PROT-CONT version axes, public API evidence, host/extension compatibility ranges, and AI contract metadata.",
  "type": "object",
  "required": ["$schema", "identity"],
  "additionalProperties": false,
  "properties": {
    "$schema": {
      "type": "string",
      "description": "Must be 'https://mvs.dev/schema/v1'.",
      "const": "https://mvs.dev/schema/v1"
    },
    "identity": {
      "type": "object",
      "description": "Version identity block.",
      "required": ["mvs", "arch", "feat", "prot", "cont"],
      "additionalProperties": false,
      "properties": {
        "mvs": {
          "type": "string",
          "description": "Canonical version string in the form ARCH.FEAT.PROT-CONT.",
          "pattern": "^[0-9]+\\.[0-9]+\\.[0-9]+-[A-Za-z0-9_-]+$"
        },
        "arch": { "type": "integer", "minimum": 0 },
        "feat": { "type": "integer", "minimum": 0 },
        "prot": { "type": "integer", "minimum": 0 },
        "cont": {
          "type": "string",
          "description": "Deployment context label, e.g. 'cli', 'lib', 'plugin'.",
          "minLength": 1
        }
      }
    },
    "compatibility": {
      "type": "object",
      "description": "Protocol range compatibility rules between hosts and extensions.",
      "additionalProperties": false,
      "properties": {
        "host_range": { "$ref": "#/$defs/protocolRange" },
        "extension_range": { "$ref": "#/$defs/protocolRange" },
        "legacy_shims": {
          "type": "array",
          "description": "Backward-compatibility shims that allow out-of-range protocol versions to pass validation.",
          "items": { "$ref": "#/$defs/legacyShim" },
          "default": []
        }
      }
    },
    "capabilities": {
      "type": "object",
      "description": "Named feature capability flags.",
      "additionalProperties": { "type": "boolean" },
      "default": {}
    },
    "ai_contract": {
      "type": "object",
      "description": "AI tool-calling contract metadata.",
      "additionalProperties": false,
      "properties": {
        "tool_schema_version": { "type": "integer", "minimum": 1 },
        "tool_schema_hash": {
          "type": "string",
          "description": "SHA-256 hex hash of the AI tool schema file, or empty string if no schema is tracked."
        },
        "required_model_capabilities": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Model capabilities that must be present at runtime (e.g. 'tool_calling', 'json_schema').",
          "default": []
        },
        "provided_model_capabilities": {
          "type": "array",
          "items": { "type": "string" },
          "default": []
        },
        "prompt_contract_id": {
          "type": "string",
          "description": "Opaque identifier for the prompt contract version in use."
        }
      }
    },
    "environment": {
      "type": "object",
      "description": "Deployment environment metadata.",
      "additionalProperties": false,
      "properties": {
        "profiles": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Active deployment profiles (e.g. 'cli', 'staging').",
          "default": []
        },
        "runtime_constraints": {
          "type": "object",
          "additionalProperties": { "type": "string" },
          "description": "Key/value runtime version constraints (e.g. {'rust': '1.74+'}).",
          "default": {}
        }
      }
    },
    "scan_policy": {
      "type": "object",
      "description": "Controls which files and symbols the generator includes in the public API inventory.",
      "additionalProperties": false,
      "properties": {
        "exclude_paths": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Glob patterns for paths to exclude from scanning entirely (e.g. 'tests', 'target').",
          "default": []
        },
        "public_api_roots": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Explicit facade files that define the public API surface.  When set, only symbols reachable from these roots are included.",
          "default": []
        },
        "public_api_includes": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Inclusion filter patterns in the form 'path|lang:sig_pattern' (e.g. 'src/api.rs|rust:fn *').",
          "default": []
        },
        "public_api_excludes": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Exclusion filter patterns in the form 'lang:sig_pattern' or 'path|lang:sig_pattern'.",
          "default": []
        },
        "ts_export_following": {
          "type": "string",
          "enum": ["off", "relative_only", "workspace_only"],
          "description": "TypeScript/JavaScript export-following depth.",
          "default": "off"
        },
        "go_export_following": {
          "type": "string",
          "enum": ["off", "package_only"],
          "description": "Go package-level export following.",
          "default": "off"
        },
        "rust_export_following": {
          "type": "string",
          "enum": ["off", "public_modules"],
          "description": "Rust public-module following (follows 'pub mod' declarations).",
          "default": "off"
        },
        "ruby_export_following": {
          "type": "string",
          "enum": ["off", "heuristic"],
          "description": "Ruby heuristic export following.",
          "default": "heuristic"
        },
        "lua_export_following": {
          "type": "string",
          "enum": ["off", "returned_root_only", "heuristic"],
          "description": "Lua/Luau module export following mode.",
          "default": "heuristic"
        },
        "python_export_following": {
          "type": "string",
          "enum": ["off", "roots_only", "heuristic"],
          "description": "Python __all__ and re-export following mode.",
          "default": "heuristic"
        },
        "python_module_roots": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Explicit Python module root directories for cross-module re-export resolution.",
          "default": []
        },
        "rust_workspace_members": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Paths to crate roots within a Rust workspace (e.g. 'crates/foo/src/lib.rs').",
          "default": []
        }
      }
    },
    "evidence": {
      "type": "object",
      "description": "Deterministic hashes and inventories captured from the last `generate` run.",
      "additionalProperties": false,
      "properties": {
        "feature_hash": { "type": "string" },
        "protocol_hash": { "type": "string" },
        "public_api_hash": { "type": "string" },
        "feature_inventory": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Sorted @mvs-feature tag values found in the codebase.",
          "default": []
        },
        "protocol_inventory": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Sorted @mvs-protocol tag values found in the codebase.",
          "default": []
        },
        "public_api_inventory": {
          "type": "array",
          "items": { "$ref": "#/$defs/publicApiSnapshot" },
          "description": "Sorted public API symbol/file pairs from the last crawl.",
          "default": []
        }
      }
    },
    "history": {
      "type": "array",
      "description": "Append-only log of past version entries with change reasons.",
      "items": { "$ref": "#/$defs/historyEntry" },
      "default": []
    }
  },
  "$defs": {
    "protocolRange": {
      "type": "object",
      "description": "Inclusive protocol version range.",
      "required": ["min_prot", "max_prot"],
      "additionalProperties": false,
      "properties": {
        "min_prot": { "type": "integer", "minimum": 0 },
        "max_prot": { "type": "integer", "minimum": 0 }
      }
    },
    "legacyShim": {
      "type": "object",
      "description": "A shim rule that allows a specific out-of-range protocol version to pass compatibility validation.",
      "required": ["from_prot", "to_prot", "adapter"],
      "additionalProperties": false,
      "properties": {
        "from_prot": { "type": "integer", "minimum": 0 },
        "to_prot": { "type": "integer", "minimum": 0 },
        "adapter": { "type": "string", "description": "Identifier of the adapter that bridges from_prot to to_prot." }
      }
    },
    "publicApiSnapshot": {
      "type": "object",
      "required": ["file", "signature"],
      "additionalProperties": false,
      "properties": {
        "file": { "type": "string", "description": "Project-relative source file path." },
        "signature": { "type": "string", "description": "Canonical language-prefixed signature, e.g. 'rust:fn run() -> i32'." }
      }
    },
    "historyEntry": {
      "type": "object",
      "required": ["mvs", "arch", "feat", "prot", "cont", "reasons", "changed_at_unix"],
      "additionalProperties": false,
      "properties": {
        "mvs": { "type": "string" },
        "arch": { "type": "integer", "minimum": 0 },
        "feat": { "type": "integer", "minimum": 0 },
        "prot": { "type": "integer", "minimum": 0 },
        "cont": { "type": "string" },
        "reasons": {
          "type": "array",
          "items": { "type": "string" }
        },
        "changed_at_unix": { "type": "integer", "minimum": 0 }
      }
    }
  }
}"##;
