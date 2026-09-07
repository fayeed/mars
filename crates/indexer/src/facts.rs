//! Facts produced by parsing one file, plus the builder every language
//! extractor writes through so stable keys and evidence stay uniform.

use std::collections::HashMap;

use serde::Serialize;
use tree_sitter::Node;

use crate::IndexError;
use crate::walk::SourceFile;

#[derive(Clone, Debug)]
pub struct ParsedFile {
    pub source: SourceFile,
    pub symbols: Vec<SymbolFact>,
    pub edges: Vec<EdgeFact>,
    pub parse_errors: usize,
}

#[derive(Clone, Debug)]
pub struct SymbolFact {
    pub stable_key: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub parent_stable_key: Option<String>,
    pub documentation: Option<String>,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub signature: Option<String>,
    pub exported: bool,
}

#[derive(Clone, Debug)]
pub struct EdgeFact {
    pub source_stable_key: Option<String>,
    pub edge_kind: String,
    pub target_kind: String,
    pub target_key: String,
    pub confidence: f64,
    pub evidence_json: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence<'a> {
    relative_path: &'a str,
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    end_line: usize,
    parser: &'static str,
    grammar: &'a str,
}

/// Accumulates symbols and edges for one file, assigning the deterministic
/// stable keys that survive re-indexing.
pub struct Builder<'a> {
    pub source: &'a SourceFile,
    symbols: Vec<SymbolFact>,
    edges: Vec<EdgeFact>,
    ordinals: HashMap<(String, String), usize>,
}

impl<'a> Builder<'a> {
    #[must_use]
    pub fn new(source: &'a SourceFile) -> Self {
        Self {
            source,
            symbols: Vec::new(),
            edges: Vec::new(),
            ordinals: HashMap::new(),
        }
    }

    #[must_use]
    pub fn finish(self, parse_errors: usize) -> (Vec<SymbolFact>, Vec<EdgeFact>, usize) {
        (self.symbols, self.edges, parse_errors)
    }

    /// Records a definition and returns its stable key.
    #[allow(clippy::too_many_arguments)]
    pub fn add_symbol(
        &mut self,
        node: Node<'_>,
        kind: &str,
        name: &str,
        owner: Option<&str>,
        parent_stable_key: Option<&str>,
        signature: Option<String>,
        documentation: Option<String>,
        exported: bool,
    ) -> String {
        let qualified_name =
            owner.map_or_else(|| name.to_owned(), |owner| format!("{owner}.{name}"));
        let ordinal = self
            .ordinals
            .entry((kind.to_owned(), qualified_name.clone()))
            .or_default();
        let stable_key = format!("{kind}:{qualified_name}:{ordinal}");
        *ordinal += 1;
        self.symbols.push(SymbolFact {
            stable_key: stable_key.clone(),
            kind: kind.into(),
            name: name.into(),
            qualified_name,
            parent_stable_key: parent_stable_key.map(str::to_owned),
            documentation,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            signature,
            exported,
        });
        stable_key
    }

    pub fn add_edge(
        &mut self,
        node: Node<'_>,
        source_stable_key: Option<String>,
        edge_kind: &str,
        target_kind: &str,
        target_key: &str,
        confidence: f64,
    ) -> Result<(), IndexError> {
        if target_key.trim().is_empty() {
            return Ok(());
        }
        let evidence_json = serde_json::to_string(&Evidence {
            relative_path: &self.source.relative_path,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            parser: "tree-sitter",
            grammar: &self.source.language,
        })?;
        self.edges.push(EdgeFact {
            source_stable_key,
            edge_kind: edge_kind.into(),
            target_kind: target_kind.into(),
            target_key: target_key.into(),
            confidence,
            evidence_json,
        });
        Ok(())
    }

    #[must_use]
    pub fn text(&self, node: Node<'_>) -> Option<String> {
        node.utf8_text(&self.source.content).ok().map(str::to_owned)
    }

    #[must_use]
    pub fn field_text(&self, node: Node<'_>, field: &str) -> Option<String> {
        self.text(node.child_by_field_name(field)?)
    }
}

/// Counts error and missing nodes so a file with broken syntax still publishes
/// the symbols the parser did recover.
#[must_use]
pub fn count_errors(node: Node<'_>) -> usize {
    let own = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    own + node
        .named_children(&mut cursor)
        .map(count_errors)
        .sum::<usize>()
}

/// Collects the run of comment lines immediately above a definition.
#[must_use]
pub fn leading_comment(node: Node<'_>, source: &[u8], comment_kinds: &[&str]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cursor = node.prev_sibling();
    let mut expected_row = node.start_position().row;

    while let Some(sibling) = cursor {
        if !comment_kinds.contains(&sibling.kind()) {
            break;
        }
        // Grammars disagree about whether a line comment owns its trailing
        // newline: Rust's ends at column 0 of the following row, Go's ends on
        // its own row. Both mean "the line directly above".
        let end_row = sibling.end_position().row;
        if end_row + 1 < expected_row || end_row > expected_row {
            break;
        }
        let Ok(text) = sibling.utf8_text(source) else {
            break;
        };
        lines.push(strip_comment_markers(text));
        expected_row = sibling.start_position().row;
        cursor = sibling.prev_sibling();
    }

    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    let documentation = lines.join("\n").trim().to_owned();
    (!documentation.is_empty()).then_some(documentation)
}

fn strip_comment_markers(text: &str) -> String {
    let trimmed = text.trim();
    let without_prefix = trimmed
        .strip_prefix("///")
        .or_else(|| trimmed.strip_prefix("//!"))
        .or_else(|| trimmed.strip_prefix("//"))
        .or_else(|| trimmed.strip_prefix('#'))
        .unwrap_or(trimmed);
    let without_block = without_prefix
        .strip_prefix("/**")
        .or_else(|| without_prefix.strip_prefix("/*"))
        .map(|value| value.strip_suffix("*/").unwrap_or(value))
        .unwrap_or(without_prefix);
    without_block
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Truncates documentation so a pathological comment cannot bloat the index.
#[must_use]
pub fn clamp_documentation(documentation: String) -> String {
    const MAX_BYTES: usize = 1024;
    if documentation.len() <= MAX_BYTES {
        return documentation;
    }
    let mut end = MAX_BYTES;
    while end > 0 && !documentation.is_char_boundary(end) {
        end -= 1;
    }
    documentation[..end].to_owned()
}
