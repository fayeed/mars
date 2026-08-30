# Security runbook

Status: Milestone F hardening

## Trust boundaries

Repository files, pasted context, skill instructions, agent/provider output, terminal output, issue text, and review comments are untrusted. Repository trust grants bounded read/index/Git observation only. Agent file writes, command execution, and network access are separate decisions recorded before launch.

Mars excludes VCS metadata, dependency/build directories, `.env` variants, credential files, private keys, and common certificate formats from default exploration/indexing. `.env.example` remains visible.

## Credential handling

- Forge tokens are stored in the native OS keychain. SQLite stores only an opaque credential reference.
- HTTP(S) Git push URLs containing user information are rejected; use the system credential helper, SSH agent, or keychain-backed provider authentication.
- Agent stdout/stderr and persisted forge errors redact common GitHub, OpenAI, AWS, bearer, assignment-style, and private-key credential patterns.
- Never paste a production token into task intent, context, issue/PR text, commit messages, or terminal commands.

Redaction is defense in depth, not a guarantee for every future secret format. If a secret appears in source or output, revoke/rotate it at the provider, remove it from the authoritative source and Git history as appropriate, then reindex. Do not rely on deleting a local audit row as incident response.

## Permission audit

Each launch capability decision is inserted into `permission_decisions` before the in-memory grant is applied. A storage failure therefore prevents launch authorization. Forge publication separately revalidates the exact plan, requires push and forge approvals, and creates a `forge_mutations` journal row before external mutation.

## Suspected compromise

1. Stop active agents and Mars.
2. Revoke affected provider/API credentials and invalidate active sessions.
3. Inspect the task worktree, Git reflog/remotes/hooks, permission decisions, agent events, and forge mutation journal.
4. Preserve a copy of the profile and repository for investigation before cleanup.
5. Rotate credentials through the provider and reconnect Mars; never write the replacement token to SQLite or project files.
