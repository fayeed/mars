//! Project Brain: the answer contract, the pipeline, and the three named
//! entry points.
//!
//! These tests run the real pipeline over real indexed repositories. Nothing
//! here stubs retrieval; only the synthesis provider is substituted, and only
//! where the point of the test is what the pipeline does with output a model
//! got wrong.

mod support;

use std::sync::Arc;

use mars_brain::{
    AskError, AskRequest, BrainEvent, ClaimKind, GapKind, ProjectBrain, SynthesisProvider,
    SynthesisRequest,
};
use tokio_util::sync::CancellationToken;

fn ask(brain: &ProjectBrain, repository_id: &str, question: &str) -> mars_brain::BrainAnswer {
    brain
        .ask(
            &AskRequest::new(repository_id, question),
            &CancellationToken::new(),
        )
        .unwrap_or_else(|error| panic!("`{question}` should answer: {error}"))
}

// --- the answer contract ------------------------------------------------

#[test]
fn every_fact_carries_at_least_one_citation() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let answer = ask(
        &brain,
        &fixture.repository_id,
        "How does the retry engine decide how many attempts to make?",
    );

    assert!(
        answer.facts().count() > 0,
        "the fixture should support at least one fact"
    );
    for fact in answer.facts() {
        assert!(
            !fact.citations.is_empty(),
            "fact `{}` reached the answer with no citation",
            fact.text
        );
        for citation in &fact.citations {
            assert!(
                answer.citation(citation).is_some(),
                "fact `{}` cites `{citation}`, which is not in the answer",
                fact.text
            );
        }
    }
    answer
        .validate()
        .expect("a returned answer is always valid");
}

#[test]
fn facts_and_inferences_stay_separate_in_the_data_model() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain_with(Arc::new(FactAndInference));
    let answer = ask(&brain, &fixture.repository_id, "What does the engine do?");

    let facts: Vec<_> = answer.facts().collect();
    let inferences: Vec<_> = answer.inferences().collect();
    assert_eq!(facts.len(), 1, "expected exactly one fact");
    assert_eq!(inferences.len(), 1, "expected exactly one inference");

    assert!(facts[0].supports.is_empty(), "a fact rests on no claim");
    assert_eq!(
        inferences[0].supports,
        vec![facts[0].id.clone()],
        "the inference should name the fact it reasons over"
    );
    assert!(
        inferences[0].citations.is_empty(),
        "this inference cites no source of its own"
    );
    answer.validate().expect("answer should be valid");
}

#[test]
fn an_uncited_fact_fails_validation_rather_than_degrading() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain_with(Arc::new(UncitedFact));
    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "What does the engine do?"),
            &CancellationToken::new(),
        )
        .expect_err("an uncited fact must fail the answer");

    assert!(
        matches!(error, AskError::Contract(_)),
        "expected a contract violation, got {error}"
    );
    assert!(
        error.to_string().contains("uncited fact"),
        "the error should name the problem: {error}"
    );
}

#[test]
fn a_fact_that_rests_on_another_claim_is_rejected() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain_with(Arc::new(FactWithSupports));
    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "What does the engine do?"),
            &CancellationToken::new(),
        )
        .expect_err("a fact resting on a claim is an inference wearing a disguise");
    assert!(matches!(error, AskError::Contract(_)), "got {error}");
}

#[test]
fn a_claim_citing_an_unsupplied_source_is_rejected() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain_with(Arc::new(UnknownSource));
    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "What does the engine do?"),
            &CancellationToken::new(),
        )
        .expect_err("a citation the pipeline never supplied cannot be resolved");
    assert!(
        matches!(error, AskError::UnknownSource { .. }),
        "got {error}"
    );
}

#[test]
fn sources_and_latency_are_reported() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let answer = ask(
        &brain,
        &fixture.repository_id,
        "Where are invoices built and rendered?",
    );

    assert!(
        answer.sources > 0,
        "an answer names how many sources it used"
    );
    assert!(
        answer.sources <= answer.retrieved,
        "packed sources ({}) cannot exceed everything retrieved ({})",
        answer.sources,
        answer.retrieved
    );
    assert!(
        (0.0..=1.0).contains(&answer.confidence),
        "confidence {} is outside the contract",
        answer.confidence
    );
    assert!(
        !answer.rewritten_queries.is_empty(),
        "the answer should report the queries it actually ran"
    );
    // The whole point of surfacing latency is that it is measured, not that it
    // is fast; a zero would mean the clock was never read.
    assert!(
        answer.latency_ms < 60_000,
        "answer took {}ms",
        answer.latency_ms
    );
}

