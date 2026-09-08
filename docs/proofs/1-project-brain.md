# Phase 1 Project Brain proof

Status: passing

Project Brain answers questions about a repository with citations. It sits on
the Phase 0 index and serves two consumers from one typed API: the agent, as a
tool, and the human, as a chat participant and a standalone activity.

## The answer contract

An answer is a structured object, not prose. Its central rule is that a
statement of fact and a piece of reasoning are different kinds of thing and are
never merged -- not in the data model, and not in any surface that renders it.

**Claims.** Each claim is `fact` or `inference`. A fact is what a cited span of
source says and must carry at least one citation; a fact with zero citations
fails validation rather than degrading into a weaker claim. An inference is a
conclusion drawn from facts and names the claim ids it reasons over. A fact may
not rest on another claim -- that would make it an inference wearing a disguise
-- and both rules are enforced by the validator regardless of what the synthesis
provider returns.

**Citations.** `{ path, lineStart, lineEnd, blobSha, snippet, why }`. `why` is
retrieval provenance in the pipeline's own words -- which rewritten query
matched, or which graph hop reached it -- rather than model prose.

**Sources and latency.** `sources` counts what was packed into the synthesis
context and `retrieved` counts everything retrieval found before packing.
`latencyMs` is measured across the whole pipeline. Both are surfaced verbatim:
the UI header reads `1.1s · 22 sources`.

**Confidence and gaps.** `confidence` is the mean claim confidence, weighting
facts above the reasoning stacked on them, less a capped penalty per declared
gap. `gaps[]` says what could not be determined and why: `notIndexed`,
`ambiguous`, `requiresRuntime`, `stale`, `budgetExhausted`, `noReasoning`.

`BrainAnswer::validate()` checks every rule, and the pipeline calls it on the
answer it is about to return. A caller receives a valid answer or an error,
never a quietly broken one.

## Pipeline

Seven stages: rewrite, retrieve, expand, rerank, pack, synthesise, validate.

**Rewrite.** Deterministic, per ADR-007. The stage earns its place because the
index's lexical search requires every term to match before falling back to
matching any, so a question asked in English is a poor query. One question
becomes up to five weighted queries -- verbatim, keywords, identifiers,
identifier subwords, and concept expansion -- plus filters the question states
outright and an intent (`locate`, `explain`, `relate`, `behaviour`, `general`).

**Retrieve and expand.** Each query runs through Phase 0 hybrid search; results
fuse by weighted reciprocal rank. The eight strongest hits then seed one hop of
graph expansion, because the answer to "how does X happen" is usually adjacent
to the chunk naming X rather than inside it. A neighbour enters at 0.55 of the
score of the hit that reached it; a chunk that is both a search hit and a graph
neighbour has its contributions added, since that is corroboration.

**Rerank.** Candidates are scored against the *question*, not against the query
that found them, from six named signals -- `retrieval`, `termOverlap`,
`identifierMatch`, `pathMatch`, `intentFit`, `freshness`. Every packed source
carries its non-zero signals into `sourceDetail`, so a ranking can be explained
rather than merely asserted. Stale evidence is penalised, not discarded; the
answer labels it instead.

**Pack.** Token-aware, with two rules. A symbol is packed whole or not at all,
because a model reading a truncated body will confidently describe behaviour
that is not there and cite a range that never contained it. And material is
dropped by lowest *marginal* value rather than lowest score: selection is greedy
on score-per-token discounted by how much of a source is already covered, with
novelty recomputed after every pick, so a cluster of near-identical chunks
cannot consume the budget. Token cost is a deterministic approximation, not a
provider's tokenizer -- Mars must not depend on one vendor's vocabulary to
decide what fits, and it errs slightly high so a packed context does not overrun
a real limit.

**Synthesise.** The provider is injected, as with memory extraction and
cross-agent review. The system prompt forbids an uncited factual claim, requires
the fact/inference split, and states that source content is untrusted data
rather than instructions. Output is strict JSON with `deny_unknown_fields`.

**Validate.** Every citation is resolved against the file as it is on disk right
now. Three outcomes: the span still holds and the citation stands; the text
moved within its file and the citation is relabelled to where it now is; the
text is gone and the citation is dropped, taking with it any fact that rested on
nothing else. Dropping cascades -- an inference left standing on no surviving
claim is dropped too -- and the loss is reported as a `stale` gap rather than
hidden.

