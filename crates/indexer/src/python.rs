//! Python symbol and edge extraction.

use tree_sitter::Node;

use crate::IndexError;
use crate::facts::{Builder, clamp_documentation};

const COMMENT_KINDS: [&str; 1] = ["comment"];

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
        "function_definition" => {
            if let Some(name) = builder.field_text(node, "name") {
                // A function directly inside a class body is a method.
                let kind = if owner.is_some() { "method" } else { "function" };
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    owner,
                    parent,
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    !name.starts_with('_'),
                );
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
                child_owner = None;
            }
        }
        "class_definition" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "class",
                    &name,
                    owner,
                    parent,
                    None,
                    documentation(builder, node),
                    !name.starts_with('_'),
                );
                capture_superclasses(builder, node, &key)?;
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
                child_owner = Some(name);
            }
        }
        "assignment" => {
            // Only module- and class-level bindings are definitions; locals are not.
            if enclosing.is_none()
                && let Some(name) = builder.field_text(node, "left")
                && is_plain_identifier(node)
            {
                builder.add_symbol(
                    node,
                    "variable",
                    &name,
                    owner,
                    parent,
                    None,
                    None,
                    !name.starts_with('_'),
                );
            }
        }
        "import_statement" => {
            if let Some(module) = builder.field_text(node, "name") {
                builder.add_edge(node, None, "IMPORTS", "module", &module, 1.0)?;
            }
        }
        "import_from_statement" => {
            if let Some(module) = builder.field_text(node, "module_name") {
                builder.add_edge(node, None, "IMPORTS", "module", &module, 1.0)?;
            }
        }
        "call" => {
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
        "identifier" => {
            if let Some(source_key) = enclosing
                && !is_definition_name(node)
                && !is_call_target(node)
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

/// Python documents a definition with the first string in its body, and falls
/// back to the comment block directly above it.
fn documentation(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    let docstring = node
        .child_by_field_name("body")
        .and_then(|body| body.named_child(0))
        .filter(|first| first.kind() == "expression_statement")
        .and_then(|statement| statement.named_child(0))
        .filter(|value| value.kind() == "string")
        .and_then(|string| {
            let mut cursor = string.walk();
            string
                .named_children(&mut cursor)
                .find(|child| child.kind() == "string_content")
        })
        .and_then(|content| builder.text(content));

    docstring
        .or_else(|| {
            crate::facts::leading_comment(node, &builder.source.content, &COMMENT_KINDS)
        })
        .map(|documentation| clamp_documentation(documentation.trim().to_owned()))
        .filter(|documentation| !documentation.is_empty())
}

fn capture_superclasses(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    source_key: &str,
) -> Result<(), IndexError> {
    let Some(arguments) = node.child_by_field_name("superclasses") else {
        return Ok(());
    };
    let mut cursor = arguments.walk();
    let bases: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|child| matches!(child.kind(), "identifier" | "attribute"))
        .collect();
    for base in bases {
        if let Some(name) = callable_name(builder, base) {
            builder.add_edge(
                base,
                Some(source_key.to_owned()),
                "EXTENDS",
                "symbol",
                &name,
                0.8,
            )?;
        }
    }
    Ok(())
}

fn is_plain_identifier(node: Node<'_>) -> bool {
    node.child_by_field_name("left")
        .is_some_and(|left| left.kind() == "identifier")
}

fn callable_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "identifier" => builder.text(node),
        "attribute" => builder.field_text(node, "attribute"),
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

fn is_call_target(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(parent.kind(), "call" | "attribute")
            || parent
                .child_by_field_name("function")
                .is_some_and(|function| function.id() == node.id())
    })
}
