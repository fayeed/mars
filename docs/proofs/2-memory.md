# Phase 2 memory proof

Status: passing

Mars's memory system. Two rules govern it: every memory is human-approved, and
every memory is traceable. Nothing is written from inference alone.

Both rules are enforced structurally rather than by discipline. An agent can
only reach `memory.propose`, which writes to `memory_proposals`; retrieval reads
`memories`; and the only code path between them passes through `memory.approve`
and records who approved it. A proposal without the excerpt that motivated it is
refused outright, because a memory nobody can trace is not a memory.

## Record

```text
{ id, text, scope: repo|task|global,
  kind: convention|decision|preference|fact|caution,
  origin: { conversationId, messageId, quotedText, verified },
  approvedBy, approvedAt, confidence, supersedes?,
  status: active|superseded|revoked,
  useCount, usedInTasks }
```

`origin.verified` records whether the excerpt was actually checked against the
turn it claims to come from. When a transcript source is configured the check is
real and an invented excerpt is rejected; without one the trace is recorded but
unconfirmed, and both the API and the UI say so rather than implying more
certainty than exists.

The `uses[]` of the brief is served by `memory.usage(id)` rather than inlined on
every record, so listing memories never carries an unbounded array. `useCount`
and `usedInTasks` are on the record itself, because "used in 4 tasks" is
something a list view needs.

Scope is a promise about where a memory applies, so the row carries what that
promise needs: a task-scoped memory has a task, a repo-scoped memory has a
repository, and a task-scoped memory also stores its repository so that scope
overlap can be resolved without a second lookup.

## Lifecycle

**Proposal.** `memory.propose` queues a candidate with the exact excerpt.
Proposals are never applied and never retrieved. The same claim from the same
turn is one proposal however many times an agent restates it.

**Approval.** `memory.approve` accepts, edits and accepts, or reports conflicts.
The approver is recorded verbatim; an unattributed approval is refused. Editing
before accepting stores the edit and records the transition as
`edited_and_approved`, distinct from `approved`, while the origin excerpt still
points at what the agent originally heard.

The approval is itself a message: it is written to `memory_events` with the
origin conversation and a newly minted message id, so a transcript view can
render it inline where it belongs rather than as out-of-band metadata.

**Retrieval.** `memory.forRetrieval(context)` selects active memories whose
scope applies, ranks them by embedding relevance to the work at hand, and
prefers the narrower scope: a rule written about this task outranks an equally
similar rule written about everything. Every injection is logged with its
surface, task, session, and the relevance it was selected at.

**Supersede and revoke.** Editing an approved memory creates a new version
linked by `supersedes`; the old text stays readable because what was once
believed is part of the audit trail. Revoking withdraws a memory from retrieval
while keeping it, with who revoked it and why. There are no hard deletes, and
history does not branch: a superseded memory cannot be revised again.

**Conflict detection.** On approval, active memories whose scope could apply
wherever the new one would are compared by embedding similarity. Anything above
the threshold is returned as a conflict and *nothing is written* until the
person chooses: keep both, replace the old one, withdraw the old one, or keep
the old one and reject this proposal. A resolution set that no longer matches
what is actually active is refused rather than partly applied, the same
revalidation rule ADR-021 applies to context packs.

The signal on each pair is `overlap` or `opposing_polarity`. Neither claims a
contradiction was detected — see the limits below.

## Where memory meets the agent

Approved memories enter the context pack as `Instructions`: a convention or a
caution is something to follow, not background reading. Each carries
`HUMAN approved by <name>` as provenance and `conversation:<id>:<message>` as
its source location, so an agent's context says who stands behind a rule and
where it came from.

Building a pack is not using a memory. A pack is built to be previewed and
rebuilt to be revalidated before anything reaches an agent, and counting those
would make the usage trail claim influence that was never had. The injection is
recorded at launch, from the pack that was actually sent, by
`memory.record_uses` reading `injections_from_pack`. A memory the human removed
from the preview is therefore never recorded as used.

## API

`memory.propose`, `memory.pending`, `memory.approve`, `memory.reject`,
`memory.list(filters)`, `memory.forRetrieval(context)`, `memory.usage(id)`, plus
`memory.revise`, `memory.revoke`, and `memory.history` which lifecycle point
four requires.

**Tauri commands.** `memory_propose`, `memory_pending`, `memory_approve`,
`memory_reject`, `memory_list`, `memory_for_retrieval`, `memory_usage`,
`memory_history`, `memory_revise`, `memory_revoke`. Every actor field is filled
from the local user and any value the client sent is discarded: an approval is
the entire authority behind a memory, so who gave it is not the webview's to
assert.

**Desktop activity.** `MemoryApproval.tsx` shows the queue with each proposal's
motivating excerpt and whether it was verified, an editable text area so
edit-then-accept is one action, and — when approval reports a conflict — the
explanation and the four choices, with no way past them. Active memories show
their approver and how many tasks they have shaped.

