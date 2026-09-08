//! The validation pass: resolving every citation back to current file content.
//!
//! Retrieval reads the index, and the index is a projection that can lag the
//! working tree. Between the moment a chunk was indexed and the moment an
//! answer is rendered, its file may have been edited, moved, or deleted. This
//! pass is what stops that gap from turning into a confident false statement:
//! it opens each cited file as it is on disk right now and asks whether the
//! cited text is still there.
//!
//! Three outcomes. The span still holds, so the citation stands. The text
//! moved within its file, so the citation is relabelled to where it now is.
//! The text is gone, so the citation is dropped — and with it any fact that
//! rested on nothing else, because a fact without a citation is not a weaker
//! fact, it is not a fact at all.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::answer::{BrainAnswer, ClaimKind, Gap, GapKind};

/// How far the same text may have moved within its file and still be
/// recognised as the same text. Beyond this the match is more likely to be a
/// coincidental repetition — a second copy of a common helper — than the
/// original span shifted by edits.
const MAX_DRIFT_LINES: usize = 400;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CitationOutcome {
    Kept,
    Relabelled { from: usize, to: usize },
    Dropped { reason: String },
}

#[derive(Debug, Default)]
pub(crate) struct ValidationReport {
    pub outcomes: Vec<(String, CitationOutcome)>,
    pub dropped_claims: Vec<String>,
}

impl ValidationReport {
    pub(crate) fn dropped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, CitationOutcome::Dropped { .. }))
            .count()
    }
}

/// Resolves every citation in `answer` against the working tree at `root`,
/// mutating the answer in place. `current_sha` supplies the blob SHA a path
/// currently hashes to, so a relabelled citation is re-anchored to the content
/// it was actually verified against rather than the content it was indexed
/// from.
pub(crate) fn resolve_citations(
    answer: &mut BrainAnswer,
    root: &Path,
    current_sha: &HashMap<String, String>,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut files: HashMap<String, Option<Vec<String>>> = HashMap::new();
    let mut surviving = Vec::new();

    for mut citation in std::mem::take(&mut answer.citations) {
        let lines = files
            .entry(citation.path.clone())
            .or_insert_with(|| read_lines(root, &citation.path));

        let Some(lines) = lines.as_ref() else {
            report.outcomes.push((
                citation.id.clone(),
                CitationOutcome::Dropped {
                    reason: format!(
                        "{} is no longer readable in the working tree",
                        citation.path
                    ),
                },
            ));
            continue;
        };

        match locate(lines, &citation.snippet, citation.line_start) {
            Some(found) if found == citation.line_start => {
                if let Some(sha) = current_sha.get(&citation.path) {
                    citation.blob_sha.clone_from(sha);
                }
                report
                    .outcomes
                    .push((citation.id.clone(), CitationOutcome::Kept));
                surviving.push(citation);
            }
            Some(found) => {
                let span = citation.line_end.saturating_sub(citation.line_start);
                let from = citation.line_start;
                citation.line_start = found;
                citation.line_end = found + span;
                // A reader following this citation should know the line numbers
                // are not the ones retrieval produced, and why. Only an actual
                // change of blob may be blamed on an edit; the span can also
                // move because the index recorded a byte offset that no longer
                // lines up, and saying "the file changed" then would be false.
                let edited = current_sha
                    .get(&citation.path)
                    .is_some_and(|sha| sha != &citation.blob_sha);
                if let Some(sha) = current_sha.get(&citation.path) {
                    citation.blob_sha.clone_from(sha);
                }
                citation.why.push_str(&if edited {
                    format!(" (relabelled from line {from}: the file changed since it was indexed)")
                } else {
                    format!(" (relabelled from line {from}: the indexed span no longer lines up)")
                });
                report.outcomes.push((
                    citation.id.clone(),
                    CitationOutcome::Relabelled { from, to: found },
                ));
                surviving.push(citation);
            }
            None => {
                report.outcomes.push((
                    citation.id.clone(),
                    CitationOutcome::Dropped {
                        reason: format!("the cited text is no longer present in {}", citation.path),
                    },
                ));
            }
        }
    }
    answer.citations = surviving;

    let live: HashSet<String> = answer
        .citations
        .iter()
        .map(|citation| citation.id.clone())
        .collect();
    for claim in &mut answer.claims {
        claim.citations.retain(|id| live.contains(id));
    }

    // A fact whose evidence vanished is removed rather than demoted. Relabelling
    // it as an inference would be a lie of a different shape: nothing in the
    // answer now supports it.
    prune_unsupported(answer, &mut report);

    if report.dropped() > 0 || !report.dropped_claims.is_empty() {
        answer.gaps.push(Gap {
            kind: GapKind::Stale,
            subject: "evidence that changed since it was indexed".into(),
            detail: format!(
                "{} citation(s) no longer resolved against the working tree and {} claim(s) \
                 were dropped for lack of surviving evidence.",
                report.dropped(),
                report.dropped_claims.len()
            ),
        });
    }
    report
}

