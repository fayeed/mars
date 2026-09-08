import type { DependencySlice, SearchResults } from "@mars/shared-types";
import { SearchIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { DerivedFrom, MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";
import type { WorkingTreeShas } from "@/shell/staleness";

export interface BrainExplorerProps {
  results: SearchResults | null;
  searchState: ActivityState;
  slice: DependencySlice | null;
  sliceState: ActivityState;
  error?: string | null;
  onSearch: (query: string) => void;
  onExplore: (target: string) => void;
  onOpen: (path: string, line: number) => void;
}

/**
 * Where a hit came from. Both retrievers finding the same span is a stronger
 * signal than either alone, and the row says which it was rather than hiding it
 * behind one number.
 */
const RETRIEVAL_LABEL: Record<string, string> = {
  lexical: "text match",
  semantic: "meaning match",
  hybrid: "text + meaning",
};

/**
 * The brain as a place to look things up, not to talk to.
 *
 * There is no chat here on purpose: this is the surface for wandering — search
 * the index, land on a symbol, walk what it depends on and what depends on it.
 * Conversing about the answer belongs in the conversation, where the answer
 * gets a citation and a record; this is for the step before that.
 */
export function BrainExplorer({
  error,
  onExplore,
  onOpen,
  onSearch,
  results,
  searchState,
  slice,
  sliceState,
}: BrainExplorerProps) {
  const [query, setQuery] = useState("");

  /**
   * What each hit's file holds now, straight out of the results.
   *
   * The core already compared every hit's path against the working tree on the
   * way out, so this costs no second read: the hit carries the sha its chunk
   * was indexed from, and this carries the sha the file has now.
   */
  const shas: WorkingTreeShas = new Map(
    (results?.freshness ?? []).map((entry) => [entry.relativePath, entry.currentBlobSha ?? null]),
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <form
        className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5"
        onSubmit={(event) => {
          event.preventDefault();
          onSearch(query);
        }}
      >
        <SearchIcon className="size-3.5 shrink-0 text-faint" />
        <input
          aria-label="Search the index"
          className="min-w-0 flex-1 bg-transparent text-[12px] text-foreground outline-none placeholder:text-faint"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search code by text or by meaning"
          value={query}
        />
        <button
          type="submit"
          disabled={query.trim().length === 0}
          className="border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          Search
        </button>
      </form>

      {error ? (
        <p className="mx-2 my-1 border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {searchState === "loading" ? (
          <StateBlock state="loading" title="Searching the index" />
        ) : !results ? (
          <StateBlock
            state="idle"
            title="Nothing searched yet"
            description="Results come from the local index. Anything not indexed cannot appear, however well it matches."
          />
        ) : results.hits.length === 0 ? (
          <StateBlock
            state="idle"
            title="No matches"
            description={`Nothing in the index matched "${results.query}".`}
          />
        ) : (
          <>
            <div className="flex items-baseline gap-2 border-b border-border px-3 pt-2 pb-1">
              <MetaLabel>Results</MetaLabel>
              <Mono className="text-faint">{results.hits.length}</Mono>
              {results.containsStale ? (
                <StateLabel state="stale" detail="some hits rest on files that changed" />
              ) : null}
            </div>
            <ul>
              {results.hits.map((hit) => {
                const anchor = { path: hit.relativePath, sha: hit.blobSha };
                return (
                  <li
                    key={hit.chunkId}
                    className="flex flex-col gap-1 border-b border-border px-3 py-2"
                  >
                    <div className="flex flex-wrap items-baseline gap-2">
                      <button
                        type="button"
                        onClick={() => onOpen(hit.relativePath, hit.startLine)}
                        className="min-w-0 flex-1 truncate text-left text-[12px] text-foreground outline-none hover:underline focus-visible:underline"
                      >
                        {hit.symbolPath || hit.relativePath}
                      </button>
                      <Mono className="shrink-0 text-faint">
                        {hit.relativePath}:{hit.startLine}
                      </Mono>
                    </div>
                    <pre className="max-h-28 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
                      {hit.body}
                    </pre>
                    <div className="flex flex-wrap items-baseline gap-2">
                      <Mono className="text-faint">
                        {RETRIEVAL_LABEL[hit.retrieval] ?? hit.retrieval}
                      </Mono>
                      {hit.language ? <Mono className="text-faint">{hit.language}</Mono> : null}
                      {/* The body above is what the index stored, not what the
                          file says now. The anchor is what makes that visible
                          rather than something the reader has to remember. */}
                      <DerivedFrom anchor={anchor} shas={shas} />
                      <button
                        type="button"
                        onClick={() => onExplore(hit.relativePath)}
                        className="ml-auto border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                      >
                        Show dependencies
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          </>
        )}

        {sliceState === "loading" ? (
          <div className="border-t border-border px-3 py-2">
            <StateLabel state="loading" detail="reading the graph" />
          </div>
        ) : slice ? (
          <section className="border-t border-border">
            <div className="flex flex-wrap items-baseline gap-2 px-3 pt-2 pb-1">
              <MetaLabel>Dependencies</MetaLabel>
              <Mono className="text-foreground">{slice.target}</Mono>
              <Mono className="text-faint">{slice.nodes.length} node(s)</Mono>
              {slice.truncated ? <Mono className="text-primary">truncated</Mono> : null}
            </div>

            <ul className="flex flex-col">
              {slice.edges.map((edge) => {
                const other = slice.nodes.find(
                  (node) => node.id === (edge.incoming ? edge.source : edge.target),
                );
                return (
                  <li
                    key={`${edge.kind}:${edge.source}:${edge.target}`}
                    className="flex items-baseline gap-2 border-b border-border px-3 py-1"
                  >
                    <Mono
                      className={cn(
                        "w-20 shrink-0",
                        edge.incoming ? "text-muted-foreground" : "text-foreground",
                      )}
                    >
                      {edge.incoming ? "used by" : edge.kind}
                    </Mono>
                    <button
                      type="button"
                      disabled={!other?.resolved}
                      onClick={() => other && onOpen(other.path, other.lineStart)}
                      className="min-w-0 flex-1 truncate text-left text-[11px] text-foreground outline-none enabled:hover:underline disabled:cursor-default disabled:text-faint"
                    >
                      {other?.label ?? (edge.incoming ? edge.source : edge.target)}
                    </button>
                    {other && !other.resolved ? (
                      <Mono className="shrink-0 text-faint">outside the repository</Mono>
                    ) : (
                      <Mono className="shrink-0 text-faint">
                        {Math.round(edge.confidence * 100)}%
                      </Mono>
                    )}
                  </li>
                );
              })}
            </ul>

            {slice.gaps.length > 0 ? (
              <div className="flex flex-col gap-0.5 px-3 py-2">
                <MetaLabel>What the graph could not resolve</MetaLabel>
                {slice.gaps.map((gap) => (
                  <span
                    className="text-[11px] text-muted-foreground"
                    key={`${gap.kind}:${gap.subject}`}
                  >
                    <Mono className="text-faint">{gap.kind}</Mono> {gap.subject} — {gap.detail}
                  </span>
                ))}
              </div>
            ) : null}
          </section>
        ) : null}
      </div>
    </div>
  );
}
