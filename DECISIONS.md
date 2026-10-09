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
- Decision: Index definitions, imports/exports, lexical references, and conservative calls with tree-sitter. Store confidence and unresolved targets. Add TypeScript compiler/LSP enrichment only after measured gaps. Phase 0 extended the grammar set beyond TS/JS to Python, Go, Rust, JSON, YAML, and Markdown; configuration and prose contribute a keyed property or heading tree and no call graph.
- Rationale: Tree-sitter is incremental, robust to incomplete code, and sufficient to test whether structural retrieval adds value. One extractor shape per language behind a shared fact builder keeps each grammar's conventions -- Go's capitalisation, Rust's `pub`, Python's underscore -- honest rather than averaged.
- Consequence: UI must communicate confidence and must not imply semantic precision where none exists. Every added grammar owes a fixture and named per-symbol expectations.

## ADR-014 — Long-running operations are persisted jobs

- Status: accepted
- Decision: Indexing, agent sessions, forge synchronization, and artifact extraction use durable identifiers, ordered events, cancellation tokens, and resumable checkpoints.
- Rationale: Desktop disconnection and crashes must not erase ownership or leave ambiguous work.
- Consequence: State transitions and reconciliation tests are first-class domain tests.

## ADR-015 — Primary navigation is task-centric

- Status: accepted; the six-item activity rail is superseded by ADR-043
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
- Consequence: Adapter contract fixtures and opt-in real CLI proofs must cover command construction and stream drift. Unsupported fine-grained provider controls remain denied or are disclosed as coarse grants. Launch decisions are durably recorded before the in-memory grant is applied.

## ADR-021 — Agent launch revalidates the exact visible context pack

- Status: accepted
- Decision: Build task context packs in Rust from durable entities, expose the exact serialized pack for preview, and accept only a server-side reconstruction equal to that preview at launch. The client may submit only its visible removal set; it cannot add policy-excluded content or alter evidence. Any intervening context, skill, index, or Git-recency change makes the preview stale and requires review again.
- Rationale: “What the agent saw” must be auditable, while a webview-supplied pack cannot be trusted as authority and a silent rebuild could differ from the user's approval.
- Consequence: Pack generation is deterministic and includes provenance, freshness, source location, inclusion reason, section usage, and hard exclusions. Launch latency includes one bounded rebuild. Required items may exceed soft section shares but never the total byte ceiling; token-aware budgeting and binary provider attachments remain future refinements.

## ADR-022 — Durable memory is a human-governed evidence projection

- Status: accepted
- Decision: Treat post-task model output as strict-schema candidates derived from a bounded untrusted evidence bundle, never as trusted memory. Validate every cited event/diff ID and changed entity, require explicit HUMAN provenance for resolution, preserve rejected/stale/superseded history, and carry semantic relationships forward when a human correction supersedes a record. “Why?” may return only approved facts as facts and must expose conflicts, staleness, insufficient evidence, bounds, and source paths.
- Rationale: A fluent extraction is not authority. Durable rationale is useful only when its evidence can be reopened and a person can correct it without severing the code/task history that made it explainable.
- Consequence: Extraction prompts are part of a tested protocol but the validator remains authoritative and fails closed. Agent-inferred candidates do not enter knowledge FTS until approval. Corrections create new records instead of rewriting history. The deterministic why service prefers source-backed facts, emits no generated inference prose, and may append exact task publication provenance (head commit and hosted PR) without treating it as rationale; broader graph explanations require explicit future evidence rules.

## ADR-023 — Cross-agent review uses an exact, revalidated handoff

- Status: accepted
- Decision: Persist provider-neutral implementation events and build review handoffs from a completed session, the normal deterministic task context pack, selected concise events, and the exact untruncated Git diff with SHA-256 identity. Require a different reviewer adapter, preview the payload, then reconstruct and compare it in the native layer before a read-only review. Accept findings only through a strict schema whose evidence IDs and source lines resolve inside the supplied diff.
- Rationale: Cross-provider review is valuable only when “what the reviewer saw” is inspectable and cannot drift between preview and launch. Full provider transcripts are noisy, provider-specific, and may contain irrelevant or sensitive material.
- Consequence: Oversized diffs fail rather than truncate silently. Raw frames remain diagnostic data and never enter the handoff. Reviews/findings are durable candidates with explicit lifecycle. Review execution still needs migration from a synchronous provider call to the process supervisor for cancellation and reviewer-session audit parity.

## ADR-024 — Forge writes require exact Git and hosted-object previews

