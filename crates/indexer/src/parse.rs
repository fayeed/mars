//! Parses one file into symbol and edge facts using the grammar for its language.

use tree_sitter::Parser;

use crate::IndexError;
use crate::facts::{Builder, ParsedFile, count_errors};
use crate::languages::Language;
use crate::walk::SourceFile;
use crate::{golang, javascript, python, rustlang, structured};

pub fn parse(source: SourceFile) -> Result<ParsedFile, IndexError> {
    let Some(language) = source.parsed_language().filter(|_| source.parseable) else {
        // Files outside the supported set still belong to the file layer.
        return Ok(ParsedFile {
            source,
            symbols: Vec::new(),
            edges: Vec::new(),
            parse_errors: 0,
        });
    };

    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .map_err(|error| IndexError::Parse(error.to_string()))?;
    let tree = parser
        .parse(&source.content, None)
        .ok_or_else(|| IndexError::Parse("parser returned no tree".into()))?;
    let parse_errors = count_errors(tree.root_node());

    let mut builder = Builder::new(&source);
    let root = tree.root_node();
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => {
            javascript::extract(&mut builder, root)?;
        }
        Language::Python => python::extract(&mut builder, root)?,
        Language::Go => golang::extract(&mut builder, root)?,
        Language::Rust => rustlang::extract(&mut builder, root)?,
        Language::Json => structured::extract_json(&mut builder, root)?,
        Language::Yaml => structured::extract_yaml(&mut builder, root)?,
        Language::Markdown => structured::extract_markdown(&mut builder, root)?,
    }
    let (symbols, edges, parse_errors) = builder.finish(parse_errors);

    Ok(ParsedFile {
        source,
        symbols,
        edges,
        parse_errors,
    })
}
