// SPDX-License-Identifier: AGPL-3.0-only
use anyhow::{anyhow, Context, Result};

use crate::cli::ValidateArgs;
use crate::mvs::manifest::Manifest;
use crate::mvs::reader::validate_host_extension;

/// @mvs-feature("manifest_compatibility_validation")
/// @mvs-protocol("cli_validate_command")
pub fn run(args: ValidateArgs) -> Result<()> {
    let host = Manifest::load(&args.host_manifest).with_context(|| {
        format!(
            "failed to read host manifest: {}",
            args.host_manifest.display()
        )
    })?;
    let extension = Manifest::load(&args.extension_manifest).with_context(|| {
        format!(
            "failed to read extension manifest: {}",
            args.extension_manifest.display()
        )
    })?;

    let result = validate_host_extension(
        &host,
        &extension,
        args.context.as_deref(),
        args.allow_shims,
        if args.host_model_capabilities.is_empty() {
            None
        } else {
            Some(args.host_model_capabilities.as_slice())
        },
    );

    if result.compatible {
        if result.degraded {
            println!("Compatibility: DEGRADED (legacy shim path)");
        } else {
            println!("Compatibility: OK");
        }
    } else {
        println!("Compatibility: INCOMPATIBLE");
    }

    for reason in &result.reasons {
        println!("- {reason}");
    }

    if result.compatible {
        Ok(())
    } else {
        Err(anyhow!("manifest compatibility validation failed"))
    }
}
