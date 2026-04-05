// SPDX-License-Identifier: AGPL-3.0-only
use std::path::Path;

use tree_sitter::Language as TreeSitterLanguage;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum SourceLanguage {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Go,
    Python,
    Java,
    Kotlin,
    Csharp,
    Php,
    Ruby,
    Swift,
    Lua,
    Luau,
    Dart,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum LexStrategy {
    CStyle,
    Python,
    Php,
    Ruby,
    LuaFamily,
}

impl SourceLanguage {
    pub(super) fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Some(Self::Rust),
            Some("ts") => Some(Self::TypeScript),
            Some("tsx") => Some(Self::Tsx),
            Some("js") => Some(Self::JavaScript),
            Some("jsx") => Some(Self::Jsx),
            Some("go") => Some(Self::Go),
            Some("py") => Some(Self::Python),
            Some("java") => Some(Self::Java),
            Some("kt") => Some(Self::Kotlin),
            Some("cs") => Some(Self::Csharp),
            Some("php") => Some(Self::Php),
            Some("rb") => Some(Self::Ruby),
            Some("swift") => Some(Self::Swift),
            Some("lua") => Some(Self::Lua),
            Some("luau") => Some(Self::Luau),
            Some("dart") => Some(Self::Dart),
            _ => None,
        }
    }

    pub(super) fn extension_label(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::TypeScript => "ts",
            Self::Tsx => "tsx",
            Self::JavaScript => "js",
            Self::Jsx => "jsx",
            Self::Go => "go",
            Self::Python => "py",
            Self::Java => "java",
            Self::Kotlin => "kt",
            Self::Csharp => "cs",
            Self::Php => "php",
            Self::Ruby => "rb",
            Self::Swift => "swift",
            Self::Lua => "lua",
            Self::Luau => "luau",
            Self::Dart => "dart",
        }
    }

    pub(super) fn lex_strategy(self) -> LexStrategy {
        match self {
            Self::Python => LexStrategy::Python,
            Self::Php => LexStrategy::Php,
            Self::Ruby => LexStrategy::Ruby,
            Self::Lua | Self::Luau => LexStrategy::LuaFamily,
            _ => LexStrategy::CStyle,
        }
    }

    pub(super) fn uses_nested_block_comments(self) -> bool {
        matches!(self, Self::Rust | Self::Swift)
    }

    pub(super) fn tree_sitter_language(self) -> Option<TreeSitterLanguage> {
        match self {
            Self::Go => Some(tree_sitter_go::LANGUAGE.into()),
            Self::Python => Some(tree_sitter_python::LANGUAGE.into()),
            Self::Java => Some(tree_sitter_java::LANGUAGE.into()),
            Self::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
            Self::Csharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            Self::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
            Self::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
            Self::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
            Self::Lua => Some(tree_sitter_lua::LANGUAGE.into()),
            Self::Luau => Some(tree_sitter_luau::LANGUAGE.into()),
            Self::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            Self::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            Self::JavaScript | Self::Jsx => Some(tree_sitter_javascript::LANGUAGE.into()),
            Self::Dart | Self::Rust => None,
        }
    }
}
