//! Symbol-aware chunking.
//!
//! A chunk never splits a definition mid-body: a symbol that exceeds the target
//! size is broken along its own nested definitions, and only the material
//! between definitions is ever split on line boundaries.

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::facts::{ParsedFile, SymbolFact};

/// Target upper bound for a chunk. Definitions larger than this are split along
/// their nested definitions; a leaf definition is kept whole regardless.
const TARGET_CHUNK_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub struct ChunkFact {
    pub id: String,
    pub ordinal: usize,
    pub symbol_stable_key: Option<String>,
    pub symbol_path: String,
    pub header: String,
    pub body: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub content_hash: String,
}

impl ChunkFact {
    /// The text an embedding or lexical index should see: the location header
    /// travels with the body so a retrieved chunk can always be placed.
    #[must_use]
    pub fn embedded_text(&self) -> String {
        format!("{}\n{}", self.header, self.body)
    }
}

/// Splits one parsed file into chunks anchored on its symbols.
#[must_use]
pub fn chunk_file(repository_id: &str, parsed: &ParsedFile) -> Vec<ChunkFact> {
    let Some(text) = parsed.source.text() else {
        return Vec::new();
    };
    if !parsed.source.parseable || text.trim().is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    collect(parsed, None, 0, text.len(), &mut spans);

    let mut chunks = Vec::new();
    let mut used_ids = HashSet::new();
    for span in spans {
        let body = text
            .get(span.start_byte..span.end_byte)
            .unwrap_or_default()
            .to_owned();
        if body.trim().is_empty() {
            continue;
        }
        let symbol_path = span.symbol_path.clone();
        let header = if symbol_path.is_empty() {
            parsed.source.relative_path.clone()
        } else {
            format!("{} :: {symbol_path}", parsed.source.relative_path)
        };
        let content_hash = hash(&body);
        let id = stable_id(
            repository_id,
            &parsed.source.relative_path,
            &symbol_path,
            &content_hash,
            &mut used_ids,
        );
        chunks.push(ChunkFact {
            id,
            ordinal: chunks.len(),
            symbol_stable_key: span.symbol_stable_key,
            symbol_path,
            header,
            start_line: line_of(text, span.start_byte),
            end_line: line_of(text, span.end_byte.saturating_sub(1)),
            start_byte: span.start_byte,
            end_byte: span.end_byte,
            content_hash,
            body,
        });
    }
    chunks
}

#[derive(Debug)]
struct Span {
    symbol_stable_key: Option<String>,
    symbol_path: String,
    start_byte: usize,
    end_byte: usize,
}

/// Emits the spans covering `[start, end)`, descending into a definition only
/// when it is too large to be one chunk.
fn collect(
    parsed: &ParsedFile,
    parent: Option<&SymbolFact>,
    start: usize,
    end: usize,
    spans: &mut Vec<Span>,
) {
    let children = direct_children(parsed, parent, start, end);
    let mut cursor = start;

    for child in &children {
        if child.start_byte > cursor {
            push_interstitial(parsed, parent, cursor, child.start_byte, spans);
        }
        let size = child.end_byte.saturating_sub(child.start_byte);
        let grandchildren = direct_children(parsed, Some(child), child.start_byte, child.end_byte);
        if size > TARGET_CHUNK_BYTES && !grandchildren.is_empty() {
            // Too large to embed whole, but it has nested definitions to split on.
            collect(parsed, Some(child), child.start_byte, child.end_byte, spans);
        } else {
            spans.push(Span {
                symbol_stable_key: Some(child.stable_key.clone()),
                symbol_path: symbol_path(parsed, child),
                start_byte: child.start_byte,
                end_byte: child.end_byte,
            });
        }
        cursor = cursor.max(child.end_byte);
    }

    if cursor < end {
        push_interstitial(parsed, parent, cursor, end, spans);
    }
}

