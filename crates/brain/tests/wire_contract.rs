//! The exact JSON these types produce.
//!
//! The TypeScript declarations in `packages/shared-types` are written by hand
//! against this shape, and nothing at compile time connects the two. A field
//! that silently changes case reaches the webview as `undefined`, which is the
//! kind of failure that survives a passing test suite on both sides. These
//! tests are the connection.
//!
//! The repository's convention is camelCase fields and snake_case enum values.

use mars_brain::{
    BrainEvent, Citation, Claim, ClaimKind, ConflictChoice, ConflictSignal, Gap, GapKind,
    MemoryEventKind, MemoryKind, MemoryScope, MemoryStatus, ProposalStatus, QueryIntent,
};
use serde_json::{Value, json};

fn encode<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("value should encode")
}

#[test]
fn brain_event_fields_are_camel_case() {
    assert_eq!(
        encode(&BrainEvent::Retrieved {
            candidates: 3,
            stale_paths: 1,
        }),
        json!({ "type": "retrieved", "candidates": 3, "stalePaths": 1 })
    );
    assert_eq!(
        encode(&BrainEvent::Packed {
            sources: 2,
            tokens: 100,
            dropped: 4,
        }),
        json!({ "type": "packed", "sources": 2, "tokens": 100, "dropped": 4 })
    );
    assert_eq!(
        encode(&BrainEvent::Completed {
            sources: 1,
            claims: 2,
            latency_ms: 5,
            confidence: 0.5,
        }),
        json!({
            "type": "completed",
            "sources": 1,
            "claims": 2,
            "latencyMs": 5,
            "confidence": 0.5
        })
    );
    assert_eq!(
        encode(&BrainEvent::Planned {
            queries: vec!["a".into()],
            intent: QueryIntent::Locate,
        }),
        json!({ "type": "planned", "queries": ["a"], "intent": "locate" })
    );
}

#[test]
fn answer_records_are_camel_case() {
    assert_eq!(
        encode(&Claim {
            id: "claim:0".into(),
            kind: ClaimKind::Inference,
            text: "text".into(),
            citations: vec!["cite:0".into()],
            supports: vec!["claim:1".into()],
            confidence: 0.5,
        }),
        json!({
            "id": "claim:0",
            "kind": "inference",
            "text": "text",
            "citations": ["cite:0"],
            "supports": ["claim:1"],
            "confidence": 0.5
        })
    );
    assert_eq!(
        encode(&Citation {
            id: "cite:0".into(),
            path: "src/a.rs".into(),
            line_start: 1,
            line_end: 2,
            blob_sha: "sha".into(),
            snippet: "code".into(),
            why: "reason".into(),
        }),
        json!({
            "id": "cite:0",
            "path": "src/a.rs",
            "lineStart": 1,
            "lineEnd": 2,
            "blobSha": "sha",
            "snippet": "code",
            "why": "reason"
        })
    );
    assert_eq!(
        encode(&Gap {
            kind: GapKind::RequiresRuntime,
            subject: "s".into(),
            detail: "d".into(),
        }),
        json!({ "kind": "requiresRuntime", "subject": "s", "detail": "d" })
    );
}

#[test]
fn memory_enum_values_are_snake_case() {
    assert_eq!(encode(&MemoryScope::Global), json!("global"));
    assert_eq!(encode(&MemoryKind::Caution), json!("caution"));
    assert_eq!(encode(&MemoryStatus::Superseded), json!("superseded"));
    assert_eq!(encode(&ProposalStatus::Pending), json!("pending"));
    assert_eq!(
        encode(&MemoryEventKind::EditedAndApproved),
        json!("edited_and_approved")
    );
    assert_eq!(
        encode(&ConflictSignal::OpposingPolarity),
        json!("opposing_polarity")
    );
    assert_eq!(
        encode(&ConflictChoice::SupersedeOther),
        json!("supersede_other")
    );
}

#[test]
fn approval_outcome_is_tagged_and_camel_case() {
    let value = encode(&mars_brain::ApprovalOutcome::ConflictsRequireChoice {
        conflicts: Vec::new(),
    });
    assert_eq!(
        value,
        json!({ "outcome": "conflictsRequireChoice", "conflicts": [] })
    );
}
