# Agent-Driven IDE MVP Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Deliver six executable vertical proofs from repository opening and structural search through cross-agent review and GitHub pull-request feedback.

**Architecture:** A Tauri 2/React desktop app calls a reusable Rust core through a versioned command/event protocol. Domain crates own SQLite/FTS, tree-sitter indexing, system-Git worktrees, CLI agent processes, context/skills, retrieval, memory, and a provider-neutral forge layer; each milestone extends the same production path.

**Tech Stack:** Rust stable, Tauri 2, React, TypeScript, pnpm, Vite, shadcn/ui, Monaco, xterm.js, SQLite/FTS5, tree-sitter, Tokio, serde, reqwest, Git CLI, Vitest, Playwright.

---

## Delivery rules

- Follow [ARCHITECTURE.md](../../ARCHITECTURE.md) and record any divergence in [DECISIONS.md](../../DECISIONS.md) before implementing it.
- Work milestone-by-milestone. No later proof may substitute a fake provider, fake Git state, or UI-only fixture for its real boundary.
- Use TDD for parsers, storage, retrieval, Git planning/state transitions, and permission policy. Record real CLI/API smoke tests separately because credentials/tools may be unavailable in CI.
- At each milestone: demonstrate the named workflow, run all repository checks, update architecture/decisions, add a short `docs/proofs/<milestone>.md`, list shortcuts/debt, and make one coherent milestone commit after smaller component commits.
- Never run an agent in the primary working tree. Never silently stage, destroy, merge, push, or create a remote object.

## Milestone 0 — Walking skeleton and contracts

**Scope:** Establish buildable workspaces, the Rust core boundary, versioned protocol, SQLite migration runner, job/event plumbing, and an empty but real desktop shell. This is infrastructure only; it claims none of proofs A–F.

### Task 0.1: Bootstrap workspaces

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `package.json`
- Create: `pnpm-workspace.yaml`
- Create: `tsconfig.base.json`
- Create: `apps/desktop/package.json`
- Create: `apps/desktop/src/main.tsx`
- Create: `apps/desktop/src/App.tsx`
- Create: `apps/desktop/src-tauri/Cargo.toml`
- Create: `apps/desktop/src-tauri/src/main.rs`
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `packages/protocol/package.json`
- Create: `packages/shared-types/package.json`
- Create: `packages/ui/package.json`

**Steps:**

1. Add the Cargo and pnpm workspace manifests with only the packages needed by Milestone 0.
2. Write a failing Rust test that constructs `Core` and returns a semantic protocol version.
3. Run `cargo test -p mars-core`; expect failure because `Core` is absent.
4. Implement the minimal `Core` and `ProtocolVersion`.
5. Add a React shell that invokes a `core_info` Tauri command and renders connected/disconnected state.
6. Run `cargo test --workspace` and `pnpm -r typecheck`; expect both to pass.
7. Commit: `chore: bootstrap Mars workspaces`.

### Task 0.2: Add protocol and compatibility checks

**Files:**
- Create: `packages/protocol/schema/core.schema.json`
- Create: `packages/protocol/src/generated.ts`
- Create: `crates/core/src/protocol.rs`
- Create: `crates/core/tests/protocol_compat.rs`
- Create: `scripts/check-protocol.mjs`

**Steps:**

1. Define envelopes `CommandRequest`, `CommandResponse`, and `CoreEvent` with protocol version, correlation ID, typed payload, and structured error.
2. Write a failing fixture round-trip test in Rust and TypeScript.
3. Implement serialization and schema validation; preserve unknown event fields for forward compatibility.
4. Run `cargo test -p mars-core protocol_compat` and `pnpm --filter @mars/protocol test`; expect pass.
5. Commit: `feat: define versioned core protocol`.

### Task 0.3: Add storage, migrations, and durable jobs

**Files:**
- Create: `crates/storage/Cargo.toml`
- Create: `crates/storage/src/lib.rs`
- Create: `crates/storage/src/migrations.rs`
- Create: `crates/storage/migrations/0001_core.sql`
- Create: `crates/core/src/jobs.rs`
- Create: `crates/core/tests/job_recovery.rs`