- Status: accepted
- Decision: Keep normalized forge entities and workflows in `mars-forge`, with GitHub HTTP details isolated in `mars-forge-github`. Verify account tokens before placing them in the native OS keychain; SQLite stores only opaque credential references. A PR plan captures and fingerprints the fetch URL, actual Git `pushurl`, worktree, head SHA, commit count, cleanliness, head/base/title/body, and draft state. Immediately rebuild that plan, require separate push and forge approvals, push the exact reviewed URL with system Git, then create the hosted PR. Persist issues, PRs, checks, and comments idempotently.
- Rationale: A Git remote name can hide a distinct `pushurl`, webview state is not authority, and a successful push followed by an ambiguous API response must be safely recoverable without silently targeting or duplicating the wrong object.
- Consequence: Changing Git config, commit state, or PR fields invalidates approval. A GitHub 422 after retry is recovered only when the provider returns an open PR with the exact reviewed head/base. Tokens do not enter SQLite or logs. Conditional ETags and rate metadata remain future hardening.

## ADR-025 — Parallel-work warnings are advisory graph-backed evidence

- Status: accepted
- Decision: Compare every ready/active/review worktree from its task base through the current working diff. Emit non-blocking exact-file and rename conflicts plus changed-symbol and one-hop dependency warnings derived from the latest local index. Every warning includes the other task/worktree, evidence identifiers, severity, and refresh time; an incomplete index produces an explicit unknown warning.
- Rationale: Different worktrees prevent filesystem collisions but not semantic conflict. The index is useful as a warning signal, not authoritative enough to block work automatically.
- Consequence: Symbol overlap is conservatively mapped from zero-context diff hunks to stored spans and can produce false positives after large line shifts. Users can refresh at any time, and source/Git remain truth.

## ADR-026 — Security decisions and forge mutations are durable before effect

- Status: accepted
- Decision: Centralize default sensitive-path policy and credential redaction in `mars-security`. Persist every agent capability decision before applying its in-memory grant. For an approved PR publish, insert a forge mutation journal row before Git push, mark it completed or failed after the provider result, and convert abandoned started rows to interrupted at startup. Persist a minimal confirmed PR snapshot before fetching optional comments and checks.
- Rationale: A crash or partial network failure must not turn an attempted external write into an implied success, and a database failure must not produce an unaudited agent grant. Shared redaction prevents diagnostic paths from drifting into inconsistent secret handling.
- Consequence: Recovery exposes uncertainty instead of guessing. Retrying requires a fresh exact preview and approvals; a confirmed provider identity survives secondary refresh failure. The journal stores bounded normalized details and redacted errors, not credentials or full provider payloads.

## ADR-027 — The file layer covers every readable file, with exclusions recorded separately

- Status: accepted
- Decision: Index a row for every non-ignored file in the working tree, including configuration, prose, and binary files, with a language sentinel where no grammar applies. Record each excluded entry in `ignored_paths` with the rule that excluded it, collapsing a pruned directory to one row rather than enumerating its contents.
- Rationale: Freshness and watcher deltas need a complete picture of the tree, not only the files that produce symbols. A consumer that asks why a path is missing deserves the rule, not silence.
- Consequence: File counts exceed source counts and `files` no longer implies "has symbols". The exclusion set is a projection of the walk, so a `.gitignore` edit is reflected only after the next full run.

## ADR-028 — Renames are detected by blob identity, not by path similarity

- Status: accepted
- Decision: Pair a disappeared path with an appeared path holding the same Git blob SHA and republish the file under its new path, keeping its row id and its symbols' stable keys. Leave ambiguous matches, where several identical blobs moved at once, to the ordinary delete-and-add path.
- Rationale: Content identity is what makes a move a move. Preserving file and symbol identity keeps anything that references a symbol valid across a rename, which a delete-and-add cycle would break.
- Consequence: A move combined with an edit is reported as a delete plus an add, because the blob changed. Chunk ids include the path, so a renamed file's chunks are reissued even though its bytes are unchanged.

## ADR-029 — The semantic layer ships a deterministic local embedder

- Status: accepted
- Decision: Embed chunks with a feature-hashing model over case- and punctuation-split identifier tokens, stored as normalized 256-dimension vectors in SQLite alongside the model name. Make no cloud or model-server call. Fuse lexical and semantic results with reciprocal-rank fusion rather than a tuned weighted sum.
- Rationale: Phase 0 forbids network and model calls, and a reproducible embedder means a chunk's vector changes only when its text does. Recording the model name lets a learned embedder be introduced later without a schema change.
- Consequence: Semantic recall captures identifier and path overlap, not meaning; it complements lexical search rather than replacing it. Similarity is scanned in memory per repository, which bounds how far this scales before an approximate index is needed.

## ADR-030 — A Brain answer separates fact from inference, and fails closed

