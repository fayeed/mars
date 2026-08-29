pub mod context_pack;
pub mod extraction;
pub mod knowledge;
pub mod query;
pub mod retrieval;
pub mod tools;
pub mod why_query;

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
pub use tools::{BrainEdge, BrainNeighborhood, BrainSymbol, Dependency};
pub use why_query::{
    WhyAnswer, WhyError, WhyFact, WhyPath, WhyPathStep, WhyRequest, WhyService, WhySource,
    WhyStatus,
};
