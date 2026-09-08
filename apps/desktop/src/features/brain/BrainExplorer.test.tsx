import type { DependencySlice, SearchResults } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BrainExplorer } from "./BrainExplorer";

const results: SearchResults = {
  query: "rate limit",
  hits: [
    {
      chunkId: "chunk-1",
      relativePath: "crates/ingest/src/lib.rs",
      symbolPath: "Ingest::accept",
      header: "",
      body: "self.limiter.check(token)?;",
      language: "rust",
      startLine: 80,
      endLine: 92,
      blobSha: "abc",
      score: 0.9,
      retrieval: "hybrid",
      lexicalRank: 1,
      semanticRank: 2,
    },
  ],
  freshness: [
    {
      state: "stale",
      relativePath: "crates/ingest/src/lib.rs",
      indexedBlobSha: "abc",
      currentBlobSha: "def",
    },
  ],
  containsStale: true,
};

const slice: DependencySlice = {
  target: "crates/ingest/src/lib.rs",
  targetKind: "file",
  nodes: [
    {
      id: "node-1",
      label: "Limiter",
      kind: "symbol",
      path: "crates/ingest/src/limit.rs",
      lineStart: 3,
      lineEnd: 20,
      distance: 1,
      resolved: true,
    },
    {
      id: "node-2",
      label: "tokio",
      kind: "package",
      path: "",
      lineStart: 0,
      lineEnd: 0,
      distance: 1,
      resolved: false,
    },
  ],
  edges: [
    { kind: "imports", source: "root", target: "node-1", confidence: 0.95, incoming: false },
    { kind: "imports", source: "root", target: "node-2", confidence: 0.8, incoming: false },
  ],
  citations: [],
  gaps: [{ kind: "ambiguous", subject: "tokio", detail: "resolves to a package, not a file" }],
  truncated: false,
  latencyMs: 40,
};

function renderExplorer(props: Partial<Parameters<typeof BrainExplorer>[0]> = {}) {
  return render(
    <BrainExplorer
      onExplore={vi.fn()}
      onOpen={vi.fn()}
      onSearch={vi.fn()}
      results={results}
      searchState="idle"
      slice={null}
      sliceState="idle"
      {...props}
    />,
  );
}

describe("BrainExplorer search", () => {
  it("is a place to look things up, with no chat in sight", () => {
    renderExplorer();

    expect(screen.getByRole("textbox", { name: "Search the index" })).toBeVisible();
    expect(screen.queryByRole("textbox", { name: /message/i })).not.toBeInTheDocument();
  });

  it("shows the snippet and says which retriever found it", () => {
    renderExplorer();

    expect(screen.getByText("self.limiter.check(token)?;")).toBeVisible();
    expect(screen.getByText("text + meaning")).toBeVisible();
  });

  it("marks a hit whose file has changed since it was indexed", () => {
    renderExplorer();

    expect(screen.getByText("changed since indexed")).toBeVisible();
    expect(screen.getByText("some hits rest on files that changed")).toBeVisible();
  });

  it("opens a hit at its line", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    renderExplorer({ onOpen });

    await user.click(screen.getByRole("button", { name: "Ingest::accept" }));
    expect(onOpen).toHaveBeenCalledWith("crates/ingest/src/lib.rs", 80);
  });

  it("searches on submit", async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn();
    renderExplorer({ onSearch, results: null });

    await user.type(screen.getByRole("textbox", { name: "Search the index" }), "limiter");
    await user.click(screen.getByRole("button", { name: "Search" }));
    expect(onSearch).toHaveBeenCalledWith("limiter");
  });

  it("says an unindexed thing cannot appear, however well it matches", () => {
    renderExplorer({ results: null });

    expect(screen.getByText(/Anything not indexed cannot appear/)).toBeVisible();
  });
});

describe("BrainExplorer dependencies", () => {
  it("walks the edges over the index's own edge kinds", () => {
    renderExplorer({ slice });

    expect(screen.getByText("Limiter")).toBeVisible();
    expect(screen.getAllByText("imports")).toHaveLength(2);
    expect(screen.getByText("95%")).toBeVisible();
  });

  it("will not pretend a package outside the repository can be opened", () => {
    renderExplorer({ slice });

    expect(screen.getByText("outside the repository")).toBeVisible();
    expect(screen.getByRole("button", { name: "tokio" })).toBeDisabled();
  });

  it("says what the graph could not resolve", () => {
    renderExplorer({ slice });

    expect(screen.getByText("What the graph could not resolve")).toBeVisible();
    expect(screen.getByText(/resolves to a package, not a file/)).toBeVisible();
  });

  it("asks for a dependency slice from a hit", async () => {
    const user = userEvent.setup();
    const onExplore = vi.fn();
    renderExplorer({ onExplore });

    await user.click(screen.getByRole("button", { name: "Show dependencies" }));
    expect(onExplore).toHaveBeenCalledWith("crates/ingest/src/lib.rs");
  });
});