- Status: accepted
- Decision: Model an answer as `claims[]` where each claim is either `fact` — what a cited span of source says, carrying at least one citation — or `inference`, a conclusion that names the claim ids it reasons over. Never merge the two in the data model or in any surface that renders them. Validate the whole answer before returning it: an uncited fact, a fact resting on another claim, a citation the pipeline never supplied, or a duplicate id fails the answer rather than degrading it. Keep `sources`, `retrieved`, `latencyMs`, `confidence`, and `gaps[]` on the answer and surface the counts verbatim.
- Rationale: A reader who cannot tell an observation from a conclusion cannot calibrate trust, and a fluent uncited assertion is exactly the failure mode a retrieval layer exists to prevent. Degrading an uncited fact into a hedge would preserve the fluency and destroy the accountability; refusing the answer makes the producer's error visible. This extends ADR-022's evidence boundary from durable memory to live retrieval.
- Consequence: The validator, not the synthesis provider, is what the contract rests on, so a provider may be swapped without renegotiating trust. A caller must handle a contract error as a real outcome. Surfaces owe two visually distinct regions, and a gap list is part of a complete answer rather than an admission of failure.

## ADR-031 — Citations are resolved against the working tree, not the index

- Status: accepted
- Decision: After synthesis, open every cited file as it is on disk and re-find the cited text. Confirm the span, relabel it when the text has moved within its file, or drop the citation when the text is gone — and drop any claim left resting on nothing, cascading to inferences whose supporting claims all vanished. Report the loss as a `stale` gap. Match on significant lines, ignoring blank lines and indentation, with the first and last lines matched partially because a chunk is a byte range rather than a run of whole lines. Blame an edit for a relabel only when the blob SHA actually changed.
- Rationale: The index is a projection that can lag the working tree, per ADR-003. Between indexing a chunk and rendering an answer its file may have been edited, moved, or deleted, and a citation that no longer resolves is worse than no citation: it is a wrong answer wearing the costume of evidence. Reformatting should not invalidate a citation while deleting the code should, which is why matching is line-significant rather than byte-exact.
- Consequence: Every answer costs one read of each cited file. An answer over a heavily edited tree may return fewer claims than retrieval found, which is the correct outcome and is reported rather than hidden. Claim prose must not embed line numbers, since the citation is the authority on location and validation may move it.

## ADR-032 — Context budget packing keeps symbols whole and drops by marginal value

- Status: accepted
- Decision: Pack the synthesis context against a token budget estimated by a deterministic local approximation rather than any provider's tokenizer. Include a symbol whole or not at all. Select greedily by score-per-token discounted by how much of a candidate is already covered by what is packed, recomputing novelty after each pick, and record a reason for every dropped source.
- Rationale: A truncated function body makes a model describe behaviour that is not there and cite a range that never contained it, so a partial symbol is worse than an absent one. Ranking by score alone lets a cluster of near-identical chunks consume the budget while the source that would actually answer the question is dropped. Depending on a vendor tokenizer would make what fits a function of which provider is configured.
- Consequence: Token figures are approximate and deliberately err high. A symbol larger than the whole budget is never included, and says so. Budget pressure is reported as a `budgetExhausted` gap rather than silently narrowing the answer.

## ADR-033 — Synthesis is an injected provider over a deterministic default

- Status: accepted
- Decision: Define a `SynthesisProvider` trait and ship `DeterministicSynthesizer` as the default, which reads citation-backed structural facts off the packed evidence, makes no network call, and declares the absence of reasoning as a `noReasoning` gap. Give every provider the same system prompt forbidding uncited factual claims and declaring repository content untrusted data, and parse output through a strict schema.
- Rationale: Project Brain must work, and must be testable, with no model configured — the same constraint Phase 0 accepted for embeddings under ADR-029. A default that describes without concluding is honest; one that generates plausible reasoning from a template would be the exact failure the answer contract exists to prevent.
- Consequence: Retrieval, packing, and validation are covered by tests that need no provider. A configured model changes the quality of reasoning but cannot widen what counts as a fact. The rendered prompt is inspectable through the CLI harness, so what a model is asked is auditable without running one.

## ADR-034 — A memory exists only because a person approved it