**Steps:**

1. Write failing tests for a fresh migration, repeated migration, WAL/foreign-key settings, ordered job events, cancellation, and recovery of an interrupted job.
2. Run `cargo test -p mars-core -p mars-storage`; expect missing implementation failures.
3. Implement the bundled-FTS5 SQLite connection, migration journal, connection setup, `jobs`/`job_events` tables, and a Tokio cancellation registry.
4. Run the same tests; expect pass, including a reopen of an on-disk temporary database.
5. Document the actual database location and backup behavior in `docs/runbooks/local-data.md`.
6. Commit: `feat: add durable storage and jobs`.

### Milestone 0 gate

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm -r lint`, `pnpm -r typecheck`, and `pnpm -r test`. Record versions and results in `docs/proofs/0-walking-skeleton.md`; list debt explicitly. Update architecture/decisions and commit `docs: record walking skeleton proof`.

## Milestone A — Open repo → edit/browse → index code → query Brain

**Scope:** Open a trusted real repository, browse/edit through Monaco, use a native PTY terminal, incrementally index TS/JS, and answer structural queries from SQLite with source evidence.

### Task A.1: Repository trust and opening

**Files:**
- Create: `crates/security/src/lib.rs`
- Create: `crates/core/src/projects.rs`
- Create: `crates/storage/migrations/0002_projects.sql`
- Create: `crates/core/tests/open_repository.rs`
- Create: `apps/desktop/src/features/projects/OpenRepository.tsx`

**Steps:**

1. Write failing tests for canonical path handling, non-Git rejection, `.git` file/worktree discovery, repeated open idempotency, and trust-required state.
2. Implement `ProjectService::open_repository` and repository persistence without invoking shell text.
3. Build an open dialog and trust confirmation that displays the resolved root and enabled capabilities.
4. Run targeted Rust/UI tests; expect pass.
5. Commit: `feat: open and trust Git repositories`.

### Task A.2: File explorer, Monaco editing, and terminal

**Files:**
- Create: `crates/core/src/files.rs`
- Create: `crates/core/src/terminal.rs`
- Create: `apps/desktop/src/features/explorer/FileExplorer.tsx`
- Create: `apps/desktop/src/features/editor/CodeEditor.tsx`
- Create: `apps/desktop/src/features/terminal/TerminalPanel.tsx`
- Create: `apps/desktop/src/features/editor/editor.test.tsx`

**Steps:**

1. Write failing tests for ignored/binary/oversized files, optimistic-save conflict detection, terminal cwd, resize, bounded scrollback, and process cancellation.
2. Implement paginated file listing, versioned reads, atomic writes, and a native PTY session rooted in the selected repository.
3. Wire Explorer → Monaco tabs, dirty indicators, save conflict UI, and xterm.js.
4. Verify manually that an edit made in Monaco is visible in `git diff` in the terminal.
5. Commit: `feat: add repository editor and terminal`.

### Task A.3: TS/JS structural index

**Files:**
- Create: `crates/indexer/Cargo.toml`
- Create: `crates/indexer/src/walk.rs`
- Create: `crates/indexer/src/javascript.rs`
- Create: `crates/indexer/src/publish.rs`
- Create: `crates/storage/migrations/0003_code_graph.sql`
- Create: `fixtures/repositories/typescript-graph/`
- Create: `crates/indexer/tests/typescript_graph.rs`
- Create: `crates/indexer/tests/interrupted_run.rs`

**Steps:**

1. Create a tiny fixture covering imports/exports, function/class/interface/method definitions, calls, references, inheritance, tests, ignored paths, syntax errors, and a rename.
2. Write failing assertions for node spans/stable keys, resolved and unresolved edges, confidence/evidence, incremental unchanged-file skipping, deletion, and interrupted-run isolation.
3. Implement ignore-aware walking, content hashing, tree-sitter queries, conservative resolution, and transactional per-file publication.
4. Run `cargo test -p mars-indexer`; expect exact graph fixture pass.
5. Commit: `feat: index TypeScript and JavaScript structure`.

### Task A.4: Project Brain query tools

**Files:**
- Create: `crates/brain/Cargo.toml`
- Create: `crates/brain/src/query.rs`
- Create: `crates/brain/src/tools.rs`
- Create: `crates/brain/tests/structural_queries.rs`
- Create: `apps/desktop/src/features/brain/BrainSearch.tsx`
- Create: `apps/desktop/src/features/brain/EntityInspector.tsx`

**Steps:**

1. Write failing tests for `find_symbol`, `references`, `callers`, `callees`, `dependencies`, `related_code`, and bounded `blast_radius` with evidence paths.
2. Implement typed queries using indexed adjacency lookups; reject unbounded traversal.
3. Add Brain search/results and source navigation to Monaco.
4. Run Rust/UI tests and verify query results against the fixture.
5. Commit: `feat: query structural Project Brain`.

### Milestone A gate

Open a separate real TS repository, edit a file, run a command in the terminal, index it, and navigate from a Brain result to code. Run all checks, capture timings/known precision gaps in `docs/proofs/a-brain.md`, update architecture/decisions/debt, then commit `docs: record milestone A proof`.

## Milestone B — Create task → worktree → real agent → stream output

**Scope:** A task provisions a safe Git branch/worktree and launches an installed Claude Code or Codex CLI through a real process adapter with normalized streaming, permissions, cancellation, and recovery.

### Task B.1: Task and worktree state machines

**Files:**
- Create: `crates/git/Cargo.toml`
- Create: `crates/git/src/command.rs`
- Create: `crates/git/src/worktree.rs`
- Create: `crates/git/src/status.rs`
- Create: `crates/storage/migrations/0004_tasks_worktrees.sql`
- Create: `crates/git/tests/worktree_lifecycle.rs`
- Create: `crates/git/tests/fixtures.rs`

**Steps:**

1. Write failing tests using temporary real Git repositories for preflight, deterministic collision handling, branch/worktree creation, rollback, dirty removal refusal, stale reconciliation, and paths with spaces.
2. Implement system-Git argument execution, porcelain parsing, mutation journal, and explicit lifecycle transitions.
3. Run `cargo test -p mars-git`; expect pass without global Git config assumptions.
4. Commit: `feat: manage task worktrees safely`.

### Task B.2: Permission engine and process supervisor

**Files:**
- Create: `crates/security/src/permissions.rs`
- Create: `crates/agents/Cargo.toml`
- Create: `crates/agents/src/process.rs`
- Create: `crates/agents/tests/process_lifecycle.rs`
- Create: `crates/storage/migrations/0005_agent_sessions.sql`

**Steps:**

1. Write failing table tests for deny/ask/allow-once/session scope and for child requests never widening grants.
2. Write failing process tests for stdout/stderr chunking, ordered sequence numbers, cancellation escalation, exit capture, backpressure, and restart reconciliation.
3. Implement capability evaluation and a Tokio process supervisor with bounded channels and redaction hooks.
4. Run `cargo test -p mars-security -p mars-agents`; expect pass.
5. Commit: `feat: supervise permissioned agent processes`.

### Task B.3: Claude and Codex adapters

**Files:**
- Create: `crates/agents/src/adapter.rs`
- Create: `crates/agents/src/claude.rs`
- Create: `crates/agents/src/codex.rs`
- Create: `fixtures/agents/claude/`
- Create: `fixtures/agents/codex/`
- Create: `crates/agents/tests/adapter_contract.rs`

**Steps:**

1. Capture redacted representative streams for supported installed CLI versions and check them into fixtures with version metadata.
2. Write one adapter contract suite covering probe, command construction, text/tool/command/file/test/permission/checkpoint/unknown/end events, malformed frames, partial frames, resume capability, and nonzero exit.
3. Implement both adapters without provider branches in the supervisor/UI.
4. Add opt-in ignored smoke tests that invoke each real installed CLI in a disposable worktree.
5. Run fixture contract tests; run available real smoke tests and record exact skipped prerequisites.
6. Commit: `feat: adapt Claude and Codex CLI sessions`.

### Task B.4: Task and agent user flow

**Files:**
- Create: `apps/desktop/src/features/tasks/TaskList.tsx`
- Create: `apps/desktop/src/features/tasks/TaskDetail.tsx`
- Create: `apps/desktop/src/features/tasks/CreateTaskDialog.tsx`
- Create: `apps/desktop/src/features/agents/AgentTimeline.tsx`
- Create: `apps/desktop/src/features/agents/PermissionPrompt.tsx`
- Create: `apps/desktop/src/features/tasks/task-agent.test.tsx`

**Steps:**

1. Write failing UI tests for task creation, worktree-plan approval, adapter probe failure, launch, incremental events, steering, cancellation, crash state, and opening worktree in editor/terminal.
2. Implement the Tasks-first flow and bounded event virtualization.
3. Run UI tests and a desktop smoke workflow with one real adapter.
4. Commit: `feat: run agents from isolated tasks`.

### Milestone B gate

Create a task in a real repository, approve its branch/worktree, launch one real agent, observe streamed commands/changes, cancel or complete it, and reopen the changed files. Run all checks; record CLI versions, permissions, recovery outcome, and debt in `docs/proofs/b-agent-worktree.md`; update architecture/decisions and commit `docs: record milestone B proof`.

## Milestone C — Capture context + install skill → retrieve → agent work

**Scope:** Preserve text/image context and install an inspected pinned skill, retrieve a compact explainable task pack, preview it, and deliver it through the real adapter path.

### Task C.1: Context artifact store and capture

**Files:**
- Create: `crates/context/Cargo.toml`
- Create: `crates/context/src/artifacts.rs`
- Create: `crates/context/src/capture.rs`
- Create: `crates/storage/migrations/0006_context.sql`
- Create: `crates/context/tests/capture.rs`
- Create: `apps/desktop/src/features/context/ContextTray.tsx`
- Create: `apps/desktop/src/features/context/QuickCapture.tsx`

**Steps:**

1. Write failing tests for text/image capture, SHA-256 exact dedup, verified atomic copy, original preservation, failed-transaction cleanup, task attachment, search, and secret quarantine.
2. Implement content-addressed artifacts and context repositories.
3. Add paste/drop capture, global quick-capture window, task tray, preview, and convert-to-constraint action.
4. Run tests and manually reopen an original captured image.
5. Commit: `feat: capture task context`.

### Task C.2: Skill inspection, trust, and installation

**Files:**
- Create: `crates/skills/Cargo.toml`
- Create: `crates/skills/src/registry.rs`
- Create: `crates/skills/src/install.rs`
- Create: `crates/skills/src/policy.rs`
- Create: `crates/storage/migrations/0007_skills.sql`
- Create: `crates/skills/tests/install.rs`
- Create: `apps/desktop/src/features/skills/SkillBrowser.tsx`
- Create: `apps/desktop/src/features/skills/SkillInspector.tsx`

**Steps:**

1. Write failing tests for discovery metadata, quarantine, mandatory `SKILL.md` inspection, pin/hash verification, update diff, global/project/task scope, adapter deny, and failed install rollback.
2. Implement a registry client boundary and filesystem installer using a disposable staging directory.
3. Add search, source/trust display, `SKILL.md` viewer, install/update/remove, scope, and adapter policy UI.
4. Run unit tests; perform one real skills.sh discovery/install smoke test if network is available and record source/ref/hash.
5. Commit: `feat: install and scope trusted skills`.

### Task C.3: Retrieval and context packs

**Files:**
- Create: `crates/brain/src/retrieval.rs`
- Create: `crates/brain/src/context_pack.rs`
- Create: `crates/storage/migrations/0008_knowledge_fts.sql`
- Create: `fixtures/retrieval/cases.json`
- Create: `crates/brain/tests/retrieval_golden.rs`
- Create: `apps/desktop/src/features/agents/ContextPackPreview.tsx`

**Steps:**

1. Encode golden cases for explicit attachment, exact symbol, FTS, graph neighbors, active constraint, Git recency, relevant skill, blocked skill exclusion, secret exclusion, duplicate suppression, provenance, deterministic order, and total/section budgets.
2. Run tests; expect failures with the empty retriever.
3. Implement candidate generation, score normalization, authority/freshness rules, diversification, budgeting, and rank explanations from Architecture §13.
4. Add a pack preview with inclusion reason and remove/add controls before launch.
5. Run golden tests repeatedly to confirm stable ordering.
6. Commit: `feat: build compact explainable context packs`.

### Milestone C gate

Attach text and an image to a real task, inspect/install a real skill, build and preview the pack, then launch an agent that can reference the supplied items. Run all checks; record pack size, included evidence, install hash, exclusions, and debt in `docs/proofs/c-context-skills.md`; update architecture/decisions and commit `docs: record milestone C proof`.

## Milestone D — Agent work → durable memory → ask why

**Scope:** Extract evidence-backed memory candidates from a completed session plus diff, let a human resolve them, and answer a `why` query with traceable sources.

### Task D.1: Knowledge records and entity links

**Files:**
- Create: `crates/storage/migrations/0009_knowledge.sql`
- Create: `crates/brain/src/knowledge.rs`
- Create: `crates/brain/tests/knowledge.rs`

**Steps:**

1. Write failing tests for provenance, confidence bounds, human approval, correction, rejection, staleness, superseding without deletion, generic link validation, and authority ordering.
2. Implement repositories and transactional memory resolution.
3. Run targeted tests; expect pass.
4. Commit: `feat: store provenance-backed project memory`.

### Task D.2: Post-task extraction

**Files:**
- Create: `crates/brain/src/extraction.rs`
- Create: `fixtures/memory/session-events.jsonl`
- Create: `fixtures/memory/task.diff`
- Create: `crates/brain/tests/extraction.rs`

**Steps:**

1. Define structured output for summary, decisions/rationale, constraints, affected entities, attempts, risks, questions, tests, and commits; exclude hidden reasoning and unsupported claims.
2. Write failing tests for schema validation, evidence requirements, duplicate candidates, malicious/untrusted text, changed-symbol linking, and provider failure.
3. Implement a replaceable extractor interface, deterministic evidence assembly, strict output validation, and a manual fallback candidate editor. Use the configured agent/model only with visible permission.
4. Run fixture tests and one opt-in real extraction smoke test.
5. Commit: `feat: extract reviewable post-task memory`.

### Task D.3: Why query and memory review UI

**Files:**
- Create: `crates/brain/src/why_query.rs`
- Create: `crates/brain/tests/why_query.rs`
- Create: `apps/desktop/src/features/brain/MemoryReview.tsx`
- Create: `apps/desktop/src/features/brain/WhyAnswer.tsx`

**Steps:**

1. Write failing tests showing `code → change/commit → task → decision → evidence`, contradiction handling, stale facts, and “insufficient evidence.”
2. Implement bounded path search and answer assembly that separates facts from inference.
3. Add approve/edit/reject/pin/supersede controls and clickable provenance paths.
4. Run tests and verify every answer source opens.
5. Commit: `feat: answer why with durable provenance`.

### Milestone D gate

Complete real agent work, extract candidates, approve/correct at least one decision, then ask why a changed symbol exists and trace the answer to task/session/diff evidence. Run all checks; record unsupported/uncertain behavior and debt in `docs/proofs/d-memory.md`; update architecture/decisions and commit `docs: record milestone D proof`.

## Milestone E — Claude implements → Codex reviews shared context

**Scope:** Build an automatic compact handoff from an implementation session to the other provider, normalize review findings, warn about overlapping active work, and send a finding back to the implementer.

### Task E.1: Handoff pack

**Files:**
- Create: `crates/agents/src/handoff.rs`
- Create: `crates/agents/tests/handoff.rs`
- Create: `fixtures/handoff/expected-pack.json`

**Steps:**

1. Write failing tests for original intent, explicit context, approved facts, selected skills, implementation summary, test evidence, commits, exact diff, path normalization, secret exclusion, dedup, and hard budgets.
2. Implement the provider-neutral handoff builder on top of the same context-pack types used in C.
3. Assert the full implementation transcript is absent by default and all inclusions have provenance.
4. Run tests; expect pass.
5. Commit: `feat: build cross-agent handoff packs`.

### Task E.2: Review findings contract

**Files:**
- Create: `crates/agents/src/review.rs`
- Create: `crates/agents/tests/review.rs`
- Create: `apps/desktop/src/features/reviews/ReviewPanel.tsx`

**Steps:**

1. Write failing tests for severity/title/body/path/line parsing, invalid locations, finding fingerprints/dedup, verdict, and unstructured fallback.
2. Implement a review role prompt/contract and strict parser into `Review`/`ReviewFinding` records.
3. Add “Review with other agent,” handoff preview, findings on the diff, and “send to implementer” as a visible steer event.
4. Run adapter/UI tests.
5. Commit: `feat: review agent work with another agent`.

### Task E.3: Overlap warnings

**Files:**
- Create: `crates/git/src/overlap.rs`
- Create: `crates/git/tests/overlap.rs`
- Create: `apps/desktop/src/features/tasks/OverlapWarnings.tsx`

**Steps:**

1. Write failing tests for exact-file, rename, changed-symbol, and one-hop dependency overlap; assert unknown indexes degrade visibly.
2. Implement warnings by comparing active worktree diffs with indexed symbol spans and graph neighbors.
3. Display severity, evidence, other task/worktree, and refresh time without blocking work automatically.
4. Run tests; expect pass.
5. Commit: `feat: warn about parallel work overlap`.

### Milestone E gate

Have Claude implement a small change in its worktree, click Review with Codex, verify the preview contains shared context and exact diff, receive a structured issue, and send it back to Claude. Create a second overlapping task and observe warnings. Run all checks; record adapter versions, pack size, finding outcome, false-positive notes, and debt in `docs/proofs/e-cross-agent-review.md`; update architecture/decisions and commit `docs: record milestone E proof`.

## Milestone F — GitHub issue → task → worktree → PR → reviews/checks

**Scope:** Detect GitHub from a Git remote, authenticate safely, import an issue, run the existing task workflow, push with system Git, create a PR after approval, and display normalized checks/reviews.

### Task F.1: Provider contract and GitHub detection/authentication

**Files:**
- Create: `crates/forge/Cargo.toml`
- Create: `crates/forge/src/provider.rs`
- Create: `crates/forge/src/types.rs`
- Create: `crates/forge-github/Cargo.toml`
- Create: `crates/forge-github/src/remote.rs`
- Create: `crates/forge-github/src/auth.rs`
- Create: `crates/forge-github/tests/remote.rs`
- Create: `crates/storage/migrations/0010_forge.sql`

**Steps:**

1. Write failing detection tests for GitHub HTTPS/SSH/SCP forms, ports, enterprise hosts, malformed URLs, and non-GitHub remotes.
2. Define normalized provider types and implement detection.
3. Add keychain facade tests proving raw tokens never enter SQLite/logs; implement device flow or explicit token onboarding behind `ForgeProvider`.
4. Run tests and an opt-in current-user authentication smoke test.
5. Commit: `feat: connect GitHub accounts and remotes`.

### Task F.2: Issues and task import

**Files:**
- Create: `crates/forge-github/src/issues.rs`
- Create: `crates/forge-github/tests/issues.rs`
- Create: `apps/desktop/src/features/forge/IssueList.tsx`
- Create: `apps/desktop/src/features/forge/ImportIssueDialog.tsx`

**Steps:**

1. Write HTTP fixture tests for pagination, conditional requests, rate limit, error mapping, payload preservation, and idempotent import.
2. Implement issue list/get and explicit Issue → Task conversion with an entity link.
3. Add issue browsing/import UI; show what fields/context will be copied.
4. Run tests and import one issue from a test repository.
5. Commit: `feat: turn GitHub issues into tasks`.

### Task F.3: Pull requests, checks, and review comments

**Files:**
- Create: `crates/forge-github/src/pulls.rs`
- Create: `crates/forge-github/src/checks.rs`
- Create: `crates/forge-github/tests/pulls.rs`
- Create: `apps/desktop/src/features/forge/PullRequestPanel.tsx`
- Create: `apps/desktop/src/features/forge/ChecksPanel.tsx`

**Steps:**

1. Write HTTP fixture tests for PR preview/create idempotency, head/base validation, pagination, check conclusions, review locations, rate limits, and retry-safe failures.
2. Implement system-Git push as a separately approved action, then provider PR creation as another approved action.
3. Implement foreground polling with conditional requests while the PR view is open and normalize checks/comments.
4. Add PR preview, push/create confirmations, checks/reviews, deep links, and “send finding to agent.”
5. Run tests and the opt-in test-repository E2E.
6. Commit: `feat: create and review GitHub pull requests`.

### Milestone F gate

Using a disposable real GitHub repository: import an issue, create its worktree, make/review a change, approve push, preview/create a PR, observe at least one check and review/comment (or document the configured test equivalent), and route a finding back to an agent. Run all checks; record repository, object URLs, API scopes, redacted logs, failure/retry evidence, and debt in `docs/proofs/f-github.md`; update architecture/decisions and commit `docs: record milestone F proof`.

## Final MVP hardening

### Task H.1: Cross-cutting recovery and security tests

**Files:**
- Create: `crates/core/tests/crash_recovery.rs`
- Create: `crates/security/tests/redaction.rs`
- Create: `crates/security/tests/ignore_policy.rs`
- Create: `docs/runbooks/recovery.md`
- Create: `docs/runbooks/security.md`

**Steps:**

1. Add fault injection at database commit, process spawn/exit, Git mutation, artifact copy, index publish, and forge response boundaries.
2. Prove restart reconciliation never loses worktree changes or reports an unconfirmed remote success.
3. Test repository/skill/forge prompt injection remains untrusted data, secrets are redacted, ignored files are absent, and permissions cannot self-escalate.
4. Run all tests and document operator/user recovery actions.
5. Commit: `test: harden recovery and trust boundaries`.

### Task H.2: Killer-demo acceptance suite

**Files:**
- Create: `apps/desktop/e2e/killer-demo.spec.ts`
- Create: `scripts/prepare-demo-repo.sh`
- Create: `docs/KILLER_DEMO.md`
- Create: `docs/MVP_STATUS.md`

**Steps:**

1. Automate all deterministic local portions of proofs A–E against a disposable repository; mark real provider/network steps as explicit opt-in tests, never mocked “passes.”
2. Write the exact human-run killer-demo script with prerequisites and expected UI/evidence.
3. Run the full repository check suite and the real demo twice from a clean local-data directory.
4. In `MVP_STATUS.md`, list what genuinely works, versions tested, known limitations, shortcuts/debt, and every manual prerequisite.
5. Update `ARCHITECTURE.md` and `DECISIONS.md` to match reality.
6. Commit: `release: prove agent-driven IDE MVP`.

## Definition of done

- All six required proofs run through production code paths and have evidence documents.
- A clean checkout builds with documented prerequisites and all non-network tests pass.
- At least one tested Claude Code version and one tested Codex version complete their real roles.
- No primary-working-tree agent edits, silent destructive Git, hidden remote writes, raw stored credentials, or hidden chain-of-thought.
- Context packs and “why” answers expose sources, provenance, and uncertainty.
- `ARCHITECTURE.md`, `DECISIONS.md`, recovery/security runbooks, and MVP status match the shipped behavior.
- Deferred work is explicit; none is presented as functioning UI.
