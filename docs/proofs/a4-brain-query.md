# A.4 Project Brain query proof

Status: passing

## Query surface

`mars-brain` exposes typed, repository-scoped structural queries over the published SQLite graph:

- symbol search with exact-name-first ordering;
- references, callers, callees, and file dependencies;
- related-code neighborhoods and reverse-only blast radius traversal;
- source spans, parser provenance, and confidence on every returned relationship.

All list queries require an explicit limit from 1 through 100. Graph traversals additionally require a depth from 1 through 5, use indexed adjacency lookups, stop at both bounds, and reject invalid bounds before querying.

## Desktop proof

The repository workspace includes a shadcn/ui Brain tab beside Explorer. A trusted project can be indexed from the tab, searched by symbol, and inspected as an evidence-backed relationship neighborhood. Opening a definition or relationship evidence loads its repository-relative source file in Monaco and reveals the indexed line.

The UI test proves the command sequence for indexing, searching, inspecting a neighborhood, and navigating to the selected definition. The native command boundary exposes all seven structural tools with structured errors.

## Automated evidence

`crates/brain/tests/structural_queries.rs` indexes the TypeScript graph fixture and verifies:

- exact symbol paths and source lines;
- reference, caller, and callee direction;
- dependency specifiers;
- related-code and blast-radius traversal;
- evidence paths on returned edges;
- rejection of zero-depth and over-depth traversal.

## Current limits

- Results inherit the syntax-only resolution and precision limits recorded in the A.3 proof.
- Neighborhood ranking is deterministic breadth-first adjacency with confidence ordering; semantic relevance and recency ranking are deferred.
- Indexing currently runs as one native command. Durable background execution, progress events, and cancellation controls arrive with the jobs/agent slices.
- The inspector visualizes the related-code neighborhood; dedicated callers, callees, references, dependencies, and blast-radius views can build on the already exposed command surface.