- Status: accepted
- Decision: Split proposals from memories at the table level. An agent may call only `memory.propose`, which writes to `memory_proposals`; retrieval reads `memories`; and the only path between them is `memory.approve`, which records the approver verbatim and refuses an unattributed approval. Require the exact motivating excerpt on every proposal and keep it on the memory. Record every lifecycle transition — proposed, approved, edited and approved, rejected, revised, superseded, revoked, conflict resolved — with its actor and the conversation turn it happened in, and write the approval as a message in the origin conversation. In the desktop layer, fill every actor field from the local user and discard whatever the client sent.
- Rationale: "The agent must not write memory without approval" is a property that has to be structural, because a rule enforced only by discipline is one refactor away from being untrue. Two tables and one approving function make the invariant something a reviewer can see. Requiring the excerpt is the other half: a durable claim whose origin cannot be reopened is a rumour with a database row, and it is precisely the claims that sound plausible that most need checking. Who approved is the entire authority behind a memory, so accepting it from the webview would let the least trusted layer mint that authority.
- Consequence: There is no bulk-import or auto-approve path, and adding one would have to defeat the table split deliberately rather than by accident. Proposals accumulate until a person works the queue. Origin identifiers are opaque strings until Mars has a conversation store; a supplied transcript source verifies them where one exists and the record says plainly when it could not.

## ADR-035 — Conflicting memory blocks approval and the human chooses

- Status: accepted
- Decision: On approval, compare the candidate against active memories whose scope could apply wherever it would, by embedding similarity. Return anything above the threshold as a conflict and write nothing. Require the retried approval to resolve exactly the set that was detected — keep both, supersede, revoke, or keep the existing and reject this proposal — refusing a set that no longer matches. Label each pair `overlap` or `opposing_polarity` and never as a contradiction. Allow at most one supersede per approval, matching the single `supersedes` link on the record.
- Rationale: Two active memories that disagree make an agent's behaviour depend on which one retrieval happened to surface, which is worse than either being absent. But deciding that two statements contradict each other is a judgement about meaning, and this layer has a feature-hashing embedder and a word list. It can honestly narrow the field and then insist a person decide; claiming to have detected the contradiction would be a confident guess wearing the costume of a check. Refusing a stale resolution set follows ADR-021: a decision made against a view that has since moved is not the decision being applied.
- Consequence: Approving a memory can require a second round trip, and the UI must have no way past the choice. Pairs that genuinely conflict while sharing no vocabulary are not flagged, so this narrows the problem rather than solving it. Threshold tuning trades review burden against missed pairs and is a product decision, not a correctness one.

## ADR-036 — A memory is used when an agent receives it, not when it is retrieved

- Status: accepted
- Decision: Log every injection with its surface, task, session, and selection relevance, and expose the distinct-task count on the record. Let `memory.forRetrieval` log its own selection, because calling it is the injection. Do not log when a context pack is built: a pack is built to be previewed and rebuilt to be revalidated before anything is sent. Record at launch instead, from the pack that was actually sent, by reading its memory items back out.
- Rationale: The usage trail is what turns a memory from an assertion into something with a record of influence, and "used in 4 tasks" is only worth showing if it is true. A preview the human is still editing has shaped nothing; ADR-021's revalidation rebuild has shaped nothing; and counting either would inflate the figure exactly where a person is deciding whether to trust it. Reading the sent pack is also strictly more accurate than re-deriving a selection that may have moved on between the two moments.
- Consequence: Any future surface that injects memory owes an explicit recording call at the point of injection; there is no ambient logging to rely on. A memory the human removed from the preview is correctly never recorded. Usage rows accumulate without bound and will need retention before the count becomes expensive to compute.

## ADR-037 — The activity record is the single source of truth, and append-only in the database

- Status: accepted
- Decision: Give every task one append-only log, `activity_events`, with a monotonic per-task sequence assigned inside the insert, wall-clock time, an optional duration, and either an inline payload or the digest of a content-addressed one. Enforce immutability with a `BEFORE UPDATE` trigger that aborts and a `BEFORE DELETE` trigger that aborts unless the parent task has already gone, so rows leave only with their task. Derive every UI state — timeline, files touched, pending permissions, run status, crash recovery, export — by folding the log, never by reading a status column. Keep `agent_runs.status` as an index for querying, not as an authority.
- Rationale: Reload, crash recovery, and export are the same question asked at different times, and they disagree exactly when each is computed from a different place. One fold over one log makes agreement structural rather than a thing to keep in sync. Putting immutability in the schema matters more than putting it in the service: a log that the code merely promises not to rewrite is one bug fix away from being rewritten, and a transcript that can be edited is not evidence of anything. The delete guard is the one concession to reality — a deleted task must be able to take its history with it.
- Consequence: Anything the UI wants to show must be expressible as a fold over events, which is a real constraint on what gets recorded and forces payloads to be complete at write time. Corrections are new events, never edits, so a mistake is visible forever. Payload objects and their rows are never reclaimed, and retention will be needed before long-running tasks become expensive.

## ADR-038 — Capability ceilings are constructed, not requested

