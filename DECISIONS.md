# Architecture Decision Log

Status values: proposed, accepted, superseded. These decisions are proposed together as the internally consistent MVP baseline; implementation discoveries may supersede them with evidence.

## ADR-001 — Reusable Rust core behind a versioned protocol

- Status: accepted
- Decision: Put repository, process, indexing, storage, security, and forge lifecycle in Rust domain crates. Keep Tauri commands/events as a thin adapter. Define a versioned protocol consumable by React and future CLI/headless transports.
- Rationale: Machine lifecycle needs native process/filesystem control, while the same functionality must later run without a GUI.
- Consequence: UI cannot call Git, SQLite, or agent CLIs directly. Protocol compatibility tests are mandatory.

## ADR-002 — SQLite + FTS5 is the sole MVP metadata database

- Status: accepted
- Decision: Use a bundled SQLite build with FTS5 enabled, WAL mode, append-only migrations, a single write coordinator, and explicit graph edge tables. Do not add Neo4j or a vector database.
- Rationale: The product is local-first and the MVP graph fits indexed relational adjacency queries. One durable store simplifies backup and crash recovery.
- Consequence: Polymorphic entity links are application-validated. Optional embeddings must fit behind retrieval interfaces.

## ADR-003 — Source files, Git, and original artifacts remain truth

- Status: accepted
- Decision: Treat Project Brain as a versioned, rebuildable projection. Preserve original context artifacts; store extracted text/classification as derived data.
- Rationale: Incorrect or stale machine inference must never silently replace primary evidence.
- Consequence: Every answer can open its source. Reindex/re-extraction is safe.

## ADR-004 — System Git performs all MVP mutations

- Status: accepted
- Decision: Invoke the user’s Git executable with structured argument arrays for branch, worktree, commit, fetch, push, merge, and rebase operations.
- Rationale: This preserves SSH agents, credential helpers, signing, hooks, LFS, and existing configuration.
- Consequence: Git version/capability probing and porcelain parsers need fixtures. A library may later accelerate read-heavy operations only.

## ADR-005 — A task owns one implementation branch/worktree

- Status: accepted
- Decision: Provision a deterministic isolated branch/worktree for each autonomous task. One implementing session holds its lease; reviewers are read-only or use a separate review workspace.
- Rationale: Ownership makes parallel work, recovery, overlap detection, and handoff understandable.
- Consequence: No agent edits the user’s primary working tree by default. Cleanup is an explicit lifecycle action.

## ADR-006 — Normalize agent streams, preserve redacted raw frames

- Status: accepted
- Decision: Each CLI adapter translates provider output to ordered normalized events while retaining bounded, redacted raw frames for diagnostics.
- Rationale: UI, memory extraction, and handoffs must not depend on Claude/Codex output formats, but parsing failures must be diagnosable.
- Consequence: Unknown provider frames degrade to `Unknown`; they do not crash a session.

## ADR-007 — Deterministic retrieval precedes embeddings

- Status: accepted
- Decision: Start with explicit attachments, entity matching, bounded graph traversal, FTS/BM25, provenance authority, Git recency, and task affinity. Keep semantic retrieval optional.
- Rationale: The baseline is local, testable, explainable, and avoids model/download/privacy dependencies.
- Consequence: Golden retrieval fixtures and score explanations are required before adding embeddings.

## ADR-008 — Durable memory is evidence-backed and human-correctable

- Status: accepted
- Decision: Extract concise decisions, constraints, outcomes, risks, attempts, questions, and test evidence from normalized sessions plus diffs. Store provenance/confidence; allow approve, edit, reject, pin, stale, and supersede actions.
- Rationale: Useful project memory needs accountability without retaining hidden reasoning.
- Consequence: Agent-inferred facts never silently override human-approved facts.

## ADR-009 — Context artifacts use content-addressed local storage

- Status: accepted
- Decision: Store binaries outside SQLite by SHA-256 digest and metadata/links inside SQLite. Preserve originals and exact-deduplicate within policy scope.
- Rationale: Database size stays manageable and original evidence remains immutable and verifiable.
- Consequence: Database backup and artifact backup are coordinated but distinct; garbage collection must follow reachability and grace periods.

## ADR-010 — Skills require inspection and explicit trust

- Status: accepted
- Decision: Separate discovery, quarantine, inspection, trust, installation, scoping, and activation. Pin source/version/hash and show `SKILL.md` before first activation.
- Rationale: Skills are executable instructions and therefore a supply-chain boundary, not ordinary content.
- Consequence: Suggested skills are not automatically trusted. Most-specific deny policy wins.

