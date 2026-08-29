# Milestone C proof — context, skills, retrieval, and real agent delivery

Date: 2026-08-29  
Branch: `feat/milestone-0`

## Scope proved

Milestone C establishes one production path that:

- captures task text and images while preserving immutable SHA-256-addressed originals;
- exact-deduplicates context, searches it, converts an item to an active constraint, and quarantines detected credentials before attachment;
- searches the live skills.sh registry, downloads and inspects `SKILL.md`, verifies a local content hash, pins the registry snapshot, and installs with global/project/task/workflow plus Claude/Codex policy scope;
- retrieves explicit attachments, constraints, project context/knowledge FTS matches, exact symbols, one-hop graph neighbors, Git recency, active work, and relevant trusted skills;
- excludes blocked/adapter-denied skills, secret context, preview removals, duplicates, and over-budget candidates with visible reasons;
- previews the exact pack in a shadcn/ui dialog with provenance, freshness, source, rank explanation, section usage, and remove/re-add controls;
- revalidates the exact preview in Rust immediately before sending it through the real Claude/Codex adapter path.

## Real vertical evidence

The opt-in test `captured_text_image_and_real_skill_reach_a_real_agent` used a disposable real Git repository and the same storage, context, skills, worktree, context-pack, Codex adapter, and process-supervisor services as the desktop.

It performed this sequence:

1. created and provisioned a real task worktree;
2. captured the human constraint token `ORBIT-COPPER-731` and a valid one-pixel PNG titled `Mars context pixel`;
3. discovered `web-design-guidelines` from `vercel-labs/agent-skills` through the live skills.sh API;
4. inspected and task-installed the skill for Codex only;
5. built a 1,960-byte pack containing exactly the constraint, image reference, and pinned skill, with zero exclusions;
6. delivered the rendered pack to authenticated Codex CLI `0.150.0-alpha.12.2` in the isolated worktree;
7. reopened `context-proof.txt` and verified that Codex resolved the exact text token, image title, and skill title from the supplied pack.

The real pinned local content hash was:

```text
a6a44d5498f7e8f68289902f3dedfc6f38ae0cee1e96527c80724cf27f727c2a
```

Final live command and result:

```text
cargo test -p mars-brain --test milestone_c_real -- --ignored --nocapture

1 passed; finished in 18.56s
milestone-c proof: pack_bytes=1960, items=3, exclusions=0, skill_hash=a6a44d5498f7e8f68289902f3dedfc6f38ae0cee1e96527c80724cf27f727c2a
```

The first harness run failed before network access because a reqwest blocking client was constructed inside Tokio. A backtrace identified `SkillsShRegistry::new` as the exact boundary. The proof was corrected to perform blocking registry/context preparation synchronously and enter Tokio only for process streaming, matching the working Milestone B pattern; the unchanged real scenario then passed.

## Deterministic and UI evidence

`fixtures/retrieval/cases.json` drives a comprehensive golden test for explicit attachment, exact symbol, FTS, graph neighbor, active constraint, Git recency, relevant/blocked/denied skills, secret exclusion, cross-section duplicate suppression, provenance, stable ordering, user removal/restoration, rendering, and total/section budgets. The same pack was rebuilt and byte-for-byte serialized five times.

The desktop interaction suite proves pack construction before adapter probing, exact evidence display, policy exclusions that cannot be re-added, user remove/add controls, progression to capability approval, and inclusion of the reviewed pack in launch. The backend independently reconstructs it and rejects a mismatched or stale preview.

## Gate results

Toolchain:

- Rust 1.96.1
- Node.js 24.19.0
- pnpm 10.23.0
- Git 2.43.0
- Codex CLI 0.150.0-alpha.12.2

The gate commands are:

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

Before the final proof-document changes, all checks passed with 59 non-ignored Rust tests and 14 desktop tests. Five network/authenticated tests remain intentionally ignored in the default suite, including the now-passing Milestone C live proof. The complete gate is rerun after this document is added.

## Shortcuts and technical debt

- Binary image bytes remain preserved and reopenable in the Context Tray, while the text pack delivers the image's title, MIME type, and logical entity reference. Provider-specific multimodal binary attachment is not implemented; the real proof establishes reference delivery, not visual interpretation of pixels.
- The 20/25/35/10/10 section allocation is byte-based. Token-aware accounting is still needed, and hard-required items can exceed a soft section share while remaining inside the hard total.
- Git recency currently invokes structured read-only `git log` once per selected symbol. A batched read model is needed for large candidate sets.
- The FTS baseline uses SQLite BM25 ordering with fixed normalized heuristic scores rather than calibrating raw scores across corpora. Retrieval telemetry and larger relevance judgments are needed before tuning.
- `knowledge_fts` is ready for durable decisions/constraints, but Milestone D owns authoritative knowledge records, evidence, contradiction, staleness, and superseding.
- The quick-capture shortcut is application-local in the current single-window shell. Registering an operating-system-wide shortcut and separate capture window remains desktop integration work.
- The headless environment did not visually drive the native Tauri window. React interaction tests cover the command/UI sequence; a packaged desktop smoke remains required.
