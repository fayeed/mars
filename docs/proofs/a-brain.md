# Milestone A proof — repository to Project Brain

Date: 2026-08-29
Branch: `feat/milestone-0`

## Scope proved

Milestone A implements the first full product slice:

1. inspect and explicitly trust a canonical real Git repository;
2. browse repository-relative files and edit text through versioned atomic saves;
3. run a native PTY rooted in that repository and observe the edit with `git diff`;
4. incrementally index TypeScript/JavaScript structure into SQLite;
5. search Project Brain, inspect evidence-backed relationships, and navigate the result to its exact Monaco source line.

The desktop uses app-local shadcn/ui primitives for the trust dialog, resizable workspace, tabs, search input, result controls, alerts, and status badges. Monaco and xterm are lazy-loaded behind the native Rust command boundary.

## End-to-end automated workflow

`crates/brain/tests/milestone_a.rs` creates a separate temporary repository with the system Git executable and a tracked TypeScript file. In one test it:

- trusts and persists the repository;
- finds and reads `src/main.ts` through `FileService`;
- changes `Hello` to `Welcome` through optimistic, atomic `write_file`;
- launches the native PTY in the repository and waits for the added line from `git --no-pager diff`;
- indexes the edited source;
- finds `greet`, traverses to its caller `run`, and reopens the evidence path;
- verifies the indexed 1-based line points at the definition in the reopened source.

Observed warm local timings from the proof run were 543 ms for PTY startup plus diff output, 1 ms for indexing the one-file repository, and below 1 ms for symbol search plus a depth-two neighborhood query. These are development-machine smoke timings, not benchmarks or product latency guarantees.

The React interaction test independently proves the Tauri command and UI sequence for index → search → inspect → open definition. Domain tests cover trust, worktree metadata discovery, ignore/security rules, save conflicts, PTY resize/cancellation/scrollback, incremental index publication, interrupted-run isolation, query direction, traversal bounds, and source evidence.

## Gate results

Tested toolchain:

- Rust/Cargo 1.96.1
- Node.js 24.19.0
- pnpm 10.23.0
- Git 2.43.0

The milestone gate passes:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
pnpm -r lint
pnpm -r typecheck
pnpm -r test
pnpm --filter @mars/desktop build
git diff --check
```

The final suite contains 34 passing Rust tests, including the real-repository workflow, and 7 passing desktop interaction tests. The protocol package test, all TypeScript typechecks, ESLint, clippy, formatting, and the production Vite build pass.

## Precision and performance limits

- Tree-sitter resolution remains syntax-only. Ambiguous names stay unresolved; member dispatch, overloads, aliases, re-exports, module resolution, lexical shadowing, and type-directed calls are not compiler-precise.
- Reference extraction is conservative and can include unresolved parameters or locals. The UI exposes confidence and evidence instead of implying certainty.
- Structural traversal is breadth-first with confidence-ordered adjacency, hard-limited to 100 results and depth 5. Semantic relevance and Git-recency ranking are deferred.
- Index publication is transactional and incremental, but the desktop currently runs indexing as a synchronous native command without progress events or a user-facing cancellation control.
- Monaco language workers create large production chunks; route/editor lazy loading keeps them off the initial shell, but worker packaging and cache strategy still need performance measurement on large repositories.

## Manual boundary and debt

This environment has no graphical display, so the native GTK window could not be visually driven end to end. The same workflow is covered across a real Git/PTY/filesystem/SQLite integration test and a React command/navigation test, but a packaged desktop smoke on Linux, macOS, and Windows remains a release prerequisite.

Additional debt:

- Explorer listing is paginated in the service but the current UI loads only its first page.
- The Brain inspector renders related-code edges; dedicated references, callers, callees, dependencies, and blast-radius views are still presentation work over implemented commands.
- Search is SQL substring matching with exact-name-first ordering; FTS, ranking explanations, and large-repository query telemetry arrive in retrieval slices.
- Index work needs durable job orchestration, progress events, reconnect behavior, and cancellation UI to fully satisfy ADR-014.
- No automated visual regression, keyboard-navigation browser suite, or screen-reader pass exists yet.