- Status: accepted
- Decision: Give each agent role a `CapabilityCeiling` at construction — a reviewer gets `write_files: false`, no network, no push, and a read-only command ceiling — and pass that ceiling to the permission broker with every request the agent makes. Refuse an effect above the ceiling outright: create no pending request, hand out no ticket, and present nothing for a human to approve. Record the refusal as the system's decision with `byConstruction: true`. Apply the same ceiling when the launch command is built, so a read-only agent is started without write tools, and filter the previewed tool list by it so a preview cannot promise what the broker would deny.
- Rationale: "The reviewing agent gets read-only capabilities by construction, not by instruction" is the whole security value of cross-agent review, and a prompt is not a mechanism — an instruction is a suggestion to a system whose output is not controlled. A dialog is not a mechanism either: any prompt a person can approve is a capability the agent effectively has, given a plausible-sounding reason and a tired reader at the end of a long session. The only version of this that holds is one where the answer is unavailable. Recording the refusal keeps it honest: a silent block is indistinguishable from a bug.
- Consequence: A reviewer that needs to run something genuinely read-only but unrecognised by the classifier is stuck, and the fix is the classifier, not an override. There is deliberately no escalation path from a ceiling refusal, so mis-scoping a role means restarting the run with a different one. The ceiling is duplicated in three places — command construction, broker check, and pack preview — and they must be changed together.

## ADR-039 — A blocked agent stops, and a refusal ends the run

- Status: accepted
- Decision: When an effect is not covered by a live grant, write a `pending` row, emit the request into the task's activity record, and block the agent's event pump on a `PermissionTicket` until a person decides. Show the request in the task conversation rather than a modal. On a denial — by a person or by the ceiling — emit an `error` event saying why, cancel the process, settle the run to `cancelled`, and only then emit `done`, so a consumer reacting to the ending never reads a stale status. Offer four answers: allow once, allow for this task, allow for this pattern, deny. Make grants durable, scoped, expiring, and revocable, with revocation recorded as a timestamp rather than a delete.
- Rationale: An agent that keeps working while a question is outstanding has already answered it. Blocking the pump rather than the process is what makes "stop and wait" true of the events a person sees, and stopping the run on a denial is the honest reading of "no" — a CLI cannot be told mid-stream that its last action was disallowed, so the alternative is letting it continue as though it had been allowed. A modal makes the decision transient; the conversation makes it part of the record, which is where a question about who allowed what belongs. Expiry exists because "allow for this task" said at 2pm should not still be true tomorrow, and revocation is a write so the transcript keeps saying the grant once existed.
- Consequence: A stuck agent is a real failure mode — nobody answering means nothing happens — and the UI has to make pending requests impossible to miss. A denial costs the whole run rather than the one action, which is blunt and will be irritating when the human meant "not that, but carry on". Grant expiry is checked against SQLite's clock, so every timestamp the broker writes or compares comes from the database rather than the process.

## ADR-040 — A worktree plan pins its base, and is fingerprinted over what it decides

- Status: accepted
- Decision: Produce the worktree plan without mutating anything, pinning the commit the base ref resolved to. Fingerprint it over what it decides — base commit, branch, path, expected file scope, setup commands — and deliberately exclude free space and preflight findings from that digest. Record approval against the fingerprint, and rebuild and compare it before provisioning. Re-check the findings separately at both points. When the base moves, keep the pinned commit and raise a blocking `base_moved` finding naming both commits, offering rebase, recreate, or continue-and-diverge, each with its consequence; record a choice to diverge so the same question is not asked twice.
- Rationale: An approval is a statement about a specific plan, so provisioning must be able to prove it is acting on that plan. But a digest over everything the planner observed is a digest over a moving world: free space changes between two calls, which would invalidate an approval seconds after it was given and train people to re-approve reflexively. Splitting decisions from observations is what makes the fingerprint mean something. Pinning the base is the same idea in the other direction — silently following a moved ref would change what the agent starts from without anyone saying so, which is exactly the class of surprise this phase exists to remove.
- Consequence: A plan can be approved while a warning stands, because warnings are outside the digest; blocking findings are re-checked at provisioning instead. Moving a plan to a new base withdraws its approval, so drift always costs a second look. The acknowledgement of a specific drifted commit is stored, which means a base that moves twice asks twice.

## ADR-041 — Cleanup archives before it removes