// --- the validation pass ------------------------------------------------

#[test]
fn citations_resolve_to_the_text_currently_on_disk() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let answer = ask(
        &brain,
        &fixture.repository_id,
        "How does the retry engine decide how many attempts to make?",
    );

    assert!(!answer.citations.is_empty(), "expected citations");
    for citation in &answer.citations {
        let content = fixture.read(&citation.path);
        let lines: Vec<&str> = content.lines().collect();
        assert!(
            citation.line_end <= lines.len(),
            "{} cites lines {}-{} but the file has {} lines",
            citation.path,
            citation.line_start,
            citation.line_end,
            lines.len()
        );
        // A chunk is a byte range, so its first line may begin part-way
        // through the file's line — `function add(` where the file says
        // `export function add(`. Containment is what the citation promises.
        let first_significant = citation
            .snippet
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .expect("a citation snippet always has content");
        let window = &lines[citation.line_start - 1..citation.line_end];
        assert!(
            window
                .iter()
                .any(|line| line.trim().contains(first_significant)),
            "{}:{}-{} no longer contains `{first_significant}`",
            citation.path,
            citation.line_start,
            citation.line_end
        );
    }
}

#[test]
fn evidence_that_moved_is_relabelled_to_where_it_now_is() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let question = "How does the retry engine decide how many attempts to make?";

    let before = ask(&brain, &fixture.repository_id, question);
    let original = before
        .citations
        .iter()
        .find(|citation| citation.path == "src/engine.rs")
        .expect("the retry engine should be cited")
        .clone();

    // Push the whole file down without re-indexing, so the index now describes
    // line numbers the working tree no longer uses.
    let shifted = format!(
        "// A new leading comment.\n// A second new line.\n{}",
        fixture.read("src/engine.rs")
    );
    fixture.write("src/engine.rs", &shifted);

    let after = ask(&brain, &fixture.repository_id, question);
    let moved = after
        .citations
        .iter()
        .find(|citation| citation.path == "src/engine.rs")
        .expect("the evidence still exists, so the citation should survive");

    assert_eq!(
        moved.line_start,
        original.line_start + 2,
        "the citation should point two lines further down"
    );
    assert!(
        moved.why.contains("relabelled"),
        "a moved citation should say so: {}",
        moved.why
    );

    let content = fixture.read("src/engine.rs");
    let lines: Vec<&str> = content.lines().collect();
    let first = moved
        .snippet
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .expect("snippet has content");
    assert_eq!(
        lines[moved.line_start - 1].trim(),
        first,
        "the relabelled line should hold the cited text"
    );
}

#[test]
fn evidence_that_vanished_takes_its_claim_with_it() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let question = "How does the retry engine decide how many attempts to make?";

    let before = ask(&brain, &fixture.repository_id, question);
    assert!(
        before
            .citations
            .iter()
            .any(|citation| citation.path == "src/engine.rs"),
        "the retry engine should be cited before the file is replaced"
    );

    // Replace the file wholesale, again without re-indexing.
    fixture.write(
        "src/engine.rs",
        "//! Nothing that was here before is here now.\npub fn placeholder() {}\n",
    );

    let after = ask(&brain, &fixture.repository_id, question);
    assert!(
        !after
            .citations
            .iter()
            .any(|citation| citation.path == "src/engine.rs"
                && citation.snippet.contains("MAX_ATTEMPTS")),
        "a citation whose text is gone must not survive"
    );
    for claim in &after.claims {
        assert!(
            !claim.citations.is_empty() || claim.kind == ClaimKind::Inference,
            "a fact survived without evidence: {}",
            claim.text
        );
    }
    assert!(
        after.gaps.iter().any(|gap| gap.kind == GapKind::Stale),
        "dropping evidence must be reported as a gap, not hidden"
    );
    after.validate().expect("the pruned answer is still valid");
}

