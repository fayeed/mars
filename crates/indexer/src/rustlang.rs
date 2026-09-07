//! Rust symbol and edge extraction.

use tree_sitter::Node;

use crate::IndexError;
use crate::facts::{Builder, clamp_documentation, leading_comment};

const COMMENT_KINDS: [&str; 2] = ["line_comment", "block_comment"];

pub(crate) fn extract(builder: &mut Builder<'_>, root: Node<'_>) -> Result<(), IndexError> {
    visit(builder, root, None, None, None)
}

fn visit(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    owner: Option<&str>,
    parent: Option<&str>,
    enclosing: Option<&str>,
) -> Result<(), IndexError> {
    let mut child_owner = owner.map(str::to_owned);
    let mut child_parent = parent.map(str::to_owned);
    let mut child_enclosing = enclosing.map(str::to_owned);

    match node.kind() {
        "function_item" | "function_signature_item" => {
            if let Some(name) = builder.field_text(node, "name") {
                // A function inside an `impl` or `trait` block is a method.
                let kind = if owner.is_some() { "method" } else { "function" };
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    owner,
                    parent,
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_public(node),
                );
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
                child_owner = None;
            }
        }
        "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item" | "mod_item" => {
            if let Some(name) = builder.field_text(node, "name") {
                let kind = match node.kind() {
                    "struct_item" => "struct",
                    "enum_item" => "enum",
                    "union_item" => "union",
                    "trait_item" => "interface",
                    "mod_item" => "module",
                    _ => "type",
                };
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    None,
                    parent,
                    None,
                    documentation(builder, node),
                    is_public(node),
                );
                child_owner = Some(name);
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
            }
        }
        "const_item" | "static_item" => {
            if let Some(name) = builder.field_text(node, "name") {
                let kind = if node.kind() == "const_item" {
                    "constant"
                } else {
                    "variable"
                };
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    owner,
                    parent,
                    None,
                    documentation(builder, node),
                    is_public(node),
                );
                child_enclosing = Some(key);
            }
        }
        "impl_item" => {
            let implemented = node
                .child_by_field_name("type")
                .and_then(|kind| type_name(builder, kind));
            let trait_name = node
                .child_by_field_name("trait")
                .and_then(|kind| type_name(builder, kind));
            if let Some(implemented) = implemented.clone() {
                // `impl Trait for Type` is an IMPLEMENTS edge from the type.
                if let Some(trait_name) = trait_name {
                    builder.add_edge(node, None, "IMPLEMENTS", "symbol", &trait_name, 0.8)?;
                    builder.add_edge(node, None, "REFERENCES", "symbol", &implemented, 0.8)?;
                }
                child_owner = Some(implemented);
            }
            child_parent = None;
            child_enclosing = None;
        }
        "use_declaration" => {
            if let Some(argument) = builder.field_text(node, "argument") {
                builder.add_edge(node, None, "IMPORTS", "module", &argument, 1.0)?;
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function")
                && let Some(target) = callable_name(builder, function)
            {
                builder.add_edge(
                    node,
                    enclosing.map(str::to_owned),
                    "CALLS",
                    "symbol",
                    &target,
                    0.45,
                )?;
            }
        }
        "type_identifier" => {
            if let Some(source_key) = enclosing
                && !is_definition_name(node)
                && let Some(target) = builder.text(node)
            {
                builder.add_edge(
                    node,
                    Some(source_key.to_owned()),
                    "REFERENCES",
                    "symbol",
                    &target,
                    0.35,
                )?;
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(
            builder,
            child,
            child_owner.as_deref(),
            child_parent.as_deref(),
            child_enclosing.as_deref(),
        )?;
    }
    Ok(())
}

/// Rust doc comments are `line_comment`/`block_comment` nodes carrying a `doc`
/// field; plain comments above an item are accepted as a fallback.
fn documentation(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    leading_comment(node, &builder.source.content, &COMMENT_KINDS)
        .map(clamp_documentation)
        .filter(|documentation| !documentation.is_empty())
}

fn is_public(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "visibility_modifier")
}

fn type_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" => builder.text(node),
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name(builder, inner)),
        "scoped_type_identifier" | "scoped_identifier" => node
            .child_by_field_name("name")
            .and_then(|inner| type_name(builder, inner)),
        "reference_type" => node
            .child_by_field_name("type")
            .and_then(|inner| type_name(builder, inner)),
        _ => None,
    }
}

fn callable_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "identifier" => builder.text(node),
        "field_expression" => builder.field_text(node, "field"),
        "scoped_identifier" => builder.field_text(node, "name"),
        "generic_function" => node
            .child_by_field_name("function")
            .and_then(|inner| callable_name(builder, inner)),
        _ => None,
    }
}

fn is_definition_name(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        parent
            .child_by_field_name("name")
            .is_some_and(|name| name.id() == node.id())
    })
}