Matching ignores blank lines and indentation, so reformatting does not
invalidate a citation while deleting the code does. The first and last lines of
a span match partially, because a chunk is a byte range rather than a run of
whole lines: the indexer's span for an exported symbol begins at `function
add(` where the file line reads `export function add(`.

## Streaming and cancellation

`ask_observing` emits `BrainEvent`s: `planned`, `retrieved`, `packed`,
`synthesized`, then each `citation`, each `claim`, each `gap`, and `completed`.

Validation must run last, since evidence can only be checked once it has been
cited, but claims are streamed *during* it rather than before it. A surface that
renders progressively must never show a claim it will shortly retract, so a
claim is emitted at the moment its evidence is confirmed and a claim whose
evidence has vanished is never emitted at all. A citation is always emitted
before any claim that names it.

Every stage checks its cancellation token before starting, so a cancelled
question stops at a stage boundary and returns `Cancelled { stage }` rather than
a half-built answer.

## Named entry points

- `brain.ask(question)` -- the full pipeline.
- `brain.explain(symbolId)` -- seeds retrieval with the named symbol's own
  chunks at above the best search hit, because the caller named the definition
  outright and no ranking signal outranks being asked directly.
- `brain.whereIsX(concept)` -- asserts `locate` intent rather than inferring it.
- `brain.dependencies(target)` -- a renderable graph slice of nodes, edges, and
  citations for a file path or a symbol id. Deliberately not an answer:
  this is structure to draw, and generating prose about it would add an uncited
  layer over data the viewer can already see. Import specifiers are resolved to
  indexed files where a relative path allows; anything else is shown as an
  unresolved package node rather than guessed.

## Consumers

**Rust API.** `ProjectBrain::{ask, ask_observing, explain, explain_observing,
where_is, where_is_observing, dependencies}`.

**Tauri commands.** `brain_ask`, `brain_explain`, `brain_where_is`,
`brain_dependency_graph`. Passing a `streamId` streams `BrainEvent`s on the
`brain-event` channel, keyed so two questions in flight cannot interleave.

**Desktop activity.** `BrainAnswer.tsx` renders facts and inferences in separate
labelled regions -- the data model keeps them apart and so does the reading
order -- with the `1.1s · 22 sources` header, per-claim citation buttons that
open the exact cited span, and a "could not determine" block for gaps.

**CLI harness.** `cargo run -p mars-brain --bin mars-brain -- --repo <path>
<ask|explain|where-is|dependencies|prompt>`, driving the same typed API. The
answer document is JSON on stdout and streamed events go to stderr, which makes
the streaming contract observable without a UI. `prompt` renders the exact
synthesis prompt without calling a provider, so what a model would be asked can
be inspected directly.

## Synthesis without a model

`DeterministicSynthesizer` is the default. It reads citation-backed structural
facts straight off the packed evidence, makes no network call, and then declares
the absence of reasoning as a `noReasoning` gap rather than passing description
off as analysis. This keeps Project Brain reproducible and testable with no
model configured, and means the validator -- not the provider -- is what the
contract rests on.

Its claims carry no line numbers. The citation is the authority on location and
the validation pass may move it; a line number repeated in the prose would then
contradict the evidence attached to the very same claim.

## Automated evidence

`crates/brain/tests/project_brain.rs` -- 23 tests over really indexed fixtures.
Nothing stubs retrieval; only the synthesis provider is substituted, and only
where the point of the test is what the pipeline does with output a model got
wrong.

- Contract: every fact carries a citation; facts and inferences stay separate;
  an uncited fact fails the answer; a fact resting on another claim is rejected;
  a claim citing an unsupplied source is rejected; sources and latency are
  reported.
- Validation: citations resolve to text currently on disk; evidence that moved
  is relabelled and says so; evidence that vanished takes its claim with it and
  raises a `stale` gap; a deleted file drops its citations.
- Budget: a tight budget drops material, reports it as a gap, and stays under
  the ceiling; an unusable budget is refused; every packed source carries
  ranking signals.
- Streaming: stages stream in order, `completed` is last, every streamed claim
  appears in the final answer and vice versa, and no claim precedes its
  citation. A cancelled question refuses to answer.
- Entry points: `explain` answers about the symbol it was given and refuses an
  unknown one; `where_is` locates a concept; `dependencies` returns a slice
  whose every edge has both endpoints among its nodes, for a file and for a
  symbol, and refuses an unknown target.
- Unanswerable questions report a gap instead of inventing facts.

Unit tests cover span location (unchanged, moved, reindented, partial-line, and
changed-body cases), token estimation, and the sorted-table invariants the
rewrite stage's binary searches depend on.

`apps/desktop/src/features/brain/BrainAnswer.test.tsx` -- 3 tests: the header
reports the measured figures verbatim; facts and inferences occupy separate
regions and neither contains the other; a citation button opens the exact span;
claims render as they stream, before the answer resolves.

### Timing

Answering against this repository -- 268 files, 4,800 chunks -- release build,
on the development machine:

| Query | Time |
| --- | --- |
| `ask`, 24 sources packed | 100-110 ms |
| `where-is`, 24 sources packed | 72 ms |
| `dependencies` of a file, 18 nodes | 2 ms |

These are retrieval, packing, and validation only. A configured synthesis
provider adds its own latency, which is why `latencyMs` is measured across the
whole pipeline rather than estimated.

## Precision limits

- Retrieval precision is bounded by Phase 0. The local embedder captures
  identifier and path overlap, not meaning, so a conceptual query such as
  "reciprocal rank fusion" ranks the correct `crates/indexer/src/search.rs`
  span highly but also admits documents that merely share vocabulary. The
  `termOverlap` and `identifierMatch` signals record how much of a source
  actually matched, which is what makes such an answer inspectable rather than
  merely confident.
- When nothing packed shares a term or identifier with the question, the answer
  says so with a `notIndexed` gap. Semantic search always returns its nearest
  neighbours however far away they are, so "retrieval returned rows" is not the
  same as "retrieval found the subject".
- The concept vocabulary is a small fixed table, not semantic understanding. It
  covers the handful of expansions where the word a person types and the word in
  the code reliably differ. Anything broader belongs in a learned model.
- Identifier splitting does not divide a run of capitals: `HTTPClient` yields one
  subword, not two.
- Graph expansion is one hop from the top eight hits. A question whose answer is
  three hops from anything lexically similar will not be reached.
- Citation drift is matched within 400 lines of the recorded position. Beyond
  that a match is likelier to be a coincidental repetition than the original
  span, so it is treated as gone.
- Dependency specifier resolution follows relative paths and the extension and
  `index` conventions the indexed languages share. Path aliases, bare
  specifiers, and package `exports` maps are left unresolved rather than
  guessed, the same limit ADR-013 places on symbol resolution.
- Answers are not persisted. Each question is answered from the index as it
  stands, which is what makes a re-asked question reflect current code.
