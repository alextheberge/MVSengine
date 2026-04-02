// SPDX-License-Identifier: AGPL-3.0-only
use serde::Serialize;

use crate::mvs::crawler::{ExcludedPathDecision, PublicApiBoundaryDecision};
use crate::mvs::manifest::ScanPolicy;

pub fn build_boundary_debug(
    scan_policy: &ScanPolicy,
    decisions: &[PublicApiBoundaryDecision],
    excluded_paths: &[ExcludedPathDecision],
) -> Option<BoundaryDebugReport> {
    if !has_boundary_debug_policy(scan_policy) && excluded_paths.is_empty() {
        return None;
    }

    let included: Vec<PublicApiBoundaryDecision> = decisions
        .iter()
        .filter(|decision| decision.included)
        .cloned()
        .collect();
    let excluded: Vec<PublicApiBoundaryDecision> = decisions
        .iter()
        .filter(|decision| !decision.included)
        .cloned()
        .collect();

    Some(BoundaryDebugReport {
        included_count: included.len(),
        excluded_count: excluded.len(),
        excluded_path_count: excluded_paths.len(),
        included,
        excluded,
        excluded_paths: excluded_paths.to_vec(),
    })
}

fn has_boundary_debug_policy(scan_policy: &ScanPolicy) -> bool {
    !scan_policy.public_api_roots.is_empty()
        || !scan_policy.public_api_includes.is_empty()
        || !scan_policy.public_api_excludes.is_empty()
        || !scan_policy.ts_export_following.is_default()
        || !scan_policy.go_export_following.is_default()
        || !scan_policy.rust_export_following.is_default()
        || !scan_policy.ruby_export_following.is_default()
        || !scan_policy.lua_export_following.is_default()
        || !scan_policy.python_export_following.is_default()
        || !scan_policy.python_module_roots.is_empty()
        || !scan_policy.rust_workspace_members.is_empty()
}

#[derive(Debug, Serialize)]
pub struct BoundaryDebugReport {
    pub included_count: usize,
    pub excluded_count: usize,
    pub excluded_path_count: usize,
    pub included: Vec<PublicApiBoundaryDecision>,
    pub excluded: Vec<PublicApiBoundaryDecision>,
    pub excluded_paths: Vec<ExcludedPathDecision>,
}