## ADR-011 — Forge core uses normalized provider entities

- Status: accepted
- Decision: Define a provider-independent `ForgeProvider`; implement GitHub in a separate crate and retain opaque provider payloads for lossless caching.
- Rationale: GitHub is first class at launch without making Git synonymous with GitHub.
- Consequence: GitLab/Bitbucket/Gitea/Forgejo can be added without UI/domain rewrites.

## ADR-012 — Human approval gates destructive and remote mutations

- Status: accepted
- Decision: Require previews and explicit approval for merge/rebase application, worktree force removal, push, PR creation, and other forge writes. Never auto-merge in MVP.
- Rationale: Agent autonomy must not obscure irreversible or externally visible actions.
- Consequence: Permissions are capabilities with narrow scopes; the audit log records decisions.

## ADR-013 — Tree-sitter provides the first structural graph

- Status: accepted
- Decision: Index TS/JS definitions, imports/exports, lexical references, and conservative calls with tree-sitter. Store confidence and unresolved targets. Add TypeScript compiler/LSP enrichment only after measured gaps.
- Rationale: Tree-sitter is incremental, robust to incomplete code, and sufficient to test whether structural retrieval adds value.
- Consequence: UI must communicate confidence and must not imply semantic precision where none exists.

## ADR-014 — Long-running operations are persisted jobs

- Status: accepted
- Decision: Indexing, agent sessions, forge synchronization, and artifact extraction use durable identifiers, ordered events, cancellation tokens, and resumable checkpoints.
- Rationale: Desktop disconnection and crashes must not erase ownership or leave ambiguous work.
- Consequence: State transitions and reconciliation tests are first-class domain tests.

## ADR-015 — Primary navigation is task-centric

- Status: accepted
- Decision: Make Tasks the default workspace, with Explorer, Brain, Context, Skills, and Git/PRs as peer activities. Agent chat/stream is part of a task rather than the product shell.
- Rationale: The principal workflow is intent → context → delegation → review → integration → memory.
- Consequence: A user can still browse/edit/terminal directly, but the main information architecture cannot regress to “editor plus chat sidebar.”

## ADR-016 — shadcn/ui is the desktop component foundation

- Status: accepted
- Decision: Use Tailwind CSS v4, semantic theme tokens, and shadcn/ui components stored as local source. Start components in `apps/desktop`; promote them to `packages/ui` only after a second real consumer exists.
- Rationale: Mars needs an inspectable, adaptable design system without binding product UI to an opaque runtime component package. App-local ownership avoids premature shared abstractions.
- Consequence: New UI checks the shadcn registry and current component documentation before inventing primitives. Layout may use utility classes, while component color and typography flow through semantic tokens and variants.

## ADR-017 — Repository trust is inspect-before-persist

- Status: accepted
- Decision: Opening a repository first canonicalizes its root, resolves directory or worktree Git metadata, and presents the exact root plus the read, index, and Git-observation capabilities. Mars persists a project only after explicit confirmation. Reopening the same canonical root returns the existing project atomically.
- Rationale: Selecting a directory is not consent to index it, and worktrees may share metadata outside their visible root. Inspection needs to be mutation-free and concurrency-safe.
- Consequence: Repository trust does not grant file writes, command execution, network access, or Git mutation. Those remain separately permissioned. Canonical local paths are machine-specific metadata and repository contents remain external to the profile database.

## ADR-018 — Human editing and terminals stay behind trusted repository identities

- Status: accepted
- Decision: Explorer, editor, and terminal commands accept a persisted repository identifier rather than a client-supplied root. The Rust core resolves the canonical trusted root, applies Git/product/default ignore policy, rejects path traversal and symlink escapes, limits editable files to 2 MiB, and saves against a SHA-256 content version through same-directory replacement. A terminal launches the user's default shell only after an explicit click, starts in that root, streams raw PTY bytes, and retains at most 1 MiB of native scrollback.
- Rationale: Webview paths are untrusted input, external tools may edit files concurrently, and PTY byte streams are not safely decoded one arbitrary read chunk at a time.
- Consequence: Human saves can surface a disk-conflict instead of overwriting an external edit. Binary, oversized, ignored, sensitive, and out-of-root files are not editable. Agent command permissions remain a separate later capability layer; this decision covers direct human IDE actions.

