# A.3 TypeScript/JavaScript structural index proof

Status: passing

## Indexed slice

`mars-indexer` walks a trusted repository with Git, `.marsignore`, dependency/build, sensitive-file, binary, and language filters. It hashes TS, TSX, JS, JSX, MJS, and CJS sources, skips unchanged content, and parses changed files with current tree-sitter grammars.

The published SQLite graph contains:

- functions, exported variables, classes, interfaces, methods, and test cases;
- stable keys, qualified names, exact byte/line spans, signatures, export state, and content hashes;
- import, call, reference, and inheritance edges;
- target IDs only when a symbol name resolves uniquely, otherwise an explicit target key;
- confidence plus JSON evidence containing source path/span and parser identity;
- parse-error counts while retaining valid symbols recoverable from incomplete source.

Each run parses into memory first. Changed-file replacements, deletions, edge reconciliation, and run completion publish in one SQLite transaction. Cancellation before publication records an interrupted run and exposes no partial file or symbol replacements.

## Automated evidence

The fixture at `fixtures/repositories/typescript-graph/` covers imports/exports, variables, functions, class inheritance, interfaces, methods, calls, references, tests, ignored paths, dependencies, malformed TypeScript, deletion, and rename.

Indexer integration tests prove:

- exact symbol kinds/spans/stable keys and export flags;
- resolved and unresolved edges with confidence and evidence;
- malformed-source recovery;
- ignore policy;
- unchanged-file skipping;
- changed-file replacement;
- deletion and rename retirement;
- interrupted-run isolation.

## Precision limits

- Resolution is syntax-only. A target becomes resolved only when its simple name is unique among active indexed symbols; ambiguous names remain unresolved.
- Member dispatch, overloads, aliases, re-exports, module resolution, lexical shadowing, and type-directed calls are not compiler-precise yet.
- Identifier references are conservative candidates and may include unresolved parameters or locals. Their evidence and lower initial confidence are retained.
- A rename is currently published as one retired path plus one new path. Git-aware identity reconciliation is deferred to the Git/index enrichment slice.
- Edge identity intentionally deduplicates repeated equivalent source/target relationships; evidence currently retains the latest matching span rather than every call site.