/// Repeatedly removes claims left standing on nothing, since dropping one
/// fact can strand the inference above it.
fn prune_unsupported(answer: &mut BrainAnswer, report: &mut ValidationReport) {
    loop {
        let live: HashSet<String> = answer.claims.iter().map(|claim| claim.id.clone()).collect();
        let doomed: Vec<String> = answer
            .claims
            .iter()
            .filter(|claim| match claim.kind {
                ClaimKind::Fact => claim.citations.is_empty(),
                ClaimKind::Inference => {
                    let supports: Vec<&String> = claim
                        .supports
                        .iter()
                        .filter(|id| live.contains(*id))
                        .collect();
                    claim.citations.is_empty() && supports.is_empty()
                }
            })
            .map(|claim| claim.id.clone())
            .collect();
        if doomed.is_empty() {
            break;
        }
        answer.claims.retain(|claim| !doomed.contains(&claim.id));
        for claim in &mut answer.claims {
            claim.supports.retain(|id| !doomed.contains(id));
        }
        report.dropped_claims.extend(doomed);
    }
}

fn read_lines(root: &Path, relative_path: &str) -> Option<Vec<String>> {
    // The path came from the index, which only ever records repository-relative
    // paths, but a traversal component would still be a path escape. Refusing
    // is cheaper than proving it cannot happen.
    if relative_path
        .split('/')
        .any(|segment| segment == ".." || segment.is_empty())
    {
        return None;
    }
    let mut absolute = root.to_path_buf();
    for segment in relative_path.split('/') {
        absolute.push(segment);
    }
    let content = fs::read_to_string(&absolute).ok()?;
    Some(content.lines().map(str::to_owned).collect())
}

/// Finds where the snippet's significant lines now sit, preferring the
/// original position so an unchanged file is confirmed rather than searched.
/// Returns a 1-based line number.
fn locate(lines: &[String], snippet: &str, expected_start: usize) -> Option<usize> {
    let needle: Vec<&str> = snippet
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if needle.is_empty() {
        return None;
    }

    if matches_at(lines, &needle, expected_start) {
        return Some(expected_start);
    }

    // Search outward from where it used to be: the nearest identical text is
    // the likeliest continuation of the span that moved.
    let lowest = expected_start.saturating_sub(MAX_DRIFT_LINES).max(1);
    let highest = (expected_start + MAX_DRIFT_LINES).min(lines.len().max(1));
    let mut best: Option<usize> = None;
    for start in lowest..=highest {
        if matches_at(lines, &needle, start) {
            let distance = start.abs_diff(expected_start);
            if best.is_none_or(|current| distance < current.abs_diff(expected_start)) {
                best = Some(start);
            }
        }
    }
    best
}

/// True when every significant line of the needle appears in order from
/// `start`, ignoring blank lines and indentation changes. Reformatting should
/// not invalidate a citation; deleting the code should.
///
/// The first and last lines are matched partially, because a chunk is a byte
/// range rather than a run of whole lines: the indexer's span for a symbol
/// starts at the definition itself, so a chunk for an exported function begins
/// at `function add(...)` while the file line reads `export function add(...)`.
/// Interior lines are wholly inside the span and must match exactly.
fn matches_at(lines: &[String], needle: &[&str], start: usize) -> bool {
    if start == 0 || start > lines.len() || needle.is_empty() {
        return false;
    }
    let last = needle.len() - 1;
    let mut cursor = start - 1;
    for (position, expected) in needle.iter().enumerate() {
        loop {
            if cursor >= lines.len() {
                return false;
            }
            let actual = lines[cursor].trim();
            cursor += 1;
            if actual.is_empty() {
                continue;
            }
            let matched = match (position == 0, position == last) {
                // A one-line span may be partial at both ends.
                (true, true) => actual.contains(expected),
                // The span opens part-way through this line.
                (true, false) => actual.ends_with(expected),
                // The span closes part-way through this line.
                (false, true) => actual.starts_with(expected),
                (false, false) => actual == *expected,
            };
            if !matched {
                return false;
            }
            break;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn confirms_a_span_that_has_not_moved() {
        let file = lines("one\ntwo\nthree\nfour\n");
        assert_eq!(locate(&file, "two\nthree", 2), Some(2));
    }

    #[test]
    fn relabels_a_span_that_moved_down() {
        let file = lines("added\nadded\none\ntwo\nthree\n");
        assert_eq!(locate(&file, "two\nthree", 2), Some(4));
    }

    #[test]
    fn tolerates_reindentation_and_blank_lines() {
        let file = lines("one\n    two\n\n  three\n");
        assert_eq!(locate(&file, "two\nthree", 2), Some(2));
    }

    #[test]
    fn matches_a_span_that_opens_and_closes_mid_line() {
        // What the indexer stores for an exported symbol: the span begins at
        // the definition, after `export `, and can end before a trailing brace.
        let file = lines(
            "export class Calculator extends Base {
  run(): void {}
}
",
        );
        assert_eq!(
            locate(
                &file,
                "class Calculator extends Base {
  run(): void {}
}",
                1
            ),
            Some(1)
        );
    }

    #[test]
    fn refuses_an_interior_line_that_changed() {
        let file = lines(
            "export function add(a, b) {
  return a - b;
}
",
        );
        assert_eq!(
            locate(
                &file,
                "function add(a, b) {
  return a + b;
}",
                1
            ),
            None,
            "a changed body must not still validate"
        );
    }

    #[test]
    fn refuses_a_span_whose_text_is_gone() {
        let file = lines("one\ntwo\n");
        assert_eq!(locate(&file, "three\nfour", 1), None);
    }
}
