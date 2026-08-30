# Milestone D proof — real agent work, reviewed memory, and “Why?”

Date: 2026-08-29  
Branch: `feat/milestone-0`

## Scope proved

Milestone D establishes one production path that:

- assembles bounded, explicitly untrusted evidence from a completed normalized agent session, task intent, an exact Git diff, and diff-overlapping indexed symbols;
- sends that visible evidence to a configured Claude/Codex adapter only after explicit model permission;
- validates the response against a strict, unknown-field-denying schema, exact evidence IDs, affected entity IDs, confidence bounds, category membership, and duplicate suppression;
- stores extracted summaries, decisions, constraints, attempts, risks, questions, tests, and commits as candidates rather than trusted facts;
- requires human provenance for approve, correct/supersede, reject, stale, pin, and unpin actions, preserving history and code relationships across correction;
- answers “Why?” from approved evidence with separate facts/inferences, conflict and staleness warnings, hard depth/path limits, and source locations that can reopen diff code;
- appends a separate task → exact head commit → hosted PR path when the task has been published, without promoting hosted metadata into a knowledge fact;
- exposes extraction and review in a shadcn/ui dialog and “Ask why this exists” in the symbol inspector.

No hidden chain-of-thought is requested or stored. The durable body is concise rationale/outcome text with evidence references.

## Real vertical evidence

The opt-in test `real_agent_work_becomes_human_reviewed_explainable_memory` used a disposable real Git repository and the same project, indexer, task-worktree, Codex adapter, process-supervisor, extraction, knowledge, and why-query services as the desktop.

It performed this sequence:

1. indexed the real TypeScript fixture and created an isolated task worktree;
2. launched authenticated Codex with write permission to make `add` reject unsafe integer sums and add a focused test;
3. verified the real uncommitted diff contained `Number.isSafeInteger` and `RangeError`;
4. reopened the completed persisted session and assembled its task, event, diff-hunk, and changed-symbol evidence;
5. launched a second authenticated Codex process read-only for strict memory extraction;
6. persisted an agent-inferred summary and decision as candidates linked to the indexed `add` symbol;
7. recorded visible HUMAN review provenance, approved both, and superseded the decision with a human correction;
8. asked why `add` exists in this form and verified the answer contained the corrected HUMAN fact, the agent session, task, exact diff evidence, and bounded paths.

Final live command and result:

```text
cargo test -p mars-brain --test milestone_d_real -- --ignored --nocapture

1 passed; finished in 96.85s
milestone-d proof: implementation_ms=61982, extraction_ms=34826, candidates=2, facts=2, paths=4, total_ms=96851
```

## Failures that improved the production path

The proof found three real boundary defects before passing:

- A valid-looking extraction used plural `decisions` as a claim category. The prompt now declares every exact singular enum mapping; the strict validator was not weakened.
- A valid extraction omitted affected symbols. The prompt now requires every already-filtered `changedSymbols` entry exactly once when that list is non-empty.
- Multi-file Git diffs reassigned a pending hunk to the following file path, preventing `src/math.ts` from mapping to `add`. Hunk ownership now captures the path when the hunk begins, with a deterministic two-file regression test.

A deterministic review also found that a corrected record initially lost the original record’s semantic links. Correction now copies non-superseding relationships transactionally with HUMAN provenance before adding the `SUPERSEDES` edge.

## Deterministic and UI evidence

The brain suite covers permission denial, provider failure, strict schema rejection, unsupported evidence/entity rejection, duplicate suppression, manual fallback through the same validator, multi-file diff mapping, authority ordering, human-only resolution, preserved correction history/links, staleness, conflicts, insufficient evidence, and symbol → knowledge → session → task → evidence traces.

The desktop interaction suite proves that extraction is visibly labeled with its selected model and does not imply trust, candidates require explicit approval, resolution commands contain the reviewed task/record/action, facts and inferences render separately, warnings remain visible, and clicking exact diff evidence navigates the editor.

## Gate results

The complete workspace gate passed after the live proof and documentation changes:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -r lint
pnpm -r typecheck
pnpm -r test
pnpm -r build
pnpm check:protocol
git diff --check
```

This comprises 69 passing non-ignored Rust tests, 16 passing desktop interaction tests, successful TypeScript checks and production builds, and matching protocol `0.1.0` artifacts. Seven network/authenticated tests remain intentionally ignored in the default Rust suite, including the now-passing Milestone D live proof.

## Shortcuts and technical debt

- The desktop extraction command currently waits synchronously for the provider CLI. It must move to the persisted background-job/event model so closing a dialog or webview cannot strand UX state.
- The command persists each extracted claim and link through individually safe service operations, but the whole multi-claim batch is not yet one transaction; a mid-batch failure can leave a reviewable partial set.
- “Why?” follows direct subject ↔ knowledge links, each fact’s provenance/task/evidence chain, and exact task publication metadata. It does not yet perform a ranked general graph explanation across transitive decisions or unrelated commits/PRs.
- The response contract separates facts from inferences, but the current deterministic service emits no synthesized inference text. It favors honest source-backed facts over generating prose.
- Diff sources navigate directly to code. Session/task/knowledge source buttons currently show the exact logical source inline; dedicated timeline/task/memory deep links remain UI work.
- The real proof uses Codex for implementation and extraction. Claude shares the same normalized service boundary and fixture coverage, while the cross-provider live path is Milestone E.
- Conflict detection is intentionally conservative: multiple approved same-kind facts with different normalized bodies are warned, not automatically reconciled.
