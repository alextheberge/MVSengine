// SPDX-License-Identifier: AGPL-3.0-only
use tree_sitter::Node;

use super::lua_family::{self, LuaDialect};

pub(super) fn extract(root: Node<'_>, source: &str) -> Vec<String> {
    lua_family::extract(root, source, LuaDialect::Lua)
}
