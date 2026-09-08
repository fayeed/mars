//! The Project Brain answer contract.
//!
//! An answer is a structured object, not prose. Its central rule is that a
//! statement of fact and a piece of reasoning are different kinds of thing and
//! are never merged: not in this data model, and not in any surface that
//! renders it. A fact is what a specific span of source says; an inference is
//! what follows from facts. A reader must never have to guess which one they
//! are looking at, so the distinction is a variant rather than a confidence
//! band or a prose hedge.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Whether a claim is read directly off the source or reasoned over claims
/// that are.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimKind {
    /// Directly supported by at least one citation into current file content.
    Fact,
    /// Reasoning over facts. Carries no independent authority.
    Inference,
}

impl ClaimKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Inference => "inference",
        }
    }
}

/// One statement in an answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub id: String,
    pub kind: ClaimKind,
    pub text: String,
    /// Citation ids. A fact with none is invalid and fails validation.
    #[serde(default)]
    pub citations: Vec<String>,
    /// Claim ids an inference reasons over. Always empty for a fact: a fact
    /// that needed another claim to hold it up would be an inference.
    #[serde(default)]
    pub supports: Vec<String>,
    pub confidence: f64,
}

/// A resolvable pointer into the repository. The blob SHA and snippet together
/// let a later validation pass decide whether this citation still describes the
/// file it was drawn from.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub id: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub blob_sha: String,
    pub snippet: String,
    /// Why this span was retrieved, in the pipeline's own words. This is
    /// retrieval provenance, not model prose.
    pub why: String,
}

/// Why the Brain could not answer some part of the question. Naming the reason
/// is the difference between a bounded answer and a wrong one.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GapKind {
    /// Nothing in the index covers this, or the covering file is not indexed.
    NotIndexed,
    /// The question or a term in it resolves several ways.
    Ambiguous,
    /// Answering needs values that exist only when the program runs.
    RequiresRuntime,
    /// Evidence existed but no longer matches the working tree.
    Stale,
    /// Retrieval found material but the context budget could not carry it.
    BudgetExhausted,
    /// Synthesis produced no reasoning over the retrieved facts.
    NoReasoning,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub kind: GapKind,
    /// What could not be determined.
    pub subject: String,
    /// Why not.
    pub detail: String,
}

/// A retrieved source that reached synthesis, with the signals that ranked it.
/// Kept on the answer so a ranking decision can be explained after the fact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerSource {
    pub chunk_id: String,
    pub path: String,
    pub symbol_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub blob_sha: String,
    pub score: f64,
    pub tokens: usize,
    /// Named score contributions, largest first.
    pub signals: Vec<Signal>,
    pub stale: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    pub name: String,
    pub value: f64,
}

/// The complete answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainAnswer {
    pub question: String,
    /// The queries retrieval actually ran, after rewriting.
    pub rewritten_queries: Vec<String>,
    pub claims: Vec<Claim>,
    pub citations: Vec<Citation>,
    /// Sources packed into the synthesis context. Surfaced verbatim in the UI
    /// alongside the latency, so it counts what the answer actually rested on
    /// rather than everything retrieval touched.
    pub sources: usize,
    /// Everything retrieval found, before budget packing dropped any of it.
    pub retrieved: usize,
    pub latency_ms: u64,
    pub confidence: f64,
    pub gaps: Vec<Gap>,
    pub source_detail: Vec<AnswerSource>,
    /// True when budget packing dropped retrieved material.
    pub truncated: bool,
}

/// Ways an answer can violate the contract. Every one of these is a defect in
/// the producer rather than a degraded answer, so the pipeline fails closed
/// instead of emitting something that breaks the fact/inference separation.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum AnswerInvalid {
    #[error("fact claim `{0}` has no citations")]
    UncitedFact(String),
    #[error("fact claim `{0}` rests on other claims, which would make it an inference")]
    FactWithSupports(String),
    #[error("inference `{0}` rests on neither a citation nor another claim")]
    UngroundedInference(String),
    #[error("claim `{claim}` cites `{citation}`, which is not in this answer")]
    UnknownCitation { claim: String, citation: String },
    #[error("inference `{claim}` rests on `{support}`, which is not in this answer")]
    UnknownSupport { claim: String, support: String },
    #[error("claim `{0}` reasons over itself")]
    SelfSupport(String),
    #[error("claim `{0}` has no text")]
    EmptyClaim(String),
    #[error("duplicate claim id `{0}`")]
    DuplicateClaim(String),
    #[error("duplicate citation id `{0}`")]
    DuplicateCitation(String),
    #[error("citation `{0}` has an empty snippet")]
    EmptySnippet(String),
    #[error("citation `{0}` has no blob SHA to resolve against")]
    UnanchoredCitation(String),
    #[error("citation `{id}` spans lines {line_start} to {line_end}")]
    InvalidSpan {
        id: String,
        line_start: usize,
        line_end: usize,
    },
    #[error("{subject} confidence {value} is outside 0.0 to 1.0")]
    ConfidenceOutOfRange { subject: String, value: f64 },
}

impl BrainAnswer {
    /// Checks every rule the contract makes. The pipeline calls this on the
    /// answer it is about to return, so a caller receives a valid answer or an
    /// error, never a quietly broken one.
    pub fn validate(&self) -> Result<(), AnswerInvalid> {
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(AnswerInvalid::ConfidenceOutOfRange {
                subject: "answer".into(),
                value: self.confidence,
            });
        }