- Status: accepted
- Decision: Make worktree removal a two-step operation with a preview. `plan_cleanup` reports the branch tip, the commits ahead of base, the uncommitted files removal would discard, and whether the operation is reversible, and fingerprints that view. `cleanup` refuses a stale fingerprint, refuses to discard uncommitted work unless explicitly told to, writes the branch tip to `refs/mars/archive/<branch>` before touching the working tree, and only then removes the worktree and deletes the branch. `restore` recreates the worktree at the same path from the archived commit. Perform the removal directly rather than through the ordinary worktree service, whose refusal to delete an unmerged branch is correct only when the commits are not already archived.
- Rationale: "Cleanup is explicit and reversible" cannot mean much if the reversal depends on the reflog, which expires and which no user reads. An archive ref is a durable, inspectable, ordinary Git object that survives the removal it protects. Archiving first is the ordering that matters: everything after it can fail without losing work. Uncommitted changes are the one thing no ref can bring back, which is why they are named individually in the preview and require a separate deliberate flag rather than being folded into a general force.
- Consequence: Archive refs accumulate under `refs/mars/archive/` and nothing prunes them, which is the intended trade — they are cheap and losing work is not. Bypassing the worktree service means the two removal paths must stay in agreement about the durable rows they update.

## ADR-042 — The context pack is the prompt, and its digest is over that prompt

- Status: accepted
- Decision: Extend the retrieval pack into a `RunContextPack` that also carries the tools the agent is given and the permissions it holds — role, ceiling, declared file scope, and the standing grants it will not be asked about. Render all of it into one string, launch the agent with exactly that string, and take the digest over it. Filter the tool list by the same ceiling that will refuse the effect later. Revalidate before launch by rebuilding and comparing digests, refusing a pack that changed after approval.
- Rationale: "The UI shows it verbatim before a run" is only true if the thing shown and the thing sent are the same object. Digesting a structured description rather than the prompt leaves room for the renderer to differ from the preview, which is exactly where an audit trail stops being one. Putting permissions inside the prompt has a second effect worth having: the agent is told what it may do in the same words the person read, so the two cannot quietly disagree. Filtering tools by the ceiling closes the gap where a preview advertises a capability that the broker would refuse on first use.
- Consequence: Any change to prompt rendering invalidates every approved pack, which is correct but means renderer changes are user-visible. The prompt is stored by digest on the run and as an activity payload, so a task's log carries every prompt its agents were sent.

## ADR-043 — The shell is three destinations around one conversation

- Status: accepted
- Decision: Build the desktop shell as a single component system in `apps/desktop/src/shell` before building screens on it. The app rail carries exactly three destinations — Conversation, Editor, Source control — plus theme and account. The conversation is always present and always live: `editor` and `source-control` open a workbench beside it rather than replacing it, and the composer stays pinned under it and usable while diffs, editors and panels are open. The task rail, inspector and utility panel collapse only on explicit user action, never as a side effect of navigation. Every surface reports one of eight shared states — `idle`, `queued`, `loading`, `streaming`, `blocked`, `failed`, `stale`, `done` — rendered identically wherever it appears, distinguished by mark and motion rather than by hue, with red reserved for failure. Design tokens are dark-first near-black panels, one warm accent used only for state and current position, 1px hairlines instead of shadows, near-square corners, IBM Plex Mono for machine-authored text and a grotesk for human text. Every region is reachable and dismissible from the keyboard.
- Rationale: ADR-015 put six activities in the rail, which made the rail a feature list and left the agent stream as one tab among many. The workflow it describes — intent, context, delegation, review, integration, memory — is a conversation with an agent that happens to open files and diffs, so the conversation is the thing that must never be swapped out from under the person watching it. Three destinations is the smallest set that still lets someone read code and review changes without leaving the run. A shared state vocabulary matters more than it looks: without it every screen invents its own words for "we are waiting" and "this is out of date", and a person cannot learn the product once. Encoding state in shape and motion rather than colour keeps the single accent meaningful and does not ask a colour-blind reader to tell amber from amber.
- Consequence: Tasks are no longer a rail destination; the task rail is always present instead, so a conversation always has a branch and a worktree behind it. Screens that used to bring their own chrome — headers, tabs, empty states — must give it up and compose shell regions, which is migration work for every existing feature panel. The three-destination rail has no room for Brain, Context, Skills or Git/PRs as peers; those become panels inside the conversation, the inspector, or the utility panel, and anything that finds no home there is a signal that the destination set is wrong rather than a reason to add a fourth icon.

## ADR-044 — Agents run on the person's own sign-in, and Mars never handles it

