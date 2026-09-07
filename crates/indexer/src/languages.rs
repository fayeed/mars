//! Language detection and grammar selection for the symbol layer.

use std::path::Path;

use tree_sitter::Language as Grammar;

/// Every language the indexer can parse into symbols.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Go,
    Rust,
    Json,
    Yaml,
    Markdown,
}

impl Language {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Jsx => "jsx",
            Self::Python => "python",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Markdown => "markdown",
        }
    }

    #[must_use]
    pub fn grammar(self) -> Grammar {
        match self {
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript | Self::Jsx => tree_sitter_javascript::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Self::Markdown => tree_sitter_md::LANGUAGE.into(),
        }
    }

    /// The JavaScript family shares one hand-written extractor; every other
    /// language is driven by declarative rules.
    #[must_use]
    pub fn is_javascript_family(self) -> bool {
        matches!(
            self,
            Self::TypeScript | Self::Tsx | Self::JavaScript | Self::Jsx
        )
    }

    /// Prose and configuration are chunked by structure but carry no call graph.
    #[must_use]
    pub fn is_structured_data(self) -> bool {
        matches!(self, Self::Json | Self::Yaml | Self::Markdown)
    }

    #[must_use]
    pub fn comment_prefix(self) -> &'static str {
        match self {
            Self::Python | Self::Yaml => "#",
            Self::Markdown => ">",
            _ => "//",
        }
    }
}

#[must_use]
pub fn detect(path: &Path) -> Option<Language> {
    if let Some(name) = path.file_name().and_then(|name| name.to_str())
        && matches!(name, "Cargo.lock" | "go.sum")
    {
        return None;
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "ts" | "mts" | "cts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "js" | "mjs" | "cjs" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "py" | "pyi" => Language::Python,
        "go" => Language::Go,
        "rs" => Language::Rust,
        "json" => Language::Json,
        "yaml" | "yml" => Language::Yaml,
        "md" | "markdown" => Language::Markdown,
        _ => return None,
    })
}

#[must_use]
pub fn from_name(name: &str) -> Option<Language> {
    Some(match name {
        "typescript" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "javascript" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "python" => Language::Python,
        "go" => Language::Go,
        "rust" => Language::Rust,
        "json" => Language::Json,
        "yaml" => Language::Yaml,
        "markdown" => Language::Markdown,
        _ => return None,
    })
}