/// Material between definitions — imports, constants, prose — split only on
/// line boundaries.
fn push_interstitial(
    parsed: &ParsedFile,
    parent: Option<&SymbolFact>,
    start: usize,
    end: usize,
    spans: &mut Vec<Span>,
) {
    let Some(text) = parsed.source.text() else {
        return;
    };
    let symbol_path = parent.map(|parent| symbol_path(parsed, parent)).unwrap_or_default();
    let mut cursor = start;
    while cursor < end {
        let limit = (cursor + TARGET_CHUNK_BYTES).min(end);
        let boundary = if limit >= end {
            end
        } else {
            text[cursor..limit]
                .rfind('\n')
                .map_or(limit, |offset| cursor + offset + 1)
        };
        let boundary = if boundary <= cursor { limit } else { boundary };
        if text[cursor..boundary].trim().is_empty() {
            cursor = boundary;
            continue;
        }
        spans.push(Span {
            symbol_stable_key: parent.map(|parent| parent.stable_key.clone()),
            symbol_path: symbol_path.clone(),
            start_byte: cursor,
            end_byte: boundary,
        });
        cursor = boundary;
    }
}

/// The definitions directly inside `parent`, in source order, with any symbol
/// nested inside an earlier sibling dropped.
fn direct_children<'a>(
    parsed: &'a ParsedFile,
    parent: Option<&SymbolFact>,
    start: usize,
    end: usize,
) -> Vec<&'a SymbolFact> {
    let parent_key = parent.map(|parent| parent.stable_key.as_str());
    let mut children: Vec<&SymbolFact> = parsed
        .symbols
        .iter()
        .filter(|symbol| symbol.parent_stable_key.as_deref() == parent_key)
        .filter(|symbol| symbol.start_byte >= start && symbol.end_byte <= end)
        .filter(|symbol| symbol.end_byte > symbol.start_byte)
        .collect();
    children.sort_by_key(|symbol| (symbol.start_byte, std::cmp::Reverse(symbol.end_byte)));

    let mut kept: Vec<&SymbolFact> = Vec::new();
    for child in children {
        if kept
            .last()
            .is_some_and(|previous| child.start_byte < previous.end_byte)
        {
            continue;
        }
        kept.push(child);
    }
    kept
}

fn symbol_path(parsed: &ParsedFile, symbol: &SymbolFact) -> String {
    let mut segments = vec![symbol.qualified_name.clone()];
    let mut current = symbol.parent_stable_key.clone();
    // Guard against a malformed parent cycle from a partially parsed file.
    let mut depth = 0;
    while let Some(key) = current {
        if depth > 16 {
            break;
        }
        let Some(parent) = parsed
            .symbols
            .iter()
            .find(|candidate| candidate.stable_key == key)
        else {
            break;
        };
        if parent.qualified_name != segments[segments.len() - 1] {
            segments.push(parent.qualified_name.clone());
        }
        current = parent.parent_stable_key.clone();
        depth += 1;
    }
    segments.reverse();
    segments.join(" / ")
}

/// A chunk keeps its identity as long as its content and location are unchanged,
/// independent of edits elsewhere in the file.
fn stable_id(
    repository_id: &str,
    relative_path: &str,
    symbol_path: &str,
    content_hash: &str,
    used: &mut HashSet<String>,
) -> String {
    let mut attempt = 0;
    loop {
        let seed = if attempt == 0 {
            format!("{repository_id}\0{relative_path}\0{symbol_path}\0{content_hash}")
        } else {
            format!("{repository_id}\0{relative_path}\0{symbol_path}\0{content_hash}\0{attempt}")
        };
        let id = hash(&seed);
        if used.insert(id.clone()) {
            return id;
        }
        attempt += 1;
    }
}

fn hash(value: &str) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(value.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn line_of(text: &str, byte: usize) -> usize {
    let clamped = byte.min(text.len());
    text[..clamped].bytes().filter(|byte| *byte == b'\n').count() + 1
}
