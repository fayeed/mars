//! Symbol extraction for configuration and prose: JSON, YAML, and Markdown.
//!
//! These carry no call graph. JSON and YAML contribute a keyed property tree so
//! `symbols.at` can name the setting under a cursor; Markdown contributes its
//! heading hierarchy so prose chunks can be addressed by section.

use tree_sitter::Node;

use crate::IndexError;
use crate::facts::Builder;

/// Deeply nested configuration adds noise without adding navigational value.
const MAX_PROPERTY_DEPTH: usize = 6;

/// Dependency manifests whose keys name external packages.
const DEPENDENCY_SECTIONS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

pub(crate) fn extract_json(builder: &mut Builder<'_>, root: Node<'_>) -> Result<(), IndexError> {
    let is_manifest = builder
        .source
        .relative_path
        .rsplit('/')
        .next()
        .is_some_and(|name| name == "package.json");
    visit_json(builder, root, None, None, 0, is_manifest)
}

fn visit_json(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    path: Option<&str>,
    parent: Option<&str>,
    depth: usize,
    is_manifest: bool,
) -> Result<(), IndexError> {
    let mut child_path = path.map(str::to_owned);
    let mut child_parent = parent.map(str::to_owned);

    if node.kind() == "pair" {
        if depth > MAX_PROPERTY_DEPTH {
            return Ok(());
        }
        if let Some(key) = json_key(builder, node) {
            let qualified = path.map_or_else(|| key.clone(), |path| format!("{path}.{key}"));
            let key_node = node.child_by_field_name("key").unwrap_or(node);
            let value = node.child_by_field_name("value");
            let kind = match value.map(|value| value.kind()) {
                Some("object") => "object",
                Some("array") => "array",
                _ => "property",
            };
            let stable_key = builder.add_symbol(
                node,
                kind,
                &key,
                path,
                parent,
                value
                    .filter(|value| !matches!(value.kind(), "object" | "array"))
                    .and_then(|value| builder.text(value)),
                None,
                true,
            );
            // package.json dependency sections name real external packages.
            if is_manifest && path.is_some_and(|path| DEPENDENCY_SECTIONS.contains(&path)) {
                builder.add_edge(key_node, None, "DEPENDS_ON", "module", &key, 1.0)?;
            }
            child_path = Some(qualified);
            child_parent = Some(stable_key);
        }
    }

    let next_depth = if node.kind() == "pair" {
        depth + 1
    } else {
        depth
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_json(
            builder,
            child,
            child_path.as_deref(),
            child_parent.as_deref(),
            next_depth,
            is_manifest,
        )?;
    }
    Ok(())
}

fn json_key(builder: &Builder<'_>, node: Node<'_>) -> Option<String> {
    let key = node.child_by_field_name("key")?;
    let mut cursor = key.walk();
    key.named_children(&mut cursor)
        .find(|child| child.kind() == "string_content")
        .and_then(|content| builder.text(content))
        .or_else(|| builder.text(key).map(|text| unquote(&text)))
}

pub(crate) fn extract_yaml(builder: &mut Builder<'_>, root: Node<'_>) -> Result<(), IndexError> {
    visit_yaml(builder, root, None, None, 0)
}

fn visit_yaml(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    path: Option<&str>,
    parent: Option<&str>,
    depth: usize,
) -> Result<(), IndexError> {
    let mut child_path = path.map(str::to_owned);
    let mut child_parent = parent.map(str::to_owned);

    if node.kind() == "block_mapping_pair" {
        if depth > MAX_PROPERTY_DEPTH {
            return Ok(());
        }
        if let Some(key) = builder
            .field_text(node, "key")
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
        {
            let qualified = path.map_or_else(|| key.clone(), |path| format!("{path}.{key}"));
            let value = node.child_by_field_name("value");
            let is_scalar = value.is_some_and(|value| {
                let mut cursor = value.walk();
                !value
                    .named_children(&mut cursor)
                    .any(|child| matches!(child.kind(), "block_mapping" | "block_sequence"))
            });
            let stable_key = builder.add_symbol(
                node,
                if is_scalar { "property" } else { "object" },
                &key,
                path,
                parent,
                value
                    .filter(|_| is_scalar)
                    .and_then(|value| builder.text(value))
                    .map(|text| text.trim().to_owned()),
                None,
                true,
            );
            child_path = Some(qualified);
            child_parent = Some(stable_key);
        }
    }

    let next_depth = if node.kind() == "block_mapping_pair" {
        depth + 1
    } else {
        depth
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_yaml(
            builder,
            child,
            child_path.as_deref(),
            child_parent.as_deref(),
            next_depth,
        )?;
    }
    Ok(())
}

pub(crate) fn extract_markdown(
    builder: &mut Builder<'_>,
    root: Node<'_>,
) -> Result<(), IndexError> {
    visit_markdown(builder, root, None, None)
}

/// The Markdown grammar nests a `section` per heading, so the enclosing symbol
/// falls out of the tree shape rather than needing a level stack.
fn visit_markdown(
    builder: &mut Builder<'_>,
    node: Node<'_>,
    path: Option<&str>,
    parent: Option<&str>,
) -> Result<(), IndexError> {
    let mut child_path = path.map(str::to_owned);
    let mut child_parent = parent.map(str::to_owned);

    if node.kind() == "section"
        && let Some(heading) = section_heading(node)
        && let Some(title) = heading_text(builder, heading)
    {
        let qualified = path.map_or_else(|| title.clone(), |path| format!("{path}.{title}"));
        let stable_key = builder.add_symbol(
            node,
            "heading",
            &title,
            path,
            parent,
            Some(heading_marker(heading).to_owned()),
            None,
            true,
        );
        child_path = Some(qualified);
        child_parent = Some(stable_key);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_markdown(
            builder,
            child,
            child_path.as_deref(),
            child_parent.as_deref(),
        )?;
    }
    Ok(())
}

fn section_heading(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "atx_heading" | "setext_heading"))
}

fn heading_text(builder: &Builder<'_>, heading: Node<'_>) -> Option<String> {
    let content = heading.child_by_field_name("heading_content").or_else(|| {
        let mut cursor = heading.walk();
        heading
            .named_children(&mut cursor)
            .find(|child| child.kind() == "inline")
    })?;
    builder
        .text(content)
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn heading_marker(heading: Node<'_>) -> &'static str {
    let mut cursor = heading.walk();
    let level = heading
        .children(&mut cursor)
        .find_map(|child| match child.kind() {
            "atx_h1_marker" => Some(1),
            "atx_h2_marker" => Some(2),
            "atx_h3_marker" => Some(3),
            "atx_h4_marker" => Some(4),
            "atx_h5_marker" => Some(5),
            "atx_h6_marker" => Some(6),
            _ => None,
        })
        .unwrap_or(1);
    match level {
        1 => "h1",
        2 => "h2",
        3 => "h3",
        4 => "h4",
        5 => "h5",
        _ => "h6",
    }
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).to_owned()
}
