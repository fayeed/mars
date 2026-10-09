# Running and building Mars

Status: Phase 6

Mars is a Tauri 2 desktop app: a Rust binary that hosts a React frontend. Both
halves have to be built, so neither `vite` nor `cargo` on its own runs the
product. The two commands below are the ones to use.

## Prerequisites

- A Rust toolchain matching `rust-toolchain.toml` (stable MSVC on Windows).
- Node and pnpm as pinned by `packageManager` in the root `package.json`.
- `pnpm install` once, which brings the Tauri CLI in as a dev dependency of
  `apps/desktop` — there is nothing to install globally.

`pnpm --filter @mars/desktop exec tauri info` reports what the machine has and
is the first thing to run when a build fails for an unclear reason.

## Development

```sh
pnpm dev
```

Starts Vite on `http://localhost:1420`, compiles the Rust binary in debug, and
opens the app window. The frontend hot-reloads on save. Rust changes trigger a
recompile and restart the window, which takes as long as `cargo build` takes.

This runs the real core — real SQLite, real Git, real agent processes — against
the local application profile described in [local-data](./local-data.md). It is
not a mock harness, so anything it does to a repository it really does.

## Production build

```sh
pnpm build
```

Builds the frontend for production, compiles the Rust binary in release, and
produces installers under `target/release/bundle/`. `bundle.targets` in
`apps/desktop/src-tauri/tauri.conf.json` is `"all"`, so this builds every
installer format the host platform supports — on Windows that is MSI and NSIS,
and the first run downloads the WiX and NSIS toolchains.

The release binary itself lands at `target/release/mars-desktop` (`.exe` on
Windows) and can be run without the installers.

`bundle.icon` has to name the icon files explicitly. Tauri defaults it to an
empty list rather than to the conventional `icons/` layout, so leaving it out
fails the bundle with `Couldn't find a .ico icon` *after* a full release
compile — the icons are only read once the binary exists.

## The other scripts

| Command | What it does |
| --- | --- |
| `pnpm build:web` | The frontend alone, into `apps/desktop/dist/`. Useful for checking a bundling change without paying for a release compile. |
| `pnpm typecheck` | `tsc` across every workspace package. |
| `pnpm test` | The desktop test suite. Rust tests are `cargo test --workspace`. |
| `pnpm lint` | ESLint across the repository. |
| `pnpm check:protocol` | Confirms the Rust and TypeScript protocol artifacts agree. |

`apps/desktop` keeps `dev` and `build` as the plain Vite scripts, because
`tauri.conf.json` calls them as its `beforeDevCommand` and `beforeBuildCommand`.
Renaming them would break the Tauri build. The app-level entry points are
`app:dev` and `app:build`, which is what the root `dev` and `build` delegate to.

## Agent CLIs

Mars drives the agent CLIs a person already has — Claude Code and Codex — and
never bundles them. It looks for each in this order and Settings → Agents says
which one it found and how:

1. a path chosen in Settings (kept in `agent-executables.json` in the profile);
2. `PATH`, honouring `PATHEXT` and preferring a real `.exe` to a script shim;
3. the installers' own locations (`~/.local/bin`, `%APPDATA%\npm`, Homebrew,
   Cargo, Volta, Bun);
4. the copies editor extensions ship (VS Code, VS Code Insiders, Cursor,
   Windsurf).

Signing in is the CLI's own: Settings opens a terminal running its sign-in
command, and can tell an agent to use that sign-in even where an API key in
Mars's environment would outrank it.

Every run is given Mars as an MCP server, served by the app's own binary:

```sh
mars-desktop mcp --database <profile>/mars.sqlite3 --repository <id> --task <id> --run <id> --worktree <path> --role implementer
```

The same server is built standalone as `mars-mcp` and takes the same flags
(`cargo run -p mars-mcp -- --database … --repository …`), which is the quickest
way to try a tool by hand: it speaks newline-delimited JSON-RPC on stdin and
stdout. Only `--database` and `--repository` are required; without `--task` the
two writes (`record_decision`, `remember`) refuse, because both belong to a
task's conversation.

### Checking a real run

The automated suite drives fixture agents and replays recorded streams; it never
starts a real agent, because a real run spends the person's plan. Before a
release, check one by hand:

1. Open a small repository, let it index, and send a request in the composer.
2. Approve the plan and the context pack; the run should stream into the
   conversation and appear on the Agents board.
3. Ask the agent to find a symbol: the transcript should show
   `mcp__mars__find_symbol`, not a shell `grep`.
4. Ask it to write outside the task's scope: a permission request should appear
   in the conversation, and denying it should leave the run going.
5. Send a follow-up while it works: it should queue, then resume the same
   provider session when the run ends.
6. If it recorded a decision, Knowledge should count it on the rail.

The ignored real-CLI tests (`cargo test -p mars-agents -- --include-ignored`)
check only what is free — versions, sign-in status, and that Codex reads the
MCP overrides Mars builds; set `MARS_CLAUDE_EXECUTABLE` and
`MARS_CODEX_EXECUTABLE` when the CLIs are not on `PATH`.

## Known local failures

None at the time of writing: `cargo test --workspace`, `cargo clippy
--workspace --all-targets -- -D warnings`, `pnpm typecheck`, `pnpm lint` and
`pnpm test` all pass on Windows. Fixture repositories under
`fixtures/repositories/` are excluded from lint because one is malformed on
purpose. See [recovery](./recovery.md) before treating a new local failure as
a regression.