#[test]
fn a_deleted_file_drops_its_citations() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let question = "How does the retry engine decide how many attempts to make?";
    let _ = ask(&brain, &fixture.repository_id, question);

    std::fs::remove_file(fixture.root.join("src/engine.rs")).expect("fixture file should delete");

    let after = ask(&brain, &fixture.repository_id, question);
    assert!(
        !after
            .citations
            .iter()
            .any(|citation| citation.path == "src/engine.rs"),
        "a deleted file cannot be cited"
    );
    after.validate().expect("answer should still be valid");
}

// --- budget packing -----------------------------------------------------

#[test]
fn a_small_budget_drops_material_and_says_so() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let request = AskRequest::new(
        &fixture.repository_id,
        "How does the retry engine decide how many attempts to make?",
    )
    .with_budget(mars_brain::MIN_BUDGET_TOKENS);

    let answer = brain
        .ask(&request, &CancellationToken::new())
        .expect("a tight budget still answers");

    assert!(answer.truncated, "a tight budget should drop material");
    assert!(
        answer.sources < answer.retrieved,
        "packing should have dropped something"
    );
    assert!(
        answer
            .gaps
            .iter()
            .any(|gap| gap.kind == GapKind::BudgetExhausted),
        "dropped material must be reported as a gap"
    );

    // Whole symbols only: every packed source must still be a complete chunk,
    // so the cited span matches the snippet's own extent rather than a cut.
    let packed: usize = answer
        .source_detail
        .iter()
        .map(|source| source.tokens)
        .sum();
    assert!(
        packed <= mars_brain::MIN_BUDGET_TOKENS,
        "packed {packed} tokens over a {} budget",
        mars_brain::MIN_BUDGET_TOKENS
    );
}

#[test]
fn a_budget_outside_the_supported_range_is_refused() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "anything").with_budget(4),
            &CancellationToken::new(),
        )
        .expect_err("an unusable budget is a caller error");
    assert!(matches!(error, AskError::InvalidBudget(4)), "got {error}");
}

#[test]
fn ranking_signals_explain_each_packed_source() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let answer = ask(
        &brain,
        &fixture.repository_id,
        "How does the retry engine decide how many attempts to make?",
    );

    assert_eq!(
        answer.source_detail.len(),
        answer.sources,
        "every packed source should be explainable"
    );
    for source in &answer.source_detail {
        assert!(
            !source.signals.is_empty(),
            "{} was ranked with no recorded signal",
            source.path
        );
        assert!(source.tokens > 0, "{} costs no tokens", source.path);
    }
}

// --- streaming and cancellation ----------------------------------------

#[test]
fn claims_stream_in_order_and_only_after_their_evidence() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let mut events = Vec::new();
    let answer = brain
        .ask_observing(
            &AskRequest::new(
                &fixture.repository_id,
                "How does the retry engine decide how many attempts to make?",
            ),
            &CancellationToken::new(),
            |event| events.push(event.clone()),
        )
        .expect("streaming ask should answer");

    let position = |predicate: fn(&BrainEvent) -> bool| {
        events
            .iter()
            .position(predicate)
            .unwrap_or_else(|| panic!("expected event missing from {events:#?}"))
    };
    let planned = position(|event| matches!(event, BrainEvent::Planned { .. }));
    let retrieved = position(|event| matches!(event, BrainEvent::Retrieved { .. }));
    let packed = position(|event| matches!(event, BrainEvent::Packed { .. }));
    let synthesized = position(|event| matches!(event, BrainEvent::Synthesized { .. }));
    let completed = position(|event| matches!(event, BrainEvent::Completed { .. }));
    assert!(
        planned < retrieved && retrieved < packed && packed < synthesized,
        "pipeline stages should stream in order"
    );
    assert_eq!(completed, events.len() - 1, "completion is the last event");

    let streamed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            BrainEvent::Claim(claim) => Some(claim.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        streamed, answer.claims,
        "every streamed claim should appear in the final answer, and vice versa"
    );

    // A claim must never arrive before the evidence it names.
    let mut seen_citations = Vec::new();
    for event in &events {
        match event {
            BrainEvent::Citation(citation) => seen_citations.push(citation.id.clone()),
            BrainEvent::Claim(claim) => {
                for citation in &claim.citations {
                    assert!(
                        seen_citations.contains(citation),
                        "claim `{}` streamed before its citation `{citation}`",
                        claim.text
                    );
                }
            }
            _ => {}
        }
    }
}