## ADR-019 — Structural graph queries are bounded and evidence-bearing

- Status: accepted
- Decision: Every Project Brain structural query is scoped to a persisted repository identity and requires an explicit result limit. Recursive related-code and blast-radius queries additionally require a depth, with hard service maxima of 100 results and depth 5. Returned relationships retain confidence plus parser/source-span evidence suitable for direct source navigation.
- Rationale: Local graph traversal is still an availability boundary, especially when invoked by an agent or rendered interactively. Fixed server-side ceilings make latency and payload size predictable, while evidence prevents a syntax-derived relationship from being presented as unexplained semantic truth.
- Consequence: Clients cannot request an unbounded graph dump. Broader exploration must page or issue narrower follow-up queries, and future ranking/enrichment must preserve evidence even when it changes ordering or confidence.

## ADR-020 — Agent CLIs run below an explicit capability ceiling

- Status: accepted
- Decision: Launch provider CLIs only inside a ready task worktree and translate the user-approved file, command, and provider-tool grants into the narrowest supported CLI arguments. Never use dangerous permission-bypass flags. Codex uses `read-only` or `workspace-write` sandboxing; Claude receives an explicit allowed-tool set. Mars still supervises the process group, redacts persisted command/output data, and owns cancellation independently of provider behavior.
- Rationale: Provider-native permissions differ and can drift, so they are defense in depth rather than the product authorization source. A provider cannot widen the capability decision recorded before launch.
- Consequence: Adapter contract fixtures and opt-in real CLI proofs must cover command construction and stream drift. Unsupported fine-grained provider controls remain denied or are disclosed as coarse grants; durable permission-decision audit wiring is required before remote Git or forge mutations.

## ADR-021 — Agent launch revalidates the exact visible context pack

- Status: accepted
- Decision: Build task context packs in Rust from durable entities, expose the exact serialized pack for preview, and accept only a server-side reconstruction equal to that preview at launch. The client may submit only its visible removal set; it cannot add policy-excluded content or alter evidence. Any intervening context, skill, index, or Git-recency change makes the preview stale and requires review again.
- Rationale: “What the agent saw” must be auditable, while a webview-supplied pack cannot be trusted as authority and a silent rebuild could differ from the user's approval.
- Consequence: Pack generation is deterministic and includes provenance, freshness, source location, inclusion reason, section usage, and hard exclusions. Launch latency includes one bounded rebuild. Required items may exceed soft section shares but never the total byte ceiling; token-aware budgeting and binary provider attachments remain future refinements.

## ADR-022 — Durable memory is a human-governed evidence projection

- Status: accepted
- Decision: Treat post-task model output as strict-schema candidates derived from a bounded untrusted evidence bundle, never as trusted memory. Validate every cited event/diff ID and changed entity, require explicit HUMAN provenance for resolution, preserve rejected/stale/superseded history, and carry semantic relationships forward when a human correction supersedes a record. “Why?” may return only approved facts as facts and must expose conflicts, staleness, insufficient evidence, bounds, and source paths.
- Rationale: A fluent extraction is not authority. Durable rationale is useful only when its evidence can be reopened and a person can correct it without severing the code/task history that made it explainable.
- Consequence: Extraction prompts are part of a tested protocol but the validator remains authoritative and fails closed. Agent-inferred candidates do not enter knowledge FTS until approval. Corrections create new records instead of rewriting history. The current deterministic why service prefers source-backed facts and emits no generated inference prose; broader graph explanations require explicit future evidence rules.

## ADR-023 — Cross-agent review uses an exact, revalidated handoff

- Status: accepted
- Decision: Persist provider-neutral implementation events and build review handoffs from a completed session, the normal deterministic task context pack, selected concise events, and the exact untruncated Git diff with SHA-256 identity. Require a different reviewer adapter, preview the payload, then reconstruct and compare it in the native layer before a read-only review. Accept findings only through a strict schema whose evidence IDs and source lines resolve inside the supplied diff.
- Rationale: Cross-provider review is valuable only when “what the reviewer saw” is inspectable and cannot drift between preview and launch. Full provider transcripts are noisy, provider-specific, and may contain irrelevant or sensitive material.
- Consequence: Oversized diffs fail rather than truncate silently. Raw frames remain diagnostic data and never enter the handoff. Reviews/findings are durable candidates with explicit lifecycle. Review execution still needs migration from a synchronous provider call to the process supervisor for cancellation and reviewer-session audit parity.
