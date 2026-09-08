pub mod answer;
pub mod budget;
pub mod context_pack;
pub mod extraction;
mod gather;
pub mod knowledge;
pub mod project_brain;
pub mod query;
pub mod retrieval;
pub mod review;
pub mod rewrite;
pub mod synthesis;
pub mod tools;
mod validate;
pub mod why_query;

pub use answer::{
    AnswerInvalid, AnswerSource, BrainAnswer, Citation, Claim, ClaimKind, Gap, GapKind, Signal,
};
pub use budget::{DEFAULT_BUDGET_TOKENS, MAX_BUDGET_TOKENS, MIN_BUDGET_TOKENS, estimate_tokens};
pub use project_brain::{
    AskError, AskRequest, BrainEvent, DEFAULT_MAX_SOURCES, DependencyEdge, DependencyNode,
    DependencySlice, DependencyTarget, ProjectBrain,
};
pub use rewrite::{QueryIntent, QueryPlan, RewritePurpose, RewrittenQuery};
pub use synthesis::{
    DeterministicSynthesizer, SYNTHESIS_INSTRUCTIONS, SynthesisOutput, SynthesisProvider,
    SynthesisRequest, SynthesisSource, SynthesizedClaim, SynthesizedGap, render_synthesis_prompt,
};

pub use context_pack::{
    ContextPack, ContextPackError, ContextPackItem, ContextPackRequest, ContextPackService,
    PackExclusion, PackSection, SectionUsage, render_context_pack,
};
pub use extraction::{
    AffectedEntityDraft, ChangedSymbolEvidence, ClaimDraft, DiffEvidence, EvidenceBundle,
    EvidenceEvent, ExtractionError, ExtractionProvider, ExtractionRequest, ExtractionResult,
    ExtractionService, ManualMemoryDraft, MemoryCategory, render_extraction_prompt,
};
pub use knowledge::{
    EntityKind, EntityLink, EntityRef, KnowledgeError, KnowledgeKind, KnowledgeRecord,
    KnowledgeService, KnowledgeStatus, NewKnowledge, NewProvenance, Provenance, ProvenanceKind,
};
pub use query::{BrainError, BrainService};
pub use review::{
    FindingSeverity, FindingStatus, HandoffEvent, ReviewDraft, ReviewError, ReviewFinding,
    ReviewFindingDraft, ReviewHandoff, ReviewHandoffRequest, ReviewProvider, ReviewRecord,
    ReviewService, ReviewStatus, ReviewVerdict, render_review_prompt,
};
pub use tools::{BrainEdge, BrainNeighborhood, BrainSymbol, Dependency};
pub use why_query::{
    WhyAnswer, WhyError, WhyFact, WhyPath, WhyPathStep, WhyRequest, WhyService, WhySource,
    WhyStatus,
};