- Status: accepted
- Decision: Mars runs the installed, unmodified `claude` and `codex` CLIs and never signs anyone in, stores a provider token, or reads one. Before a run it asks the CLI what it is signed in with (`claude auth status --json`, `codex login status`) and refuses to launch an agent that is certainly signed out, with the error code `agent.signed_out`. Signing in means typing the CLI's own command (`claude auth login`, `codex login`) into Mars's terminal, in view. It also checks its own environment for variables that outrank that sign-in in a non-interactive run — `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_API_KEY` for Claude, `CODEX_API_KEY` for Codex — and says before the run which account it will bill. A person may choose their sign-in over such a variable; Mars then removes it from that agent's environment for task runs, reviews and memory extraction, records the removed names (never values) with the session, and remembers the choice in `agent-sign-in.json` in its data directory.
- Rationale: Anthropic does not allow third-party products to offer Claude.ai login or to handle subscription credentials, but does allow an end user to sign in to the unmodified Claude Code binary with their own subscription; OpenAI documents Codex's app-server and login for host applications. Running the CLI as the user installed and signed into it is the arrangement that keeps a person's subscription theirs. The environment check exists because `claude -p` uses `ANTHROPIC_API_KEY` whenever it is present, without the one-time approval it asks for in a terminal, so a key exported for another project would quietly move billing from the person's plan to the key. Removing a variable is the only lever Mars has over billing that does not involve touching a credential.
- Consequence: Each launch costs one extra short CLI call. A CLI too old to report its sign-in reads as `unknown` and is allowed to launch and fail in its own words, rather than being refused on a guess. The default is the CLI's own precedence, so behaviour matches the terminal until someone chooses otherwise. Bundling the CLIs, hosting them, or paying for usage on someone's behalf would fall under different provider terms and is out of scope for this decision.

## ADR-045 — A task's conversation is one provider session, and a follow-up sends only what was said

- Status: accepted
- Decision: The first run of a task is launched from the context pack a person reviewed (ADR-042). Every later message in that task resumes the provider's own session — `claude --resume`, `codex exec resume` — with exactly the words the person typed as the prompt, recorded first as a human message in the task's log. The provider's session id is kept on the run (`agent_runs.provider_session_id`, migration 18) and each follow-up names the run it continues (`resumes_run_id`). A message written while an agent is still working in the task waits in the composer's queue and is sent when that run ends; a task holds one running agent at a time, refused in the core with `agent.busy`. Prompts reach every agent CLI on standard input, never as an argument.
- Rationale: A conversation the agent cannot remember is not a conversation, and resending the whole pack on every turn would pay for the same context again and again — the cost this product exists to remove. The provider already holds the session; Mars's job is to remember which one it was, which the old launch path did only in a React component's memory. Standard input is the only channel that carries a 64 KiB pack on Windows, whose command line stops near 32 K characters, and an argument is readable by every process that lists command lines.
- Consequence: What a follow-up was told is exactly what the person wrote, so the record of it is the message itself and its digest. A follow-up launched after the pack's inputs changed does not see the change unless the person says so; the pack was what the agent started with. Writing into a running agent mid-turn is not offered: the message waits.

## ADR-046 — Every run is given Mars as an MCP server, served by the app's own executable

- Status: accepted
- Decision: Each run starts with an MCP server scoped to its task: `mars-desktop mcp --database … --repository … --task … --run … --worktree … --role …`, configured through `--mcp-config` for Claude Code and `-c mcp_servers.mars.*` for Codex. The server is `crates/mcp`, linked into the desktop binary and also built as `mars-mcp`, so nothing extra ships. It offers the project graph (`search`, `find_symbol`, `symbols_at`, `references`, `callers`, `callees`, `impact`, `related`, `file_deps`, `ask`, `where_is`, `explain`, `index_status`), approved knowledge (`memory`, `decisions`), and two writes that wait for a person (`record_decision`, `remember`). Results are compact text — `path:start-end`, short snippets, ids only where a follow-up call needs them — and say so plainly when the index is missing or stale. A reviewer is offered no tool that writes.
- Rationale: Mars has already indexed the repository into symbols, edges, chunks and history; an agent that cannot ask it re-learns the code by listing directories and grepping, on every run, paying for every line. The only interface both CLIs speak for this is MCP. Serving it from the app's executable avoids a sidecar that has to be built per target and kept in step with the database it reads. Writing memory and decisions only as proposals keeps ADR-034's table split true when the proposer is an agent in another process.
- Consequence: The MCP server opens the same SQLite file as the app, as a second process; WAL and the five-second busy timeout make that safe, and its writes are short. A quote an agent offers for `remember` is verified against the task's conversation when it is found there and recorded as unverified when it is not. The graph reflects the repository as last indexed, not the agent's own uncommitted edits.

## ADR-047 — Claude asks Mars before acting; a refusal is an answer, not the end of the run

