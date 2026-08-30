# Recovery runbook

Status: Milestone F hardening

## Startup reconciliation

Mars applies append-only SQLite migrations before exposing commands. It then reconciles durable process sessions and forge mutations. Agent sessions abandoned in a live state become `interrupted`. Forge mutations abandoned in `started` become `interrupted` with a completion timestamp; Mars never promotes them to success merely because a branch or hosted object might exist.

Recovery does not delete source files, worktrees, branches, or remote objects.

## Interrupted agent session

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

The `forge_mutations` journal records `started`, `completed`, `failed`, and `interrupted` attempts with the reviewed plan fingerprint and approval flags. Error text is redacted before persistence.

## Database migration failure

Stop Mars and preserve the profile directory, including `mars.sqlite3`, `mars.sqlite3-wal`, `mars.sqlite3-shm`, and `artifacts/`. Do not edit an applied migration or manually advance `PRAGMA user_version`. Restore a coordinated backup or diagnose against a copy. Source repositories and worktrees are external and remain untouched.

See [local-data.md](local-data.md) for profile and backup details.