**CLI harness.** `mars-brain --repo <path> memory-pending|memory-list|memory-usage`.
Read-only on purpose: proposing needs a conversation to quote and approving
needs a person, and an approval typed by a script would be exactly the
rubber-stamp the design exists to prevent.

## Storage

Migration 14 adds `memories`, `memory_proposals`, `memory_uses`,
`memory_events`, and `memory_conflicts`. Constraints carry the invariants that
must not depend on application code: a scope must be resolvable, a revoked
record must name who revoked it and when, a resolved proposal must name who
resolved it, and one memory may supersede at most one other.

This is separate from `knowledge_records`, which is milestone D's projection of
one completed agent session onto reviewable claims about that task. Memory is
durable guidance that outlives the conversation it came from, anchored to a
conversation turn rather than to a session, and carrying a usage trail. The two
overlap in spirit and consolidating them is worth considering, but doing it as
part of this phase would have meant rewriting the CHECK constraints of a table
milestone D's code and tests depend on.

## Automated evidence

`crates/brain/tests/memory.rs` — 26 tests. Most exist to try to violate one of
the two rules.

- Proposal: a queued proposal is not a memory and is not retrievable; a
  proposal without its excerpt is refused; the same claim from the same turn is
  one proposal; an unresolvable scope is refused.
- Approval: approval records who approved and makes the memory retrievable;
  edit-then-accept stores the edit and says so; an unattributed approval is
  refused; a proposal cannot be approved twice; rejection writes no memory.
- Conflicts: a contradicting memory blocks approval and writes nothing until a
  human chooses; superseding links and retires the old one; keeping the existing
  memory rejects the proposal; a stale resolution set is refused; memories in
  different repositories never conflict.
- Retrieval and usage: selection is by scope and relevance and logs every
  injection; usage counts distinct tasks; a preview does not inflate the trail;
  a narrower scope outranks an equally similar broader one; revoked and
  superseded memories are never retrieved.
- Revision and audit: revising creates a linked new version; a superseded
  memory cannot be revised again; the audit trail records every transition with
  an actor and a conversation.
- Origin: an unverified origin is labelled rather than assumed good; a
  configured transcript source verifies a real excerpt and rejects an invented
  one.

`crates/brain/tests/memory_context.rs` — 6 tests on the agent boundary: an
approved memory reaches the agent as an instruction with its provenance and
source location; a pending proposal never reaches the agent; building and
revalidating a pack are not uses while launching is; a launch records only what
the sent pack carried; a revoked memory disappears from agent context; a caution
that reaches packing survives a tight section budget.

`crates/brain/tests/wire_contract.rs` — 4 tests pinning the exact JSON these
types produce. The TypeScript declarations are written by hand against that
shape and nothing at compile time connects the two, so a field that silently
changes case reaches the webview as `undefined` — a failure that survives a
passing test suite on both sides. These tests found exactly that bug in the
Phase 1 `BrainEvent`, whose struct-variant fields were serialising as
`stale_paths` and `latency_ms` while the TypeScript declared `stalePaths` and
`latencyMs`.

`apps/desktop/src/features/brain/MemoryApproval.test.tsx` — 5 tests: the queue
shows the motivating excerpt and says nothing there is in use; a memory reports
how many tasks it has shaped; accepting unchanged sends no edit and claims no
approver; editing first sends the edited text; a reported conflict removes the
accept button entirely and the chosen resolution is what gets sent.

## Limits

- **Conflict detection does not detect contradiction.** Deciding that two
  statements contradict each other is a judgement about meaning, and this layer
  has a feature-hashing embedder and a word list. What it does is narrow the
  field to pairs a person should look at and then require the person to choose.
  `opposing_polarity` means the two say similar things and one of them negates;
  that is a stronger hint, not a verdict. Two memories that genuinely conflict
  while sharing no vocabulary will not be flagged.
- Similarity rests on the Phase 0 local embedder, which captures identifier and
  path overlap rather than meaning, so both recall and precision here are
  bounded by ADR-029.
- **Retrieval ranks all kinds equally.** A caution competes with a convention on
  scope and relevance alone, and a caution beyond the retrieval limit is cut
  like anything else. Once a caution reaches packing it is protected from
  section-budget eviction, but that is a guarantee about the budget, not about
  retrieval. Whether safety-relevant memory should outrank relevance is a policy
  question this phase did not decide.
- **Origins are opaque and only optionally verified.** Mars has no conversation
  store, so `conversationId` and `messageId` are unconstrained strings. The
  supplied `AgentSessionTranscript` resolves them against agent sessions and
  events, which makes verification real for agent conversations; anything else
  is recorded unverified.
- The approval is written as a message into `memory_events` rather than into a
  conversation transcript, because there is no transcript to write into. When
  one exists, these entries carry the conversation and message ids needed to
  render them in place.
- Similarity is scanned in memory for every active memory in the project on each
  approval and each retrieval. That is adequate for the scale a single
  repository's approved memory reaches and will need an index well before it
  stops being so.
- Memory is scoped to a project. Cross-project global memory would need a
  storage owner above `projects`, which does not exist yet.
