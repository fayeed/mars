# A.2 explorer, editor, and terminal proof

Status: passing

## User-visible slice

After trusting a repository, Mars replaces the landing canvas with a resizable IDE workspace built from shadcn/ui source components:

- Explorer lists repository files under Git, `.marsignore`, default build/dependency, and sensitive-file exclusions.
- Selecting a file opens a lazy-loaded Monaco model in a tab. Drafts retain dirty state across tabs; Save and Ctrl/Cmd+S use the version that was read from disk.
- A stale save shows a conflict alert and offers to reload the disk version rather than overwriting it.
- The bottom xterm panel remains dormant until “Start terminal” is clicked. It launches the user's default shell in the trusted repository, supports input and resize, streams raw PTY bytes, and exposes Stop.

Repository commands cross the webview boundary by stored repository ID. The native core resolves paths and never accepts a client-provided workspace root for file or terminal operations.

## Automated evidence

- Rust file integration tests cover paginated ignore-aware listing, sensitive/default exclusions, binary and 2 MiB size rejection, traversal and symlink escape rejection, versioned saves, external-edit conflicts, and successful replacement.
- Rust terminal tests cover canonical repository cwd, interactive input, PTY resize, cancellation, and bounded native scrollback.
- React editor tests cover dirty state, exact expected-version save payloads, successful version refresh, conflict messaging, and reload recovery.
- Workspace Rust formatting/clippy and tests, frontend lint/typecheck/tests, `git diff --check`, and the production Vite build pass together.

## Deliberate limits

- The native Tauri window cannot be visually driven in the headless environment because no graphical display is available. Domain behavior and webview boundaries are automated separately.
- Monaco and its language workers are local production assets. They are lazy-loaded after repository trust and file selection; the TypeScript worker remains intentionally large.
- File listing is snapshot pagination and the current Explorer eagerly requests all pages. Virtualized/incremental browsing is deferred until measured repository scale requires it.
- File versions detect ordinary external-edit conflicts, but no portable filesystem primitive can make comparison plus replacement indivisible from every external writer.
- Terminal cancellation currently closes the PTY process; the later permission/process layer will add graceful-interrupt timeout and explicit forced termination UX.
