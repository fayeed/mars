//! Token-aware context budget packing.
//!
//! Two rules govern what reaches synthesis. A symbol is packed whole or not at
//! all: half a function is worse than no function, because a model reading a
//! truncated body will confidently describe behaviour that is not there and
//! cite a line range that never contained it. And material is dropped by
//! lowest marginal value rather than by lowest score, so a fifth chunk that
//! repeats the four already packed loses to a lower-scoring chunk that says
//! something new.

use std::collections::HashSet;

use crate::gather::Candidate;

/// Minimum useful budget. Below this there is no room for even one symbol plus
/// the question, and a caller asking for less has made a mistake worth
/// reporting rather than silently rounding away.
pub const MIN_BUDGET_TOKENS: usize = 256;
pub const MAX_BUDGET_TOKENS: usize = 200_000;
pub const DEFAULT_BUDGET_TOKENS: usize = 8_000;

#[derive(Clone, Debug)]
pub(crate) struct PackedSource {
    pub candidate: Candidate,
    pub tokens: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Dropped {
    pub path: String,
    pub symbol_path: String,
    pub reason: String,
}

pub(crate) struct PackedContext {
    pub sources: Vec<PackedSource>,
    pub tokens: usize,
    pub dropped: Vec<Dropped>,
}

impl PackedContext {
    pub(crate) fn truncated(&self) -> bool {
        !self.dropped.is_empty()
    }
}

/// Packs candidates into `budget_tokens`, keeping each symbol whole.
///
/// Selection is greedy on marginal value: at each step it takes the candidate
/// with the highest score-per-token *after* discounting what the already
/// selected sources cover. Novelty is recomputed after every pick, which is
/// what stops a cluster of near-identical chunks from consuming the budget.
pub(crate) fn pack(
    candidates: Vec<Candidate>,
    budget_tokens: usize,
    max_sources: usize,
) -> PackedContext {
    let mut remaining: Vec<PackedSource> = candidates
        .into_iter()
        .map(|candidate| {
            let tokens = estimate_tokens(&candidate.content());
            PackedSource { candidate, tokens }
        })
        .collect();

    let mut selected: Vec<PackedSource> = Vec::new();
    let mut dropped: Vec<Dropped> = Vec::new();
    let mut covered: HashSet<String> = HashSet::new();
    let mut used = 0_usize;

    while !remaining.is_empty() && selected.len() < max_sources {
        let mut best: Option<(usize, f64)> = None;
        for (position, source) in remaining.iter().enumerate() {
            if used + source.tokens > budget_tokens {
                continue;
            }
            let value = marginal_value(source, &covered);
            if value <= 0.0 {
                continue;
            }
            if best.is_none_or(|(_, best_value)| value > best_value) {
                best = Some((position, value));
            }
        }
        let Some((position, _)) = best else {
            break;
        };
        let source = remaining.remove(position);
        used += source.tokens;
        covered.extend(content_terms(&source.candidate.content()));
        selected.push(source);
    }

    for source in remaining {
        let reason = if source.tokens > budget_tokens {
            format!(
                "the symbol needs {} tokens, more than the whole {budget_tokens}-token budget, and is never split",
                source.tokens
            )
        } else if used + source.tokens > budget_tokens {
            format!(
                "the remaining {} tokens cannot hold this symbol whole",
                budget_tokens.saturating_sub(used)
            )
        } else if selected.len() >= max_sources {
            format!("the answer already carries the {max_sources}-source maximum")
        } else {
            "everything it says is already covered by a higher-ranked source".to_owned()
        };
        dropped.push(Dropped {
            path: source.candidate.path.clone(),
            symbol_path: source.candidate.symbol_path.clone(),
            reason,
        });
    }

    PackedContext {
        sources: selected,
        tokens: used,
        dropped,
    }
}

/// Score per token, discounted by how much of this source is already covered.
fn marginal_value(source: &PackedSource, covered: &HashSet<String>) -> f64 {
    if source.tokens == 0 {
        return 0.0;
    }
    let terms = content_terms(&source.candidate.content());
    let novelty = if terms.is_empty() {
        // No distinctive terms at all: the source is boilerplate, worth
        // something but never worth displacing substantive code.
        0.1
    } else {
        let fresh = terms.iter().filter(|term| !covered.contains(*term)).count();
        fresh as f64 / terms.len() as f64
    };
    // A source that repeats everything already packed still carries a little
    // value as corroboration, so novelty floors rather than zeroes it out.
    let novelty = novelty.max(0.05);
    source.candidate.score * novelty / source.tokens as f64
}

/// Distinctive terms of a chunk, used to measure redundancy. Short tokens are
/// dropped because `if`, `let`, and `fn` appear in every chunk and would make
/// everything look identical.
fn content_terms(content: &str) -> HashSet<String> {
    content
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| term.len() > 3)
        .map(str::to_lowercase)
        .collect()
}

/// Estimates the token cost of a string.
///
/// This is a deterministic approximation, not a tokenizer: Mars must not
/// depend on a provider's vocabulary to decide what fits, and every provider
/// counts differently anyway. It models the shape most byte-pair vocabularies
/// share — roughly four characters per token inside a word, and one token for
/// each piece of punctuation — and errs slightly high so a packed context does
/// not overrun a real limit.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0_usize;
    let mut run = 0_usize;
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            run += 1;
        } else {
            if run > 0 {
                tokens += run.div_ceil(4);
                run = 0;
            }
            if !character.is_whitespace() {
                tokens += 1;
            }
        }
    }
    if run > 0 {
        tokens += run.div_ceil(4);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_tokens_from_word_and_punctuation_shape() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 2);
        // call/left/right cost 1, 1, and 2; the four punctuation marks cost one
        // each, and the space costs nothing.
        assert_eq!(estimate_tokens("call(left, right);"), 8);
    }

    #[test]
    fn whitespace_alone_costs_nothing() {
        assert_eq!(estimate_tokens("   \n\t  "), 0);
    }
}
