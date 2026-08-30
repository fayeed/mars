# Milestone F proof — GitHub issue to reviewed pull request

Date: 2026-08-30  
Branch: `feat/milestone-0`  
Status: deterministic production path passes; live external-write gate pending

## Scope implemented

- provider-neutral repositories, users, issues, pull requests, review comments, and check runs in `mars-forge`;
- GitHub HTTP normalization isolated in `mars-forge-github`, including current-user authentication, bounded pagination, HTTP error mapping, and retry recovery for an already-created matching PR;
- HTTPS, SCP-style SSH, and `ssh://` remote parsing, including ports/enterprise hosts and rejection of malformed or credential-bearing HTTPS remotes;
- native OS-keychain token storage through `keyring`; SQLite persists only the opaque credential reference and normalized connection metadata;
- migration 11 for remotes, connections, issues, issue-task links, PRs, comments, and checks with idempotent uniqueness constraints;
- explicit issue inspection before Issue → Task conversion;
- an exact PR plan containing fetch URL, actual Git `pushurl`, worktree, commit count, cleanliness, head SHA, head/base/title/body/draft, command preview, and SHA-256 plan identity;
- server-side plan reconstruction plus separate push and forge approvals before any external write;
- system-Git push to the exact reviewed URL, GitHub PR creation/recovery, durable restart-safe snapshots, foreground 30-second refresh, deep links, and review-comment → agent follow-up routing;
- durable pre-effect permission decisions and forge mutation journaling with failed/interrupted recovery, redacted diagnostics, and immediate persistence of confirmed PR identity;
- shadcn/ui issue-import, PR-preview, approvals, checks/comments, refresh, and overlap-warning surfaces.

## Deterministic evidence

`crates/forge/tests/workflow.rs` uses a real disposable Git repository and bare push remote. The fetch URL remains GitHub-shaped while Git has a distinct local `pushurl`, proving the safety-sensitive distinction. The test verifies:

1. issue import and task linkage use normalized provider objects;
2. the plan reports one commit and the exact push target;
3. refusing either approval leaves the remote branch absent;
4. accepting both approvals pushes exactly `HEAD:refs/heads/mars/fix-sync`;
5. the normalized PR, check, and review comment persist;
6. a subsequent foreground-style refresh reopens and updates the same durable PR.

The provider transport tests run without sockets and prove current-user/issue/PR/comment/check normalization, request routes and bodies, bounded page parameters, and 422 retry recovery by locating the exact existing head/base PR. Storage tests prove repeated issue/PR writes remain single records and that connection rows contain a keychain reference rather than a raw token. UI tests require issue inspection before task creation and both approvals before publish, then render normalized checks/comments.

The full repository gate passes:

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

Result: 90 non-ignored Rust tests, 20 desktop interaction tests, and all builds/protocol checks pass. Eight authenticated/network tests remain ignored by default.

## Live gate not claimed

This checkout has no Git remote, no `gh` executable, and no user-authorized disposable GitHub repository/issue/token target. Therefore no GitHub issue, branch, PR, check, comment, URL, or API scope is fabricated here, and no external write was attempted. The Milestone F gate remains open until the workflow is run against an explicitly supplied disposable repository.

## Known debt

- GitHub requests paginate but do not yet persist ETags, conditional-request cache state, rate-limit headers, or opaque raw payloads.
- The UI imports a known issue number; repository-wide issue browsing/filtering is not implemented.
- Polling is foreground-only at 30 seconds and stops with the component; no durable background forge job exists.
- Push and PR approvals are separately explicit and execute sequentially inside one native command; their decisions and mutation outcome are now durable, while a richer user-facing audit browser remains future work.
- If push succeeds and PR creation fails, the failed attempt remains visible in the durable journal; retrying a fresh plan safely re-pushes the same SHA and recovers a matching existing PR on GitHub 422. Other ambiguous network failures remain visible and require provider inspection or retry.
- GitHub Enterprise uses explicit GitHub connection onboarding for an arbitrary host; automatic enterprise-product discovery is not implemented.
- OS-keychain behavior is compiled through the platform-native backend but needs runtime proof in the final desktop/live GitHub gate.
