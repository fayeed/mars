# Phase 3 proof — foundations: agents, worktrees, permissions, activity record

Date: 2026-09-08
Status: implemented in the Rust core; desktop surfaces still to be wired.

Phase 3 builds the execution core as four services plus crash recovery and an
inspectable context pack. Everything below is exercised by tests against real
Git repositories, a real SQLite database, and real child processes; nothing in
this phase is proven by a fake at its own boundary.

## What was built

| Service | Where | Tests |
| --- | --- | --- |
| Activity record (event log) | `crates/activity` | `activity_record.rs` (7), `recovery.rs` (5) |
| Permission broker | `crates/security/src/{broker,classify}.rs` | `permission_broker.rs` (14), `classification.rs` (8) |
| Worktree plans | `crates/git/src/{plan,disk}.rs` | `worktree_plans.rs` (15) |
| Agent interface | `crates/agents/src/agent.rs` | `agent_interface.rs` (8) |
| Context pack builder | `crates/brain/src/run_pack.rs` | `run_context_pack.rs` (6) |

Schema: migration `0015_execution_core.sql` adds `activity_events`,
`activity_payloads`, `agent_runs`, `permission_requests`, `permission_grants`,
`worktree_plans`, and `worktree_cleanups`.

TypeScript boundary types: `packages/shared-types/src/{activity,permissions,
worktree-plan,agent-runtime}.ts`.

## The properties that carry the design

**The log is the source of truth, and it is append-only by construction.**
`activity_events` has a `BEFORE UPDATE` trigger that aborts, and a
`BEFORE DELETE` trigger that aborts unless the parent task is already gone. The
test writes an event, then attempts a direct `UPDATE` and `DELETE` through the
same connection the service uses; both fail, and the event still reads back
unchanged. Sequences are assigned inside the insert transaction, so two tasks
interleaving still each see `1, 2, 3`.

**Replay, reload, crash recovery and export are one fold.** `replay()` produces
a `TaskProjection` from events alone — never from a status column. The same fold
runs on a completed task (runs, files, commands, tests, decided permissions) and
on an interrupted one, which is why the crash-recovery dialog and the timeline
cannot disagree.

**A reviewer cannot be granted a write.** `AgentRole::Reviewer` constructs a
`CapabilityCeiling` with `write_files: false`, and that ceiling is passed to the
broker with every request. The broker refuses such an effect outright: no
pending row is created, no ticket is handed out, and nothing appears for a human
to say yes to. The refusal is still recorded, as the *system's* decision with
`byConstruction: true`. The ceiling also reaches the command line — the launch
request is built with `allow_write: false` — so the guarantee does not depend on
the broker alone.

**A blocked agent stops.** A permission request leaves a `pending` row and an
unresolved `PermissionTicket`. The test asserts that no further events arrive
for 300ms while the question is unanswered, that `status()` reports `blocked`
naming the request, and that the stream resumes only after `decide()`. A denial
ends the run: the stream carries an `error` event saying why, the process is
cancelled, and the run settles to `cancelled` *before* `done` is emitted, so a
consumer reacting to the ending never reads a stale status.

**A plan is what was approved, not what is true now.** The plan pins the base
commit and is fingerprinted over what it *decides* — base, branch, path, scope,
setup commands. Free space and preflight findings are readings of a moving
world, so they are deliberately outside the digest; they are re-checked
separately at approval and at provisioning. When the base moves, the plan does
not silently follow it: a blocking `base_moved` finding appears naming both
commits, and provisioning refuses until a person picks rebase, recreate, or
continue-and-diverge — each with its consequence spelled out. Choosing to
diverge is recorded, so the same question is not asked again.

**Cleanup is reversible where it can be, and refuses where it cannot.**
`plan_cleanup` reports the uncommitted files that removal would discard and
whether the operation is reversible. The branch tip is archived to
`refs/mars/archive/<branch>` *before* the worktree goes away, so a completed
cleanup can be restored to the same path at the same commit. Uncommitted work is
never discarded without an explicit `discard_uncommitted` flag, and a stale
cleanup preview is refused by fingerprint.

**The pack is the prompt.** `RunContextPack.prompt` is the exact string the
agent is launched with, and `digest` is the SHA-256 of that string. A reviewer's
pack is filtered by the same ceiling that would refuse the tool later, so a
preview cannot promise something the broker would deny. `revalidate()` rebuilds
and compares digests, refusing a pack that changed after approval.

## Command classification

Classification fails towards the dangerous reading: an unrecognised program is
`mutating`, never `read_only`; a chain takes the class of its most consequential
link; and a redirect makes an otherwise harmless command mutating. Quoted
operators do not split a command (`grep 'a && b' file` stays read-only).

Two gaps were found and fixed while testing: `npm ci` was not recognised as a
network install, and `rm -rf build` reports "delete a directory tree" rather
than the generic reason.

## Checks

```text
cargo fmt --all --check                                   clean
cargo clippy --workspace --exclude mars-desktop --all-targets -- -D warnings   clean
cargo test --workspace --exclude mars-desktop --no-fail-fast
    228 passed, 2 failed, 1 ignored
pnpm -r lint / typecheck / test                           clean (29 UI tests)
```

Rust 1.96.1, edition 2024. Windows 11, MSVC toolchain.

## Pre-existing failures, not introduced by this phase

Verified against a clean tree (`git stash`) before touching them:

- `mars-core` `open_repository`: `non_git_directories_are_rejected` and
  `malformed_git_files_are_rejected` fail on Windows.
- `mars-desktop` does not build on Windows: `tauri-build` requires
  `apps/desktop/src-tauri/icons/icon.ico`, and only `icon.png` is in the repo.
  This crate is excluded from the commands above.

One pre-existing Windows failure *was* fixed, because Phase 3 depends on the
code it covered: `mars-security`'s `child_requests_can_narrow_but_never_widen_
the_parent_grant` used POSIX-absolute path literals, which are not absolute on
Windows, so scope containment silently returned false. The fixtures are now
rooted per platform. `crates/git/tests/fixtures.rs` also had its field order
corrected so the database closes before its directory is removed, and its
cleanup made best-effort so a failure there cannot turn an assertion failure
into a process abort.

## Debt and what is deliberately not here

- **Desktop surfaces are not wired.** The Rust services and the TypeScript
  types exist; `AgentTimeline`, `PermissionPrompt`, and the task page still read
  the Milestone B shapes. Rendering the UI from `TaskProjection` is the next
  step, and is what makes "the request appears in the conversation, not a
  modal" true in the product rather than only in the core.
- **No Tauri commands yet** for the new services, so nothing is reachable from
  the desktop process.
- **`respond_to_permission` is reachable on any `CliAgent`**, not only the one
  whose run is blocked; the broker enforces one-decision-per-request, so this is
  a tidiness issue rather than a soundness one.
- **Setup commands run under plan approval**, not through the broker. Approving
  a plan that lists `npm ci` is what authorizes it; a destructive setup command
  is flagged in preflight but still runs if approved.
- **Network egress is classified but not intercepted.** `SideEffect::Network`
  exists and is brokered when an adapter reports it; Mars does not itself see
  the agent's sockets.
- **Recovery actions are recorded, not carried out.** `record_choice` writes the
  human's decision to the log; performing resume/discard/rerun is the caller's,
  and is not yet wired to the worktree planner.
- **Payload garbage collection.** `activity_payloads` rows and their objects are
  never reclaimed; a long-lived task grows without bound.
