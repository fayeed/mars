# Recovery runbook

Status: Milestone F hardening; Phase 3 execution core

## Startup reconciliation

Mars applies append-only SQLite migrations before exposing commands. It then reconciles durable process sessions and forge mutations. Agent sessions abandoned in a live state become `interrupted`. Forge mutations abandoned in `started` become `interrupted` with a completion timestamp; Mars never promotes them to success merely because a branch or hosted object might exist.

Recovery does not delete source files, worktrees, branches, or remote objects.

## Interrupted agent run (Phase 3)

An agent run that was not terminal when Mars stopped is a recovery candidate.
`RecoveryService::detect()` runs at startup and is read-only: it changes no
status, writes nothing to the log, and starts no process. Detection is
repeatable, so it can be run again while deciding.

For each interrupted run it reconstructs, from the activity record alone:

- **What was in flight** — commands that started and never reported an outcome,
  and permission requests that were never answered.
- **What was written to disk** — every `file_edit` the run recorded, each
  checked against the worktree. A file the log claims was written but which is
  absent is reported as absent rather than assumed.
- **What is unverified** — operations without outcomes, files written after the
  last recorded test run, and the run's own missing completion.

Three choices are offered, each stating its consequence:

| Choice | What it does |
| --- | --- |
| Resume | New run against the files already on disk. Uncommitted changes from the interrupted run stay and become part of the new run's work. |
| Discard | Marks the run abandoned. Changes nothing on disk; the worktree keeps whatever was written. |
| Rerun | Discards every uncommitted change, returns the worktree to its branch tip, and starts again. Cannot be undone. |

Nothing auto-resumes. `mark_interrupted` is the only status change recovery
makes on its own, and it moves the run further from running, never closer; it
appends a `recovery_noted` event carrying the full report. The human's choice is
written to the log before it is carried out, so the transcript says who decided
what.

## Interrupted agent session (process level)

1. Open the task and inspect the retained normalized event timeline and worktree diff.
2. Confirm the prior process is not still running outside Mars.
3. Resume with the provider checkpoint when available, or launch a new session against the same reviewed context pack.
4. Re-run relevant tests before checkpointing or publishing.

## Interrupted or failed pull-request publish

1. Inspect the task worktree, exact head SHA, branch, and configured fetch/push URLs.
2. Refresh the task's stored PR. If a confirmed provider identity was received before comments/checks failed, the minimal PR snapshot is already durable and refresh completes it.
3. If no PR is stored, preview the publish plan again. Any changed commit, Git configuration, or PR field invalidates the earlier approval.
4. Approve push and forge write separately. Re-pushing the same SHA is safe; GitHub `422` recovery accepts only an open PR matching the exact head and base.
5. If the provider result remains ambiguous, inspect GitHub directly before retrying. Do not infer PR success from a successful branch push.

The task's **Publish history** surface shows the latest bounded `forge_mutations` records: `started`, `completed`, `failed`, and `interrupted` attempts with the reviewed plan fingerprint and approval flags. It can be refreshed without repeating a mutation. Error text is redacted before persistence.

## Pending permission requests after a restart

A request left `pending` in `permission_requests` survives the restart, but the
agent that was waiting on it does not — its ticket died with the process. Decide
or abandon the request before starting a new run for the task, so the new run
does not inherit a grant nobody is expecting. Grants themselves are durable and
keep their expiry; revoke from Settings rather than deleting rows, so the
transcript keeps saying the grant existed.

## Worktree cleanup and restore

A completed cleanup archives the branch tip at `refs/mars/archive/<branch>`
before removing anything. To undo one, use the cleanup record's `restore`: it
recreates the worktree at the same path from the archived commit. Uncommitted
changes are never recoverable this way — the cleanup preview named them
individually before the removal, and discarding them required an explicit
choice.

## Database migration failure

Stop Mars and preserve the profile directory, including `mars.sqlite3`, `mars.sqlite3-wal`, `mars.sqlite3-shm`, and `artifacts/`. Do not edit an applied migration or manually advance `PRAGMA user_version`. Restore a coordinated backup or diagnose against a copy. Source repositories and worktrees are external and remain untouched.

See [local-data.md](local-data.md) for profile and backup details.
