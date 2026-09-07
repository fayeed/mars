//! Prints the tree-sitter node kinds for a fixture file so extraction rules can be
//! written against the grammar that actually ships, not against remembered names.
//!
//! Usage: `cargo run -p mars-indexer --example grammar_probe -- <path> <language>`

use std::path::Path;

use tree_sitter::{Language, Node, Parser};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let path = arguments.next().expect("a file path is required");
    let language_name = arguments.next().expect("a language name is required");
    let source = std::fs::read(Path::new(&path)).expect("fixture should read");

    let mut parser = Parser::new();
    parser
        .set_language(&language(&language_name))
        .expect("grammar should load");
    let tree = parser.parse(&source, None).expect("source should parse");
    print(tree.root_node(), &source, 0);
}

fn language(name: &str) -> Language {
    match name {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "yaml" => tree_sitter_yaml::LANGUAGE.into(),
        "markdown" => tree_sitter_md::LANGUAGE.into(),
        other => panic!("unknown language: {other}"),
    }
}

fn print(node: Node<'_>, source: &[u8], depth: usize) {
    if depth > 6 {
        return;
    }
    let text = node
        .utf8_text(source)
        .unwrap_or("<binary>")
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(48)
        .collect::<String>();
    let fields: Vec<String> = (0..u32::try_from(node.child_count()).unwrap_or(0))
        .filter_map(|index| {
            let name = node.field_name_for_child(index)?;
            Some(format!("{name}={}", node.child(index)?.kind()))
        })
        .collect();
    println!(
        "{:indent$}{} [{}] {}",
        "",
        node.kind(),
        fields.join(" "),
        text,
        indent = depth * 2
    );
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        print(child, source, depth + 1);
    }
}
