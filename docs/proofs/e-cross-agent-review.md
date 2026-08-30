# Milestone E proof — Claude implementation and Codex review

Date: 2026-08-30  
Branch: `feat/milestone-0`

## Scope proved

Milestone E establishes one production path that:

- durably stores provider-neutral normalized implementation events separately from bounded redacted raw process events;
- builds a hard-budgeted cross-agent handoff only from a completed implementation session and rejects the same adapter as reviewer;
- includes the original task context pack, concise implementation text/file/test evidence, and the exact untruncated Git diff with its SHA-256 digest;
- excludes raw provider frames and fails visibly when the exact diff cannot fit;
- previews that handoff in a shadcn/ui dialog before visible model permission;
- rebuilds and byte-compares current context and diff server-side immediately before invoking a read-only reviewer;
- validates reviewer output against a strict schema, exact diff-hunk IDs, source paths/lines, verdict consistency, and duplicate suppression;
- persists review findings atomically and supports open/sent/resolved/dismissed lifecycle plus an evidence-backed implementer follow-up prompt.
- compares active real worktree diffs for exact-file and rename intersections, maps changed hunks to indexed symbols, follows one-hop graph dependencies, and exposes incomplete-index uncertainty without blocking work.

## Real vertical evidence

The opt-in test `claude_implements_and_codex_reviews_the_exact_shared_handoff` used a disposable real Git repository and the same context, task-worktree, context-pack, Claude adapter, process-supervisor, normalized-event, handoff, Codex adapter, strict review, and persistence services as the desktop.

It performed this sequence:

1. created and provisioned a real task worktree;
2. captured and activated the human constraint `MARS_REVIEW_CONTEXT_814`, requiring whitespace trimming before uppercase;
3. previewed and delivered the task context pack to authenticated Claude Code `2.1.247`;
4. persisted nine provider-neutral implementation events while Claude edited the worktree;
5. reopened the exact uncommitted diff and verified the implementation contained both `trim` and `toUpperCase`;
6. built a 5,928-byte Codex handoff containing the same human constraint, concise normalized implementation evidence, and exact diff digest;
7. invoked authenticated Codex CLI `0.151.0-alpha.7.1` read-only through the strict review provider;
8. persisted and reopened Codex’s valid `pass` review.

Final live command and result:

```text
cargo test -p mars-brain --test milestone_e_real -- --ignored --nocapture

1 passed; finished in 36.47s
milestone-e proof: implementation_ms=31076, review_ms=5355, handoff_bytes=5928, context_items=1, normalized_events=9, verdict=Pass, findings=0, total_ms=36470
```

The deterministic review fixture separately proves the changes-requested branch: an exact hunk/line-backed finding persists, renders in the UI, becomes a compact follow-up for the original implementer, and transitions to `sent`.

The deterministic overlap fixture provisions two real task worktrees from one repository, changes a shared file plus graph-connected symbols in separate files, and observes exact-file, changed-symbol, and dependency warnings through the same service used by the desktop. Pure contract cases additionally cover rename and incomplete-index warnings.

## Gate results

The complete gate commands are:

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

The current repository gate passes with 82 non-ignored Rust tests and 20 desktop interaction tests. Eight network/authenticated Rust tests remain intentionally ignored by default, including the now-passing Milestone E proof.

## Shortcuts and technical debt

- Review execution currently waits synchronously for the reviewer CLI. It must use the durable process/job stream so cancellation, progress, and restart recovery match implementation sessions.
- Reviewer output is persisted as a review but the reviewer process itself is not yet an `agent_session`; implementation sessions are fully durable and normalized.
- The desktop selects the latest completed session whose adapter differs from the chosen reviewer. A review-session picker is needed when a task has several implementation attempts.
- The handoff context pack uses its deterministic default inclusion set. Per-review context exclusions should reuse the existing context-pack preview controls.
- Finding locations are visible and evidence-backed, while task-worktree source opening currently requires opening that worktree in the IDE first.
- Sending a finding back preloads the implementation adapter and prompt. Resume works while its provider checkpoint remains in the active task UI; restoring the latest durable checkpoint after a desktop restart remains recovery work.
- Normalized event persistence starts with this migration; existing historical raw sessions are not automatically backfilled.
- The live reviewer found no issue in the constrained implementation and correctly returned `pass`. The strict changes-requested/follow-up path is deterministic-test proven rather than artificially forcing a live defect.
