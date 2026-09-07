//! Go symbol and edge extraction.

use tree_sitter::Node;

use crate::IndexError;
use crate::facts::{Builder, clamp_documentation, leading_comment};

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
        "function_declaration" => {
            if let Some(name) = builder.field_text(node, "name") {
                let key = builder.add_symbol(
                    node,
                    "function",
                    &name,
                    None,
                    None,
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_exported(&name),
                );
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
            }
        }
        "method_declaration" => {
            if let Some(name) = builder.field_text(node, "name") {
                let receiver = receiver_type(builder, node);
                let key = builder.add_symbol(
                    node,
                    "method",
                    &name,
                    receiver.as_deref(),
                    None,
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_exported(&name),
                );
                // A method belongs to the type it is declared on.
                if let Some(receiver) = receiver.as_deref() {
                    builder.add_edge(
                        node,
                        Some(key.clone()),
                        "REFERENCES",
                        "symbol",
                        receiver,
                        0.7,
                    )?;
                }
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
            }
        }
        "type_spec" => {
            if let Some(name) = builder.field_text(node, "name") {
                let kind = match node.child_by_field_name("type").map(|node| node.kind()) {
                    Some("interface_type") => "interface",
                    Some("struct_type") => "struct",
                    _ => "type",
                };
                // Documentation sits above the enclosing `type` declaration.
                let documentation = node
                    .parent()
                    .and_then(|declaration| self::documentation(builder, declaration))
                    .or_else(|| self::documentation(builder, node));
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    None,
                    None,
                    None,
                    documentation,
                    is_exported(&name),
                );
                child_owner = Some(name);
                child_parent = Some(key.clone());
                child_enclosing = Some(key);
            }
        }
        "method_elem" => {
            if let Some(name) = builder.field_text(node, "name") {
                builder.add_symbol(
                    node,
                    "method",
                    &name,
                    owner,
                    parent,
                    builder.field_text(node, "parameters"),
                    documentation(builder, node),
                    is_exported(&name),
                );
            }
        }
        "const_spec" | "var_spec" => {
            if let Some(name) = builder.field_text(node, "name") {
                let kind = if node.kind() == "const_spec" {
                    "constant"
                } else {
                    "variable"
                };
                let documentation = node
                    .parent()
                    .and_then(|declaration| self::documentation(builder, declaration))
                    .or_else(|| self::documentation(builder, node));
                let key = builder.add_symbol(
                    node,
                    kind,
                    &name,
                    None,
                    None,
                    None,
                    documentation,
                    is_exported(&name),
                );
                child_enclosing = Some(key);
            }
        }
        "import_spec" => {
            if let Some(path) = builder.field_text(node, "path") {
                builder.add_edge(node, None, "IMPORTS", "module", &unquote(&path), 1.0)?;
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

fn documentation(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    leading_comment(node, &builder.source.content, &COMMENT_KINDS)
        .map(clamp_documentation)
        .filter(|documentation| !documentation.is_empty())
}

/// Go exports by capitalisation rather than by keyword.
fn is_exported(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn receiver_type(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let mut cursor = receiver.walk();
    let declaration = receiver
        .named_children(&mut cursor)
        .find(|child| child.kind() == "parameter_declaration")?;
    let kind = declaration.child_by_field_name("type")?;
    type_name(builder, kind)
}

fn type_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "type_identifier" => builder.text(node),
        "pointer_type" | "generic_type" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find_map(|child| type_name(builder, child))
        }
        _ => None,
    }
}

fn callable_name(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    match node.kind() {
        "identifier" => builder.text(node),
        "selector_expression" => builder.field_text(node, "field"),
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

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '`', '\'']).to_owned()
}
