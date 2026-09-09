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

## Known local failures

Some checks fail on Windows independently of any change; see
[recovery](./recovery.md) and the notes in `ARCHITECTURE.md` before treating one
as a regression. At the time of writing, `cargo test -p mars-skills --test
install` aborts with `STATUS_STACK_BUFFER_OVERRUN`, two `mars-core`
`open_repository` tests fail, and `pnpm lint` reports one parse error on
`fixtures/repositories/typescript-graph/src/broken.ts` — a deliberately
malformed indexing fixture.
