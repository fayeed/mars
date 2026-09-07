//! TypeScript and JavaScript symbol and edge extraction.

use tree_sitter::Node;

use crate::IndexError;
use crate::facts::{Builder, clamp_documentation, leading_comment};

const COMMENT_KINDS: [&str; 1] = ["comment"];

pub(crate) fn extract(builder: &mut Builder<'_>, root: Node<'_>) -> Result<(), IndexError> {
    visit(builder, root, None, None)
}

fn visit(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    owner: Option<&str>,
    enclosing: Option<String>,
) -> Result<(), IndexError> {
    let mut nested_owner = owner.map(str::to_owned);
    // The enclosing symbol is both the parent of nested definitions and the
    // source of the edges found inside it.
    let mut nested_enclosing = enclosing.clone();

    match node.kind() {
        "function_declaration" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "function",
                    &name,
                    owner,
                    enclosing.as_deref(),
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_exported(node),
                );
                nested_enclosing = Some(key);
            }
        }
        "class_declaration" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "class",
                    &name,
                    owner,
                    enclosing.as_deref(),
                    None,
                    documentation(builder, node),
                    is_exported(node),
                );
                nested_owner = Some(name);
                nested_enclosing = Some(key.clone());
                capture_extends(builder, node, &key)?;
            }
        }
        "interface_declaration" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "interface",
                    &name,
                    owner,
                    enclosing.as_deref(),
                    None,
                    documentation(builder, node),
                    is_exported(node),
                );
                nested_owner = Some(name);
                nested_enclosing = Some(key);
            }
        }
        "variable_declarator" => {
            if let Some(name) = builder.field_text(node, "name") {
                let kind = node
                    .child_by_field_name("value")
                    .filter(|value| {
                        matches!(value.kind(), "arrow_function" | "function_expression")
                    })
                    .map_or("variable", |_| "function");
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    owner,
                    enclosing.as_deref(),
                    None,
                    declaration_documentation(builder, node),
                    is_exported(node),
                );
                nested_enclosing = Some(key);
            }
        }
        "method_definition" | "method_signature" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "method",
                    &name,
                    owner,
                    enclosing.as_deref(),
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_exported(node),
                );
                nested_enclosing = Some(key);
            }
        }
        "import_statement" => {
            if let Some(target) = builder.field_text(node, "source") {
                builder.add_edge(node, None, "IMPORTS", "module", &unquote(&target), 1.0)?;
            }
        }
        "call_expression" => {
            if let Some(function) = node.child_by_field_name("function")
                && let Some(target) = callable_name(builder, function)
            {
                builder.add_edge(
                    node,
                    nested_enclosing.clone(),
                    "CALLS",
                    "symbol",
                    &target,
                    0.45,
                )?;
                if (target == "it" || target == "test")
                    && let Some(test_name) = first_string_argument(builder, node)
                {
                    let key = builder.add_symbol(
                        node,
                        "test",
                        &test_name,
                        owner,
                        nested_enclosing.as_deref(),
                        None,
                        None,
                        false,
                    );
                    nested_enclosing = Some(key);
                }
            }
        }
        "identifier" | "type_identifier" => {
            if let Some(source_key) = nested_enclosing.clone()
                && !is_definition_name(node)
                && !is_import_identifier(node)
                && !is_call_target(node)
                && let Some(target) = builder.text(node)
            {
                builder.add_edge(
                    node,
                    Some(source_key),
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
            nested_owner.as_deref(),
            nested_enclosing.clone(),
        )?;
    }
    Ok(())
}

fn documentation(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    // An exported definition carries its comment above the `export` keyword.
    let anchor = node
        .parent()
        .filter(|parent| parent.kind() == "export_statement")
        .unwrap_or(node);
    leading_comment(anchor, &builder.source.content, &COMMENT_KINDS)
        .map(clamp_documentation)
        .filter(|documentation| !documentation.is_empty())
}

/// A declarator's comment sits above the enclosing `const`/`let` statement.
fn declaration_documentation(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    let anchor = node
        .parent()
        .filter(|parent| {
            parent.kind() == "variable_declaration" || parent.kind() == "lexical_declaration"
        })
        .unwrap_or(node);
    documentation(builder, anchor)
}

fn capture_extends(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    source_key: &str,
) -> Result<(), IndexError> {
    let mut cursor = node.walk();
    let heritage: Vec<Node<'_>> = node
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "class_heritage")
        .collect();
    for child in heritage {
        if let Some(target) = first_identifier(builder, child) {
            builder.add_edge(
                child,
                Some(source_key.to_owned()),
                "EXTENDS",
                "symbol",
                &target,
                0.8,
            )?;
        }
    }
    Ok(())
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

fn callable_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        return builder.text(node);
    }
    let target = node
        .child_by_field_name("property")
        .or_else(|| node.child_by_field_name("name"))?;
    builder.text(target)
}

fn first_identifier(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        return builder.text(node);
    }
    let mut cursor = node.walk();
    let children: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    children
        .into_iter()
        .find_map(|child| first_identifier(builder, child))
}

fn first_string_argument(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    let arguments = node.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let string = arguments
        .named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "string" | "string_fragment"))?;
    builder.text(string).map(|text| unquote(&text))
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix(['\'', '"'])
        .and_then(|value| value.strip_suffix(['\'', '"']))
        .unwrap_or(value)
        .to_owned()
}
