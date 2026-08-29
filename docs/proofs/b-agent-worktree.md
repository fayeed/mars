# Milestone B proof — task, worktree, and real agent

Date: 2026-08-29  
Branch: `feat/milestone-0`

## Scope proved

Milestone B establishes a real Tasks-first path that:

- plans a deterministic task branch/worktree without mutating Git;
- provisions it only after approval and journals safe rollback/reconciliation;
- probes installed Claude Code and Codex CLIs;
- shows explicit file, command, and provider-tool permissions;
- launches an adapter command only in a ready isolated worktree;
- streams provider-neutral checkpoint, text, tool, command, test, file, permission, unknown, and end events;
- persists redacted ordered process events and terminal session state;
- supports cancellation escalation, crash display, provider-session follow-up, and interrupted-session recovery;
- opens the task worktree as the active Explorer, Monaco, and terminal project.

## Toolchain exercised

- Git: system executable with argument arrays
- Claude Code: `2.1.247`
- Codex CLI: `0.150.0-alpha.12.2`
- Agent proof provider: authenticated Codex CLI
- UI: React 19, local shadcn/ui Base UI components, Tauri events

Authentication status was checked without persisting account identifiers in this proof.

## Real vertical evidence

The ignored opt-in test `task_worktree_real_agent_stream_and_reopen` ran against a disposable real Git repository. It used the same `TaskWorktreeService`, `CodexAdapter`, `ProcessSupervisor`, `JsonLineDecoder`, SQLite migrations, and system Git path used by the desktop backend.

The test:

1. trusted a newly initialized repository with one real commit;
2. created a `planned` task and asserted that neither branch nor worktree existed yet;
3. approved/provisioned the task and verified its checked-out branch;
4. launched Codex with `workspace-write` inside only that worktree;
5. requested one exact file and `git status --short` verification;
6. observed normalized provider checkpoint, command, file-change, and successful end events;
7. verified the persisted session reached `completed`;
8. reopened `mars-proof.txt` and verified the exact `MARS_AGENT_WORKTREE_OK\n` content and dirty Git status.

Final real proof command and result:

```text
cargo test -p mars-agents --test milestone_b_real task_worktree_real_agent_stream_and_reopen --offline -- --ignored --exact --nocapture

1 passed; finished in 94.42s
```

The smaller authenticated, read-only `real_codex_smoke` also passed in 4.71 seconds with no file changes or tool use.

The first real vertical attempt exposed current Codex CLI contract drift: file changes now arrive in `item.changes[]`, while the fixture used legacy top-level `path/change`. The adapter was corrected to accept both structures, the representative fixture was updated, and the strict file-event assertion passed on the final real run. This is evidence that unknown-tolerant parsing plus real opt-in checks are necessary.

## Automated evidence

The completed gate ran:

```text
cargo fmt --all -- --check
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
pnpm -r lint
pnpm -r typecheck
pnpm -r test
pnpm --filter @mars/desktop build
pnpm check:protocol
git diff --check
```

Before the final proof-only changes, 50 non-ignored Rust tests and 10 UI tests passed. The agent suites add three intentionally ignored paid/authenticated real tests: Claude exact-response smoke, Codex exact-response smoke, and the Codex task/worktree vertical proof. The final full gate is rerun after this proof document is added.

UI interaction tests cover task creation, the exact worktree approval plan, adapter probe failure, capability approval, launch arguments, incremental checkpoint/command rendering, cancellation, crash state, provider-session follow-up, and editor/terminal worktree handoff. Process tests separately exercise graceful cancellation with forced escalation and recovery of sessions abandoned by restart.

## Permissions exercised

- Repository trust: explicit for the disposable source repository.
- Git mutation: explicit worktree-plan approval in the product flow; direct approval represented by the provision call in the headless proof.
- Worktree file writes: allowed for the proof session through Codex `workspace-write`.
- Local commands: allowed; the observed proof command included `git status --short`.
- Provider tools/network: no web-search or arbitrary network tool was requested. Provider transport still requires the authenticated CLI to reach its model service.
- Commit, merge, push, force removal, forge writes: not granted and not performed.

## Recovery outcome

- Process completion is durable and queryable after the live event receiver closes.
- On application startup, sessions left in `running` or `cancelling` are marked `interrupted` and receive an ordered recovery event.
- Cancellation sends `SIGTERM` to the isolated process group and escalates to `SIGKILL` after the configured grace period; a dedicated process test proves the uncooperative-child path.
- Missing worktrees reconcile to `orphaned`; dirty worktrees and branches with unmerged commits are refused removal.

## Shortcuts and technical debt

- The graphical Tauri workflow was not visually exercised in this headless environment. The same command/event surface is covered by React interaction tests and the native vertical proof, but a human desktop smoke remains useful.
- Permission decisions are evaluated in memory; migration `0005` has the audit table, but the desktop launch path does not yet persist each decision row.
- Provider session checkpoints are held by the mounted task UI for follow-up. They are not yet copied into `agent_sessions`, so restart-time provider resume is not exposed.
- Durable native events store redacted raw process chunks. Normalized events are emitted to the UI but are not yet persisted as their own queryable records.
- The UI bounds rendering to the newest 500 normalized events rather than using variable-height window virtualization or replaying older durable history on demand.
- Codex exposes a coarse read-only/workspace-write sandbox boundary. Claude exposes allowed tools. Exact command patterns and remote hosts are not yet separately selectable in the launch dialog.
- The app does not yet summarize a task diff, checkpoint changes, or acquire an exclusive implementation lease. Those become inputs to Milestones D/E and must land before multi-agent handoff.
- The real proof used Codex only. Claude remains covered by representative stream fixtures, contract tests, installed-version probing, and its opt-in exact-response smoke.
