# Phase 0 repository indexing proof

Status: passing

The indexing service is the substrate every later feature reads from. It watches
a Git working tree, maintains an incremental index that survives restarts, and
answers questions about that index with an explicit verdict on whether the answer
is still true of the tree.

## Layers

Each layer is independently queryable and records the Git blob SHA it was derived
from.

**Files.** Every non-ignored file in the working tree, with path, language, byte
size, mtime, Git blob SHA, and a binary marker. Files with no supported grammar
are still recorded; they simply contribute no symbols. Excluded entries are
recorded in `ignored_paths` with the rule that excluded them -- `gitignore`,
`marsignore`, `mars-default-directory`, or `sensitive-file` -- and a pruned
directory such as `node_modules/` is one row rather than one row per contained
file.

**Symbols.** Tree-sitter parses TypeScript, TSX, JavaScript, JSX, Python, Go,
Rust, JSON, YAML, and Markdown. Each definition carries its kind, name, qualified
name, byte range, line range, enclosing symbol, and leading documentation --
Python docstrings, Rust `///` comments, Go and JavaScript comment blocks. Export
visibility follows each language's own convention: capitalisation in Go, `pub` in
Rust, a leading underscore in Python, an `export` keyword in the JavaScript
family. JSON, YAML, and Markdown contribute a keyed property or heading tree so a
configuration key or prose section can be addressed like any other symbol.

**References and graph.** Import edges between files, plus call, reference,
inheritance, and implementation edges between symbols. Every edge stores its
resolver confidence and JSON evidence naming the file, span, parser, and grammar.
A `package.json` dependency section contributes `DEPENDS_ON` edges to the external
packages it names.

**Chunks and embeddings.** Chunking follows symbol boundaries: a definition is
never split mid-body, and only the material between definitions is split, on line
boundaries. A definition too large for one chunk is broken along its own nested
definitions rather than at an arbitrary offset. Each chunk carries a header
naming its file and symbol path, and is embedded by a deterministic local
feature-hashing model into a normalized 256-dimension vector stored in SQLite.
Chunk ids are content-addressed over repository, path, symbol path, and body, so
editing one function leaves every other chunk id in that file untouched.

**Git history.** Commits, per-file churn, last-touched-by, and blame ranges for
cited lines. History is read incrementally: a run reads only the commits added
since the last ingested head. Blame is computed against the working tree at query
time rather than cached, so a cited range is never attributed from a stale
snapshot.

## Storage and freshness

One bundled SQLite database per application profile, under the Mars data
directory and never inside the user's repository, with rows keyed by
`repository_id`. This follows ADR-002 rather than the one-database-per-repository
shape sketched in the phase brief; the invariant that matters -- index data never
lands in the user's tree -- holds either way. The schema is versioned with
forward-only migrations; this work added migration 13.

Every file, symbol, edge, and chunk row records the blob SHA it was derived from.
`freshness(path)` compares that against the file on disk and returns `fresh`,
`stale`, `missing`, or `unknown`, and `stale_paths()` reports every indexed path
that no longer matches. Search results carry a per-path freshness list and a
`containsStale` flag, so an answer resting on a changed file is labelled rather
than silently served.

## API

`index.status()`, `index.reindex(paths?)` (streaming `IndexEvent`s),
`search.lexical(query, filters)`, `search.semantic(query, k, filters)`,
`search.hybrid(...)` with reciprocal-rank fusion, `symbols.at(path, line)`,
`graph.neighbors(symbolId, {kinds, depth, direction})`,
`history.blame(path, range)`, plus `file_dependencies`, `ignore_status`,
`freshness`, and `stale_paths`.

Traversal is bounded: depth is clamped to 5 and results to 500, and a truncated
neighbourhood says so.

## Watching

A `notify` watcher debounces each path independently and flushes it once quiet,
so an editor's burst of writes costs one targeted re-index rather than one per
event. A change to `.git/HEAD` triggers a full pass instead, because a branch
switch rewrites arbitrarily many files and the event stream cannot be trusted to
have reported each one.

## Automated evidence

`fixtures/repositories/polyglot/` covers all ten languages, a binary file, a
`.gitignore` exclusion, a `.marsignore` exclusion, and a real Git repository with
commits. `fixtures/repositories/typescript-graph/` remains the structural fixture
from milestone A.3.

- `tests/languages.rs` -- per-language definitions, kinds, enclosing symbols,
  documentation, export visibility, import and manifest edges, and binary files
  joining the file layer without symbols.
- `tests/chunks_and_search.rs` -- chunk boundaries and headers, chunk id
  stability across re-indexing, embedding coverage and normalization, lexical,
  semantic, and hybrid retrieval, filters, stale labelling, and ignore reasons.
- `tests/graph_and_history.rs` -- who calls a symbol, what breaks if it changes,
  what a file depends on, traversal latency, blame and churn, incremental history,
  `symbols.at` ordering, and status reporting.
- `tests/deltas.rs` -- the three acceptance cases below, plus watcher debouncing.
- `tests/typescript_graph.rs` and `tests/interrupted_run.rs` -- the milestone A.3
  guarantees, still holding.

### Rename detection

Moving `src/engine.rs` to `src/retry.rs` is published as one rename, not a delete
plus an add. The file keeps its row id, its symbols keep their ids and stable
keys, `renamed_from` records where it came from, and no chunk header still names
the old path.

### Cold index timing

A generated 5,000-file TypeScript repository, release build, on the development
machine:

| Pass | Time |
| --- | --- |
| Cold index (5,000 files, 20,000 chunks) | 21.2 s |
| Unchanged pass | 505 ms |
| One-file targeted re-index | 90 ms |

Progress is reported per file as a streamable event. The test asserts a loose
120-second ceiling: it is a regression guard against an accidental quadratic, not
a benchmark of the host.

Indexing this repository cold takes 3.0 s for 266 files, 6,337 symbols, 13,994
edges, 4,450 chunks, and 44 commits.

### Branch-switch delta correctness

Switching to a branch that adds, rewrites, and deletes files, then switching back,
leaves the index matching the tree exactly: the branch-only file is retired, the
rewritten file's symbols are replaced, no symbol or chunk belongs to a retired
file, and `stale_paths()` is empty.

## CLI harness

`cargo run -p mars-indexer --bin mars-index -- --repo <path> <command>` drives the
same typed API, emitting JSON on stdout and progress on stderr: `status`,
`reindex`, `search lexical|semantic|hybrid`, `symbols-at`, `graph`, `depends`,
`blame`, `ignored`, `stale`, `files`, and `watch`. Its database defaults to the
Mars profile directory, overridable with `MARS_DATA_DIR` or `--database`.

## Precision limits

- Symbol resolution stays syntax-only. A target resolves only when its simple
  name is unique among active indexed symbols, so a name shared by, for example,
  a module and a function stays unresolved with its evidence and lower confidence
  retained. Member dispatch, overloads, aliases, re-exports, and module
  resolution are not compiler-precise.
- Semantic search captures identifier and path overlap, not meaning. It is a
  complement to lexical search, and hybrid fusion is the intended entry point.
- Similarity is scanned in memory per repository. This is adequate at the scale
  measured above and will need an approximate index well before it stops being so.
- The ignore set is refreshed by full runs. A targeted run applies the ignore
  rules to the paths it was given but does not restate the repository's exclusion
  set.
- A move combined with an edit is a delete plus an add, because the blob changed.
- Cold indexing is single-threaded. Parsing and embedding are the dominant cost
  and would parallelise, but the measured figures do not yet require it.
