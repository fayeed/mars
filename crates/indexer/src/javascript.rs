use std::collections::HashMap;

use serde::Serialize;
use tree_sitter::{Language, Node, Parser};

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
}

pub fn parse(source: SourceFile) -> Result<ParsedFile, IndexError> {
    let language = language(&source.language);
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| IndexError::Parse(error.to_string()))?;
    let tree = parser
        .parse(&source.content, None)
        .ok_or_else(|| IndexError::Parse("parser returned no tree".into()))?;
    let parse_errors = count_errors(tree.root_node());
    let mut extractor = Extractor {
        source: &source,
        symbols: Vec::new(),
        edges: Vec::new(),
        ordinals: HashMap::new(),
    };
    extractor.visit(tree.root_node(), None, None)?;
    let symbols = std::mem::take(&mut extractor.symbols);
    let edges = std::mem::take(&mut extractor.edges);
    drop(extractor);
    Ok(ParsedFile {
        source,
        symbols,
        edges,
        parse_errors,
    })
}

fn language(name: &str) -> Language {
    match name {
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

struct Extractor<'a> {
    source: &'a SourceFile,
    symbols: Vec<SymbolFact>,
    edges: Vec<EdgeFact>,
    ordinals: HashMap<(String, String), usize>,
}

impl Extractor<'_> {
    fn visit(
        &mut self,
        node: Node<'_>,
        owner: Option<&str>,
        current_source: Option<String>,
    ) -> Result<(), IndexError> {
        let mut nested_owner = owner.map(str::to_owned);
        let mut nested_source = current_source;

        match node.kind() {
            "function_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    let key = self.add_symbol(
                        node,
                        "function",
                        &name,
                        owner,
                        signature(node, &self.source.content),
                    );
                    nested_source = Some(key);
                }
            }
            "class_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    let key = self.add_symbol(node, "class", &name, owner, None);
                    nested_owner = Some(name);
                    nested_source = Some(key.clone());
                    self.capture_extends(node, &key)?;
                }
            }
            "interface_declaration" => {
                if let Some(name) = self.field_text(node, "name") {
                    nested_source = Some(self.add_symbol(node, "interface", &name, owner, None));
                    nested_owner = Some(name);
                }
            }
            "variable_declarator" => {
                if let Some(name) = self.field_text(node, "name") {
                    let kind = node
                        .child_by_field_name("value")
                        .filter(|value| {
                            matches!(value.kind(), "arrow_function" | "function_expression")
                        })
                        .map_or("variable", |_| "function");
                    nested_source = Some(self.add_symbol(node, kind, &name, owner, None));
                }
            }
            "method_definition" | "method_signature" => {
                if let Some(name) = self.field_text(node, "name") {
                    nested_source = Some(self.add_symbol(
                        node,
                        "method",
                        &name,
                        owner,
                        signature(node, &self.source.content),
                    ));
                }
            }
            "import_statement" => {
                if let Some(target) = self.field_text(node, "source") {
                    self.add_edge(node, None, "IMPORTS", "module", &unquote(&target), 1.0)?;
                }
            }
            "call_expression" => {
                if let Some(function) = node.child_by_field_name("function")
                    && let Some(target) = callable_name(function, &self.source.content)
                {
                    self.add_edge(
                        node,
                        nested_source.clone(),
                        "CALLS",
                        "symbol",
                        &target,
                        0.45,
                    )?;
                    if (target == "it" || target == "test")
                        && let Some(test_name) = first_string_argument(node, &self.source.content)
                    {
                        nested_source =
                            Some(self.add_symbol(node, "test", &test_name, owner, None));
                    }
                }
            }
            "identifier" | "type_identifier" => {
                if let Some(source_key) = nested_source.clone()
                    && !is_definition_name(node)
                    && !is_import_identifier(node)
                    && !is_call_target(node)
                    && let Ok(target) = node.utf8_text(&self.source.content)
                {
                    self.add_edge(node, Some(source_key), "REFERENCES", "symbol", target, 0.35)?;
                }
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, nested_owner.as_deref(), nested_source.clone())?;
        }
        Ok(())
    }

    fn add_symbol(
        &mut self,
        node: Node<'_>,
        kind: &str,
        name: &str,
        owner: Option<&str>,
        signature: Option<String>,
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
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            signature,
            exported: is_exported(node),
        });
        stable_key
    }

    fn capture_extends(&mut self, node: Node<'_>, source: &str) -> Result<(), IndexError> {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "class_heritage"
                && let Some(target) = first_identifier(child, &self.source.content)
            {
                self.add_edge(
                    child,
                    Some(source.into()),
                    "EXTENDS",
                    "symbol",
                    &target,
                    0.8,
                )?;
            }
        }
        Ok(())
    }

    fn add_edge(
        &mut self,
        node: Node<'_>,
        source_stable_key: Option<String>,
        edge_kind: &str,
        target_kind: &str,
        target_key: &str,
        confidence: f64,
    ) -> Result<(), IndexError> {
        let evidence_json = serde_json::to_string(&Evidence {
            relative_path: &self.source.relative_path,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            parser: "tree-sitter",
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

    fn field_text(&self, node: Node<'_>, field: &str) -> Option<String> {
        let field = node.child_by_field_name(field)?;
        field
            .utf8_text(&self.source.content)
            .ok()
            .map(str::to_owned)
    }
}

fn is_exported(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return true;
        }
        if parent.kind() == "program" {
            break;
        }
        node = parent;
    }
    false
}

fn is_definition_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent
            .child_by_field_name("name")
            .is_some_and(|name| name.id() == node.id())
    })
}

fn is_import_identifier(node: Node<'_>) -> bool {
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.kind() == "import_statement" {
            return true;
        }
        if matches!(parent.kind(), "program" | "statement_block") {
            break;
        }
        ancestor = parent.parent();
    }
    false
}

fn is_call_target(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent.kind() == "call_expression"
            && parent
                .child_by_field_name("function")
                .is_some_and(|function| function.id() == node.id())
    })
}

fn callable_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        return node.utf8_text(source).ok().map(str::to_owned);
    }
    node.child_by_field_name("property")
        .or_else(|| node.child_by_field_name("name"))?
        .utf8_text(source)
        .ok()
        .map(str::to_owned)
}

fn first_identifier(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        return node.utf8_text(source).ok().map(str::to_owned);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(|child| first_identifier(child, source))
}

fn first_string_argument(node: Node<'_>, source: &[u8]) -> Option<String> {
    let arguments = node.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let string = arguments
        .named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "string" | "string_fragment"))?;
    string.utf8_text(source).ok().map(unquote)
}

fn signature(node: Node<'_>, source: &[u8]) -> Option<String> {
    let parameters = node.child_by_field_name("parameters")?;
    parameters.utf8_text(source).ok().map(str::to_owned)
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix(['\'', '"'])
        .and_then(|value| value.strip_suffix(['\'', '"']))
        .unwrap_or(value)
        .to_owned()
}

fn count_errors(node: Node<'_>) -> usize {
    let own = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    own + node
        .named_children(&mut cursor)
        .map(count_errors)
        .sum::<usize>()
}