        let mut citation_ids = HashSet::new();
        for citation in &self.citations {
            if !citation_ids.insert(citation.id.as_str()) {
                return Err(AnswerInvalid::DuplicateCitation(citation.id.clone()));
            }
            if citation.snippet.trim().is_empty() {
                return Err(AnswerInvalid::EmptySnippet(citation.id.clone()));
            }
            if citation.blob_sha.trim().is_empty() {
                return Err(AnswerInvalid::UnanchoredCitation(citation.id.clone()));
            }
            if citation.line_start == 0 || citation.line_end < citation.line_start {
                return Err(AnswerInvalid::InvalidSpan {
                    id: citation.id.clone(),
                    line_start: citation.line_start,
                    line_end: citation.line_end,
                });
            }
        }

        let mut claim_ids = HashSet::new();
        for claim in &self.claims {
            if !claim_ids.insert(claim.id.as_str()) {
                return Err(AnswerInvalid::DuplicateClaim(claim.id.clone()));
            }
            if claim.text.trim().is_empty() {
                return Err(AnswerInvalid::EmptyClaim(claim.id.clone()));
            }
            if !(0.0..=1.0).contains(&claim.confidence) {
                return Err(AnswerInvalid::ConfidenceOutOfRange {
                    subject: format!("claim `{}`", claim.id),
                    value: claim.confidence,
                });
            }
            for citation in &claim.citations {
                if !citation_ids.contains(citation.as_str()) {
                    return Err(AnswerInvalid::UnknownCitation {
                        claim: claim.id.clone(),
                        citation: citation.clone(),
                    });
                }
            }
            match claim.kind {
                ClaimKind::Fact => {
                    if claim.citations.is_empty() {
                        return Err(AnswerInvalid::UncitedFact(claim.id.clone()));
                    }
                    if !claim.supports.is_empty() {
                        return Err(AnswerInvalid::FactWithSupports(claim.id.clone()));
                    }
                }
                ClaimKind::Inference => {
                    if claim.citations.is_empty() && claim.supports.is_empty() {
                        return Err(AnswerInvalid::UngroundedInference(claim.id.clone()));
                    }
                }
            }
        }

        for claim in &self.claims {
            for support in &claim.supports {
                if support == &claim.id {
                    return Err(AnswerInvalid::SelfSupport(claim.id.clone()));
                }
                if !claim_ids.contains(support.as_str()) {
                    return Err(AnswerInvalid::UnknownSupport {
                        claim: claim.id.clone(),
                        support: support.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn facts(&self) -> impl Iterator<Item = &Claim> {
        self.claims
            .iter()
            .filter(|claim| claim.kind == ClaimKind::Fact)
    }

    pub fn inferences(&self) -> impl Iterator<Item = &Claim> {
        self.claims
            .iter()
            .filter(|claim| claim.kind == ClaimKind::Inference)
    }

    #[must_use]
    pub fn citation(&self, id: &str) -> Option<&Citation> {
        self.citations.iter().find(|citation| citation.id == id)
    }

    /// Drops citations no longer referenced by any claim. Called after the
    /// validation pass removes claims, so the answer never carries evidence for
    /// a statement it no longer makes.
    pub(crate) fn prune_orphan_citations(&mut self) {
        let referenced: HashSet<&str> = self
            .claims
            .iter()
            .flat_map(|claim| claim.citations.iter().map(String::as_str))
            .collect();
        self.citations
            .retain(|citation| referenced.contains(citation.id.as_str()));
    }

    /// Answer confidence is the mean claim confidence, weighted so that facts
    /// count for more than the reasoning stacked on them, and reduced when the
    /// Brain had to declare gaps.
    pub(crate) fn recompute_confidence(&mut self) {
        if self.claims.is_empty() {
            self.confidence = 0.0;
            return;
        }
        let mut total = 0.0;
        let mut weight = 0.0;
        for claim in &self.claims {
            let claim_weight = match claim.kind {
                ClaimKind::Fact => 1.0,
                ClaimKind::Inference => 0.5,
            };
            total += claim.confidence * claim_weight;
            weight += claim_weight;
        }
        let mean = if weight > 0.0 { total / weight } else { 0.0 };
        // Each declared gap is a piece of the question left unanswered, capped
        // so that a thorough gap list cannot drive a well-evidenced answer to
        // zero.
        let penalty = (self.gaps.len() as f64 * 0.05).min(0.3);
        self.confidence = (mean - penalty).clamp(0.0, 1.0);
    }
}

/// Assigns stable, order-independent ids so that two runs over the same
/// evidence produce the same answer document.
pub(crate) struct IdAllocator {
    prefix: &'static str,
    next: usize,
    assigned: HashMap<String, String>,
}

impl IdAllocator {
    pub(crate) fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            next: 0,
            assigned: HashMap::new(),
        }
    }

    /// Returns the id already assigned to this key, or mints the next one.
    pub(crate) fn id_for(&mut self, key: &str) -> String {
        if let Some(existing) = self.assigned.get(key) {
            return existing.clone();
        }
        let id = format!("{}:{}", self.prefix, self.next);
        self.next += 1;
        self.assigned.insert(key.to_owned(), id.clone());
        id
    }
}