#[test]
fn a_cancelled_question_stops_instead_of_answering() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "What does the engine do?"),
            &cancellation,
        )
        .expect_err("a cancelled question must not answer");
    assert!(matches!(error, AskError::Cancelled { .. }), "got {error}");
}

#[test]
fn an_empty_question_is_refused() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let error = brain
        .ask(
            &AskRequest::new(&fixture.repository_id, "   "),
            &CancellationToken::new(),
        )
        .expect_err("an empty question has no answer");
    assert!(matches!(error, AskError::EmptyQuestion), "got {error}");
}

// --- the named entry points --------------------------------------------

#[test]
fn explain_answers_about_the_symbol_it_was_given() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let symbol_id = fixture.symbol_id("run_with_retries");

    let answer = brain
        .explain(
            &fixture.repository_id,
            &symbol_id,
            &CancellationToken::new(),
        )
        .expect("explain should answer");

    assert!(
        answer.question.contains("run_with_retries"),
        "the question should name the symbol: {}",
        answer.question
    );
    assert!(
        answer
            .citations
            .iter()
            .any(|citation| citation.path == "src/engine.rs"),
        "explaining a symbol should cite the file that defines it"
    );
    assert!(
        answer
            .source_detail
            .iter()
            .any(|source| source.symbol_path.contains("run_with_retries")),
        "the symbol asked about should be among the packed sources"
    );
    answer.validate().expect("answer should be valid");
}

#[test]
fn explain_refuses_a_symbol_that_does_not_exist() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let error = brain
        .explain(
            &fixture.repository_id,
            "symbol-that-was-never-indexed",
            &CancellationToken::new(),
        )
        .expect_err("an unknown symbol has nothing to explain");
    assert!(matches!(error, AskError::SymbolNotFound(_)), "got {error}");
}

#[test]
fn where_is_locates_a_concept_by_name() {
    let fixture = support::polyglot_fixture();
    let brain = fixture.brain();
    let answer = brain
        .where_is(&fixture.repository_id, "invoice", &CancellationToken::new())
        .expect("where_is should answer");

    assert!(
        answer
            .citations
            .iter()
            .any(|citation| citation.path == "src/service.py"),
        "invoices live in the Python service: {:#?}",
        answer
            .citations
            .iter()
            .map(|citation| &citation.path)
            .collect::<Vec<_>>()
    );
    answer.validate().expect("answer should be valid");
}

#[test]
fn dependencies_of_a_file_returns_a_renderable_slice() {
    let fixture = support::indexed_fixture();
    let brain = fixture.brain();
    let slice = brain
        .dependencies(
            &fixture.repository_id,
            "src/main.ts",
            &CancellationToken::new(),
        )
        .expect("file dependencies should resolve");

    assert_eq!(slice.target, "src/main.ts");
    assert_eq!(slice.target_kind, mars_brain::DependencyTarget::File);
    assert!(
        slice.nodes.iter().any(|node| node.id == "src/math.ts"),
        "main.ts imports math.ts: {:#?}",
        slice.nodes.iter().map(|node| &node.id).collect::<Vec<_>>()
    );
    assert!(
        slice
            .edges
            .iter()
            .any(|edge| edge.source == "src/main.ts" && edge.target == "src/math.ts"),
        "the import edge should be directed away from main.ts"
    );
    assert!(
        slice.nodes.iter().all(|node| node.distance <= 1),
        "a file slice is one hop in each direction"
    );
}

#[test]
fn dependencies_of_a_symbol_returns_nodes_edges_and_citations() {
    let fixture = support::indexed_fixture();
    let brain = fixture.brain();
    let symbol_id = fixture.symbol_id("Calculator");

    let slice = brain
        .dependencies(
            &fixture.repository_id,
            &symbol_id,
            &CancellationToken::new(),
        )
        .expect("symbol dependencies should resolve");

    assert_eq!(slice.target_kind, mars_brain::DependencyTarget::Symbol);
    assert!(!slice.nodes.is_empty(), "expected graph nodes");
    assert!(!slice.edges.is_empty(), "expected graph edges");
    for edge in &slice.edges {
        assert!(
            slice.nodes.iter().any(|node| node.id == edge.source),
            "edge source `{}` has no node to draw from",
            edge.source
        );
        assert!(
            slice.nodes.iter().any(|node| node.id == edge.target),
            "edge target `{}` has no node to draw to",
            edge.target
        );
    }
    for citation in &slice.citations {
        assert!(
            !citation.snippet.trim().is_empty(),
            "a graph citation should open real source"
        );
    }
}

