//! Query rewriting: turning a question into the queries retrieval can serve.
//!
//! This stage is deterministic, per ADR-007. It does no model call, so the
//! same question always produces the same retrieval plan and a failed answer
//! can be reproduced exactly.
//!
//! The stage earns its place because the index's lexical search requires every
//! term to match before it falls back to matching any. A question asked in
//! English ("where do we decide whether to retry a request?") shares almost no
//! tokens with the code that answers it, so the verbatim question is a poor
//! query. Rewriting emits several narrower queries instead, each with a stated
//! purpose, and fuses their results downstream.

use std::collections::BTreeSet;

use mars_indexer::search::SearchFilters;
use serde::{Deserialize, Serialize};

/// Words that carry no retrieval signal. Kept deliberately short: an
/// over-eager stopword list silently deletes the subject of a question.
const STOPWORDS: &[&str] = &[
    "a", "about", "all", "am", "an", "and", "any", "are", "as", "at", "be", "because", "been",
    "but", "by", "can", "did", "do", "does", "doing", "done", "for", "from", "get", "gets", "had",
    "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "of", "on", "or",
    "our", "out", "over", "should", "show", "so", "some", "tell", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "this", "those", "to", "us", "was", "we", "were",
    "what", "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// A small, deliberately modest concept vocabulary. This is not semantic
/// understanding; it is the handful of expansions where the word a person uses
/// and the word in the code reliably differ. Anything broader belongs in a
/// learned model, and pretending otherwise would make the Brain confidently
/// wrong. Each entry maps a term a person types to terms that appear in source.
const CONCEPTS: &[(&str, &[&str])] = &[
    ("auth", &["auth", "authenticate", "credential", "token"]),
    (
        "authentication",
        &["auth", "authenticate", "credential", "token"],
    ),
    ("authorisation", &["authorize", "permission", "grant"]),
    ("authorization", &["authorize", "permission", "grant"]),
    ("cache", &["cache", "cached", "memo"]),
    (
        "config",
        &["config", "configuration", "settings", "options"],
    ),
    (
        "configuration",
        &["config", "configuration", "settings", "options"],
    ),
    ("database", &["database", "sqlite", "storage", "connection"]),
    ("db", &["database", "sqlite", "storage"]),
    ("dependency", &["import", "require", "depends", "module"]),
    ("endpoint", &["route", "handler", "endpoint", "path"]),
    ("error", &["error", "err", "failure", "exception", "panic"]),
    ("failure", &["error", "failure", "exception"]),
    ("log", &["log", "logger", "logging", "trace"]),
    ("logging", &["log", "logger", "logging", "trace"]),
    (
        "permission",
        &["permission", "grant", "capability", "allow"],
    ),
    ("retry", &["retry", "retries", "backoff", "attempt"]),
    ("route", &["route", "router", "handler", "endpoint"]),
    ("setting", &["config", "configuration", "settings"]),
    ("startup", &["init", "initialize", "start", "boot", "main"]),
    ("test", &["test", "spec", "fixture", "assert"]),
    ("timeout", &["timeout", "deadline", "elapsed", "duration"]),
    ("validation", &["validate", "valid", "check", "verify"]),
];

/// Language names a question may mention, mapped to the index's own labels.
const LANGUAGES: &[(&str, &str)] = &[
    ("go", "go"),
    ("golang", "go"),
    ("javascript", "javascript"),
    ("json", "json"),
    ("jsx", "jsx"),
    ("markdown", "markdown"),
    ("python", "python"),
    ("rust", "rust"),
    ("tsx", "tsx"),
    ("typescript", "typescript"),
    ("yaml", "yaml"),
];

/// What the question is asking for. The pipeline uses this to weight ranking
/// and to decide how much graph expansion is worth doing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QueryIntent {
    /// "where is X", "which file holds X" — find a location.
    Locate,
    /// "what does X do", "explain X" — describe a definition.
    Explain,
    /// "what calls X", "what breaks if X changes" — follow the graph.
    Relate,
    /// "how does X happen", "why does X" — trace behaviour across files.
    Behaviour,
    General,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RewritePurpose {
    /// The question as asked, minus punctuation.
    Verbatim,
    /// Only the words that carry signal.
    Keywords,
    /// Identifier-shaped tokens, kept exactly as written.
    Identifiers,
    /// Identifiers split into their component words, so `parseRetryPolicy`
    /// can match a chunk that says "retry policy".
    Subwords,
    /// Concept vocabulary expansion.
    Concept,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewrittenQuery {
    pub text: String,
    pub purpose: RewritePurpose,
    /// Multiplier applied to this query's contribution during fusion. The
    /// verbatim question is the most faithful reading of what was asked;
    /// expansions are progressively more speculative.
    pub weight: f64,
}

/// The full retrieval plan derived from one question.
#[derive(Clone, Debug)]
pub struct QueryPlan {
    pub original: String,
    pub intent: QueryIntent,
    pub queries: Vec<RewrittenQuery>,
    pub filters: SearchFilters,
    /// Distinct content terms, used later to score how well a candidate
    /// matches the question rather than the query that found it.
    pub terms: Vec<String>,
    /// Identifier-shaped tokens the question named, used to prefer symbols the
    /// user actually mentioned.
    pub identifiers: Vec<String>,
}

impl QueryPlan {
    #[must_use]
    pub fn query_texts(&self) -> Vec<String> {
        self.queries
            .iter()
            .map(|query| query.text.clone())
            .collect()
    }
}

/// Builds the retrieval plan for a question.
#[must_use]
pub fn plan(question: &str) -> QueryPlan {
    let question = question.trim();
    let words = words(question);
    let intent = intent(&words);
    let filters = filters(question, &words);

    let identifiers = words
        .iter()
        .filter(|word| is_identifier_shaped(word))
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_owned()
        })
        .filter(|word| !word.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let keywords = words
        .iter()
        .map(|word| word.to_lowercase())
        .filter(|word| !is_stopword(word))
        // A language or path mentioned in the question became a filter; leaving
        // it in the query text as well would double-count it.
        .filter(|word| !is_filter_word(word, &filters))
        .collect::<Vec<_>>();

    let subwords = identifiers
        .iter()
        .flat_map(|identifier| split_identifier(identifier))
        .filter(|word| word.len() > 1)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let concepts = keywords
        .iter()
        .filter_map(|word| lookup_concept(word))
        .flat_map(|expansion| expansion.iter().map(|term| (*term).to_owned()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let mut queries = Vec::new();
    push_query(
        &mut queries,
        question.to_owned(),
        RewritePurpose::Verbatim,
        1.0,
    );
    push_query(
        &mut queries,
        keywords.join(" "),
        RewritePurpose::Keywords,
        0.9,
    );
    push_query(
        &mut queries,
        identifiers.join(" "),
        RewritePurpose::Identifiers,
        0.85,
    );
    push_query(
        &mut queries,
        subwords.join(" "),
        RewritePurpose::Subwords,
        0.6,
    );
    push_query(
        &mut queries,
        concepts.join(" "),
        RewritePurpose::Concept,
        0.5,
    );

    let terms = keywords
        .iter()
        .cloned()
        .chain(subwords.iter().cloned())
        .chain(concepts.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    QueryPlan {
        original: question.to_owned(),
        intent,
        queries,
        filters,
        terms,
        identifiers,
    }
}

/// Adds a query unless it is empty or repeats one already planned. Duplicate
/// queries would double-count the same hits during fusion.
fn push_query(
    queries: &mut Vec<RewrittenQuery>,
    text: String,
    purpose: RewritePurpose,
    weight: f64,
) {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return;
    }
    let normalized = normalize(&text);
    if normalized.is_empty()
        || queries
            .iter()
            .any(|existing| normalize(&existing.text) == normalized)
    {
        return;
    }
    queries.push(RewrittenQuery {
        text,
        purpose,
        weight,
    });
}

fn normalize(text: &str) -> String {
    let mut terms = text
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms.join(" ")
}

fn words(question: &str) -> Vec<String> {
    question
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| {
                !character.is_alphanumeric()
                    && character != '_'
                    && character != '.'
                    && character != '/'
                    && character != ':'
            })
            .to_owned()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_stopword(word: &str) -> bool {
    STOPWORDS.binary_search(&word).is_ok()
}

/// Identifier-shaped means: not an ordinary English word. A camel hump, an
/// underscore, a path separator, or a namespace separator all mark a token the
/// user copied out of the code rather than typed as prose.
fn is_identifier_shaped(word: &str) -> bool {
    let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
    if trimmed.len() < 2 {
        return false;
    }
    let has_inner_capital = trimmed
        .chars()
        .skip(1)
        .any(|character| character.is_ascii_uppercase());
    let has_separator = word.contains('_') || word.contains('/') || word.contains("::");
    let has_digit = trimmed.chars().any(|character| character.is_ascii_digit());
    has_inner_capital || has_separator || has_digit
}

/// Splits `parseRetryPolicy`, `parse_retry_policy`, and `parse-retry-policy`
/// alike into their component words.
fn split_identifier(identifier: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for character in identifier.chars() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_lower = character.is_lowercase() || character.is_ascii_digit();
        current.push(character.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn lookup_concept(word: &str) -> Option<&'static [&'static str]> {
    CONCEPTS
        .binary_search_by_key(&word, |(key, _)| key)
        .ok()
        .map(|position| CONCEPTS[position].1)
}

fn intent(words: &[String]) -> QueryIntent {
    let lowered = words
        .iter()
        .map(|word| word.to_lowercase())
        .collect::<Vec<_>>();
    let has = |value: &str| lowered.iter().any(|word| word == value);

    if has("where") || has("location") || has("find") || has("locate") {
        return QueryIntent::Locate;
    }
    if has("calls") || has("callers") || has("uses") || has("depends") || has("breaks") {
        return QueryIntent::Relate;
    }
    if has("why") || has("happens") || has("flow") || has("when") {
        return QueryIntent::Behaviour;
    }
    if has("what") || has("explain") || has("describe") || has("does") {
        return QueryIntent::Explain;
    }
    QueryIntent::General
}

/// Reads filters the question states outright. Only explicit signals count: a
/// guessed filter silently hides the answer, which is worse than a slow query.
fn filters(question: &str, words: &[String]) -> SearchFilters {
    let mut languages = BTreeSet::new();
    let mut path_prefixes = BTreeSet::new();
    for word in words {
        let lowered = word.to_lowercase();
        if let Ok(position) = LANGUAGES.binary_search_by_key(&lowered.as_str(), |(key, _)| key) {
            languages.insert(LANGUAGES[position].1.to_owned());
        }
        // A token containing a slash and no spaces is a path, either a
        // directory to scope to or a file the user named.
        if lowered.contains('/') && !lowered.starts_with("http") {
            path_prefixes.insert(word.trim_end_matches('/').to_owned());
        }
    }
    // "in the tests", "under src" — a directory named after a preposition.
    let lowered = question.to_lowercase();
    for (phrase, prefix) in [("in tests", "tests"), ("in test", "tests")] {
        if lowered.contains(phrase) {
            path_prefixes.insert(prefix.to_owned());
        }
    }
    SearchFilters {
        path_prefixes: path_prefixes.into_iter().collect(),
        languages: languages.into_iter().collect(),
    }
}

fn is_filter_word(word: &str, filters: &SearchFilters) -> bool {
    LANGUAGES
        .binary_search_by_key(&word, |(key, _)| key)
        .is_ok_and(|position| {
            filters
                .languages
                .iter()
                .any(|language| language == LANGUAGES[position].1)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_and_concepts_stay_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS, "STOPWORDS must stay sorted");

        let mut concepts = CONCEPTS.iter().map(|(key, _)| *key).collect::<Vec<_>>();
        concepts.sort_unstable();
        assert_eq!(
            concepts,
            CONCEPTS.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            "CONCEPTS must stay sorted"
        );

        let mut languages = LANGUAGES.iter().map(|(key, _)| *key).collect::<Vec<_>>();
        languages.sort_unstable();
        assert_eq!(
            languages,
            LANGUAGES.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            "LANGUAGES must stay sorted"
        );
    }

    #[test]
    fn splits_identifiers_into_component_words() {
        assert_eq!(
            split_identifier("parseRetryPolicy"),
            ["parse", "retry", "policy"]
        );
        assert_eq!(
            split_identifier("parse_retry_policy"),
            ["parse", "retry", "policy"]
        );
        assert_eq!(split_identifier("HTTPClient"), ["httpclient"]);
    }
}