- Status: accepted; refines ADR-039 for agents with a permission channel
- Decision: Claude Code runs with `--permission-mode default` and `--permission-prompt-tool mcp__mars__permission_prompt`, with only reading and self-organising tools on its allow-list. Before any other action the CLI calls Mars's tool and waits. Read-only commands and writes inside the plan's declared scope are allowed at once; writes outside the worktree or into `.git` are refused; everything else goes to the permission broker, which answers from a standing grant or records a pending request in the task's conversation and waits — across processes, by reading the decision back from SQLite — for a person. A refusal is returned to the agent as a denial it must not work around, and the run continues. When a run ends, any request it left waiting is withdrawn as a decision by Mars. Codex, which cannot ask in `exec` mode, runs inside its own sandbox with the network off, and its prompt says plainly that a refused action fails rather than asks.
- Rationale: The earlier pump judged events the CLI had already reported, so a command it disliked had usually already run; that is theatre, not a gate. The prompt tool is the CLI's own pre-action hook, so "stop and ask a person" becomes true before the effect. With a real channel, ADR-039's blunt "a denial ends the run" is no longer necessary: the agent is told no, in words, and can take another route — which is what a person meant by no in almost every case.
- Consequence: A pending request blocks the agent until someone answers, so the app watches the logs of running tasks and nudges the screen when another process writes to them. `MCP_TOOL_TIMEOUT` is raised for Claude runs so a person deciding is not mistaken for a tool hanging. The capability ceiling is still applied by construction: a reviewer's write is refused whoever answers. The older launch path with ticked tools and no prompt tool remains for the task detail screen and keeps `dontAsk`.

## ADR-048 — Mars finds the agent CLIs a person has, and says where

- Status: accepted
- Decision: Resolve each agent CLI in order: a path chosen in Settings (`agent-executables.json`), then `PATH` honouring `PATHEXT` and preferring a real `.exe` to a script shim, then the installers' own locations (`~/.local/bin`, `%APPDATA%\npm`, Homebrew, Cargo, Volta, Bun), then the copies editor extensions ship. Report the path and how it was found, and list where Mars looked when nothing was. Every launch, probe, sign-in check, review and extraction uses the resolved path.
- Rationale: Spawning the bare name `claude` found nothing on a Windows npm install — the shim is `claude.cmd`, which a bare-name spawn never resolves — and nothing at all in an app opened from the macOS Finder, which does not inherit the shell's `PATH`. The editor extensions ship the vendor's own unmodified binaries, signed in with the same account, so they are the person's CLI rather than a substitute (ADR-044).
- Consequence: A machine with several copies runs the first found, and Settings says which; choosing one is explicit and remembered.

## ADR-049 — Normalized agent streams pair calls with results and keep no reasoning

- Status: accepted; extends ADR-006
- Decision: Tool, command, test and file events carry the provider's call id, and a `tool_result` event carries the call's output (bounded to 64 KiB, head and tail kept), error flag and exit code. Claude's `thinking` blocks and Codex's `reasoning` items are dropped in the parser and never reach the log, the transcript or a handoff. Frames that are the CLI's own bookkeeping — status, compaction, rate-limit notices that are not limiting — are not reported; frames Mars cannot read are recorded as diagnostics that do not fail the run. The projection updates a command or test from its completion instead of listing it twice, and a run someone stopped replays as `cancelled`. Process output is decoded as a stream, so a character split across two reads stays one character, and a reader that falls behind the live channel recovers the gap from the durable record.
- Rationale: Every real Claude stream reports tool results as `user` frames, which Mars read as unrecognised and logged as errors — and the projection marks a run failed on its first error, so every real run looked failed. The same path stored thinking text in the error's attachment, breaking Architecture §17.7. Without results, the Tests panel and failure evidence had nothing to read.
- Consequence: Agent-originated diagnostics are kept but quiet. Adding a provider means mapping its call ids, or its commands appear without outcomes.

## ADR-050 — Agents is a destination: every run, on one timeline, beside the conversation

- Status: accepted; supersedes ADR-043's limit of three destinations
- Decision: Add a fourth destination, Agents (`Mod+4`), that opens a workbench beside the conversation like Editor and Source control do. It shows every run in the repository on a shared clock — one lane per task, one bar per run, a follow-up joined to the run it continued, a reviewer's bar hollow because it cannot write, a run waiting on a person drawn as blocked — and below it the same runs as a list, waiting-on-you first, with branch, elapsed time, files touched, tokens and reported cost, and Stop for anything live. The task rail and status bar draw from the same live set, so a glance says which of several agents needs an answer.
- Rationale: ADR-043 asked that anything without a home be read as a sign the destination set was wrong rather than a reason to add an icon. Running several agents at once is now the ordinary case, and "which of them is working, which is stuck on me, and what did they cost" has no home in a single task's conversation, the inspector, or the utility panel: it is a view across tasks. The conversation remains present and live on every destination, which is the part of ADR-043 that matters.
- Consequence: The rail has four destinations. The board reads `repository_runs` and `active_runs`, refreshed when any run starts or finishes; its clock ticks only while a run is live and the board is open.
