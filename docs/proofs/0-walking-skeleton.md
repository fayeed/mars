# Milestone 0 proof — walking skeleton and contracts

Date: 2026-08-28  
Branch: `feat/milestone-0`

## Scope proved

Milestone 0 establishes one buildable repository with:

- a Tauri 2 native entrypoint and React 19/Vite desktop shell;
- a task-centric dark UI using Tailwind CSS v4 and local shadcn/ui components;
- a thin `core_info` Tauri command backed by reusable `mars-core` logic;
- a versioned Rust/TypeScript command, response, error, and event protocol;
- bundled SQLite with transactional checksum-journaled migrations, WAL, foreign keys, and FTS5;
- persisted jobs/events, live cancellation tokens, terminal states, and interrupted-job recovery.

This milestone does not claim product proof A. It has no repository opening, editor, terminal, or structural index yet.

## Toolchain exercised

- Rust/Cargo: 1.96.1
- Node.js: 24.19.0
- pnpm: 10.23.0
- Tauri: 2.11.x
- React: 19.2.8
- Vite: 8.2.2
- rusqlite: 0.40.2 with bundled SQLite
- Tailwind CSS: 4.3.3
- shadcn: `base-nova`, Base UI primitives

## Automated evidence

The milestone gate passed:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
pnpm -r lint
pnpm -r typecheck
pnpm -r test
pnpm --filter @mars/desktop build
pnpm check:protocol
git diff --check
```

Rust workspace tests: 14 passed across core protocol, migration/settings, and job lifecycle suites. TypeScript protocol tests, app/protocol typechecks, ESLint, the production frontend build, and the schema/artifact version check passed.

The storage test creates and drops an FTS5 virtual table against the bundled library. Migration replay is tested after closing and reopening an on-disk temporary database. Recovery is likewise tested across a real close/reopen boundary.

## Runtime smoke evidence

- The Vite development server bound to `127.0.0.1:1420`; an HTTP request returned the Mars dark-shell document and module entrypoint.
- `cargo run -p mars-desktop --offline` compiled and launched the native binary. This container has no graphical display, so GTK initialization stopped before a window could open. The end-to-end visual `core_info` connected state therefore remains a manual desktop smoke check, not a claimed automated pass.

## What genuinely works

- Clean Rust and pnpm workspaces build from the tracked lockfiles.
- The native command accepts a versioned/correlated request and serializes the typed response from `mars-core`; incompatible versions return a structured protocol error.
- The frontend models connecting, connected, and disconnected states and builds with current shadcn sources.
- Rust and TypeScript agree on protocol 0.1.0 fixtures; events preserve unknown fields while commands/responses reject fields outside their versioned schema.
- A file-backed database migrates exactly once and rejects changed migration checksums by design.
- Job events receive monotonic per-job sequence numbers inside a transaction.
- Cancellation persists `cancel_requested` before signaling the live token.
- Reopening storage marks abandoned running/cancelling jobs `interrupted` without deleting evidence.

## Shortcuts and technical debt

- The desktop does not yet resolve/open its application-profile database; `Storage::open` takes an explicit path.
- Native window launch was not visually exercised because the execution environment has no GTK display.
- There are no React interaction tests yet; the shell is covered by typecheck, lint, and production build only.
- TypeScript protocol artifacts are hand-maintained and version-checked rather than generated from JSON Schema. Rust/TS fixture tests are the current drift defense.
- The JSON Schema is not executed by a general-purpose schema validator in tests; equivalent strict runtime decoders and Rust serde tests cover the current envelopes.
- `packages/ui` and `packages/shared-types` are placeholders. shadcn components remain app-local until reuse is demonstrated.
- Persisted jobs include a checkpoint column but checkpoint update/resume APIs wait for the first resumable indexer.
- Online database backup/restore is not implemented; see `docs/runbooks/local-data.md`.
