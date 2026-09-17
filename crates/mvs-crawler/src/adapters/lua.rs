// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::lua_family::{self, LuaDialect};
use mvs_core::manifest::LuaExportFollowing;

pub(super) fn extract(
    root: Node<'_>,
    source: &str,
    export_following: LuaExportFollowing,
) -> Vec<String> {
    lua_family::extract(root, source, LuaDialect::Lua, export_following)
}