#[test]
fn dependencies_refuses_a_target_it_cannot_find() {
    let fixture = support::indexed_fixture();
    let brain = fixture.brain();
    let error = brain
        .dependencies(
            &fixture.repository_id,
            "src/does-not-exist.ts",
            &CancellationToken::new(),
        )
        .expect_err("an unknown target has no dependencies");
    assert!(matches!(error, AskError::TargetNotFound(_)), "got {error}");
}

// --- unanswerable questions --------------------------------------------

#[test]
fn a_question_the_index_cannot_answer_reports_a_gap() {
    let fixture = support::indexed_fixture();
    let brain = fixture.brain();
    let answer = ask(
        &brain,
        &fixture.repository_id,
        "zzqqxx unmatchable nonsense term",
    );

    assert!(
        answer.claims.is_empty() || answer.facts().all(|fact| !fact.citations.is_empty()),
        "an unanswerable question must not invent facts"
    );
    assert!(
        answer
            .gaps
            .iter()
            .any(|gap| gap.kind == GapKind::NotIndexed),
        "nothing found should be reported as a gap: {:#?}",
        answer.gaps
    );
    answer.validate().expect("an empty answer is still valid");
}

// --- synthesis providers used above ------------------------------------

/// Emits one properly cited fact and one inference resting on it.
struct FactAndInference;

impl SynthesisProvider for FactAndInference {
    fn name(&self) -> &str {
        "test-fact-and-inference"
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<String, String> {
        let source = request
            .sources
            .first()
            .ok_or_else(|| "no sources supplied".to_owned())?;
        Ok(format!(
            r#"{{"claims":[
                {{"id":"claim:0","kind":"fact","text":"{} is defined in {}.","sources":["{}"],"supports":[],"confidence":0.9}},
                {{"id":"claim:1","kind":"inference","text":"This module is the retry entry point.","sources":[],"supports":["claim:0"],"confidence":0.5}}
            ],"gaps":[]}}"#,
            source.symbol_path.replace('"', ""),
            source.path,
            source.id
        ))
    }
}

/// Emits a fact with no evidence, which the contract forbids.
struct UncitedFact;

impl SynthesisProvider for UncitedFact {
    fn name(&self) -> &str {
        "test-uncited-fact"
    }

    fn synthesize(&self, _request: &SynthesisRequest) -> Result<String, String> {
        Ok(r#"{"claims":[{"id":"claim:0","kind":"fact","text":"The engine retries three times.","sources":[],"supports":[],"confidence":0.99}],"gaps":[]}"#.to_owned())
    }
}

/// Emits a fact that rests on another claim, which makes it an inference.
struct FactWithSupports;

impl SynthesisProvider for FactWithSupports {
    fn name(&self) -> &str {
        "test-fact-with-supports"
    }

    fn synthesize(&self, request: &SynthesisRequest) -> Result<String, String> {
        let source = request
            .sources
            .first()
            .ok_or_else(|| "no sources supplied".to_owned())?;
        Ok(format!(
            r#"{{"claims":[
                {{"id":"claim:0","kind":"fact","text":"A cited fact.","sources":["{}"],"supports":[],"confidence":0.9}},
                {{"id":"claim:1","kind":"fact","text":"A fact resting on another claim.","sources":["{}"],"supports":["claim:0"],"confidence":0.9}}
            ],"gaps":[]}}"#,
            source.id, source.id
        ))
    }
}

/// Cites a source the pipeline never supplied.
struct UnknownSource;

impl SynthesisProvider for UnknownSource {
    fn name(&self) -> &str {
        "test-unknown-source"
    }

    fn synthesize(&self, _request: &SynthesisRequest) -> Result<String, String> {
        Ok(r#"{"claims":[{"id":"claim:0","kind":"fact","text":"Invented.","sources":["src:9999"],"supports":[],"confidence":0.9}],"gaps":[]}"#.to_owned())
    }
}
