import type {
  BrainAnswer,
  BrainNeighborhood,
  BrainSymbol,
  DependencySlice,
  IndexStatus,
} from "@mars/shared-types";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => undefined) }));

import { CodeGraph, layoutSlice } from "./CodeGraph";

const index: IndexStatus = {
  repositoryId: "repo-1",
  rootPath: "/repo",
  branch: "main",
  headOid: "abc",
  indexedHeadOid: "abc",
  headMatchesIndex: true,
  embeddingModel: "mars-hashing-v1",
  embeddingDimensions: 256,
  counts: { files: 120, symbols: 900, edges: 2000, chunks: 1500, commits: 40, ignoredPaths: 3 },
  lastRun: null,
  runningRuns: 0,
};

const limiter: BrainSymbol = {
  id: "sym-limit",
  name: "limit",
  kind: "function",
  qualifiedName: "ingest::limit",
  relativePath: "src/ingest.ts",
  startLine: 12,
  endLine: 30,
  exported: true,
};

const slice: DependencySlice = {
  target: "sym-limit",
  targetKind: "symbol",
  nodes: [
    { id: "sym-limit", label: "limit", kind: "function", path: "src/ingest.ts", lineStart: 12, lineEnd: 30, distance: 0, resolved: true },
    { id: "sym-handler", label: "handleIngest", kind: "function", path: "src/server.ts", lineStart: 40, lineEnd: 70, distance: 1, resolved: true },
    { id: "sym-bucket", label: "TokenBucket", kind: "class", path: "src/bucket.ts", lineStart: 1, lineEnd: 50, distance: 1, resolved: true },
    { id: "pkg-redis", label: "redis", kind: "package", path: "", lineStart: 0, lineEnd: 0, distance: 1, resolved: false },
  ],
  edges: [
    { kind: "calls", source: "sym-handler", target: "sym-limit", confidence: 0.95, incoming: true },
    { kind: "calls", source: "sym-limit", target: "sym-bucket", confidence: 0.9, incoming: false },
    { kind: "imports", source: "sym-limit", target: "pkg-redis", confidence: 0.5, incoming: false },
  ],
  citations: [],
  gaps: [{ kind: "notIndexed", subject: "redis", detail: "a package outside the repository" }],
  truncated: false,
  latencyMs: 4,
};

const bucketSlice: DependencySlice = {
  ...slice,
  target: "sym-bucket",
  nodes: [{ ...slice.nodes[2], distance: 0 }],
  edges: [],
  gaps: [],
};

const reach: BrainNeighborhood = {
  root: limiter,
  symbols: [
    limiter,
    { ...limiter, id: "sym-handler", name: "handleIngest", qualifiedName: "server::handleIngest", relativePath: "src/server.ts" },
  ],
  edges: [],
};

const answer: BrainAnswer = {
  question: "What does limit do?",
  rewrittenQueries: [],
  claims: [],
  citations: [],
  sources: 0,
  retrieved: 0,
  latencyMs: 1,
  confidence: 0.4,
  gaps: [],
  sourceDetail: [],
  truncated: false,
};

beforeEach(() => {
  invoke.mockReset();
  invoke.mockImplementation((command: string, args: { target?: string }) => {
    switch (command) {
      case "find_symbols":
        return Promise.resolve([limiter]);
      case "brain_dependency_graph":
        return Promise.resolve(args.target === "sym-bucket" ? bucketSlice : slice);
      case "brain_blast_radius":
        return Promise.resolve(reach);
      case "brain_explain":
      case "brain_where_is":
        return Promise.resolve(answer);
      default:
        return Promise.reject(new Error(command));
    }
  });
});

describe("layoutSlice", () => {
  it("puts the target in the middle, users to the left and what it uses to the right", () => {
    const { center, placed } = layoutSlice(slice);
    expect(center).toBe("sym-limit");
    const x = (id: string) => placed.get(id)?.x ?? Number.NaN;
    expect(x("sym-handler")).toBeLessThan(x("sym-limit"));
    expect(x("sym-bucket")).toBeGreaterThan(x("sym-limit"));
    expect(x("pkg-redis")).toBe(x("sym-bucket"));
  });
});

describe("CodeGraph", () => {
  it("finds a symbol and draws what uses it and what it uses", async () => {
    const user = userEvent.setup();
    render(<CodeGraph index={index} onOpen={vi.fn()} repositoryId="repo-1" />);

    expect(screen.getByText("120 files · 900 symbols")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Find a symbol or file" }), "limit");
    const matches = await screen.findByRole("list", { name: "Matches" });
    await user.click(within(matches).getByRole("button", { name: /ingest::limit/ }));

    expect(await screen.findByRole("img", { name: "Dependencies of ingest::limit" })).toBeVisible();
    expect(invoke).toHaveBeenCalledWith("brain_dependency_graph", { repositoryId: "repo-1", target: "sym-limit" });
    const selected = screen.getByRole("complementary", { name: "Selected" });
    expect(within(selected).getByRole("region", { name: "Uses" })).toHaveTextContent("TokenBucket");
    expect(within(selected).getByRole("region", { name: "Used by" })).toHaveTextContent("handleIngest");
    expect(screen.getByText(/a package outside the repository/)).toBeVisible();
  });

  it("opens, re-centres, and walks back", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(<CodeGraph index={index} initialTarget="sym-limit" onOpen={onOpen} repositoryId="repo-1" />);

    await user.click(await screen.findByRole("button", { name: "TokenBucket (class)" }));
    const selected = screen.getByRole("complementary", { name: "Selected" });
    await user.click(within(selected).getByRole("button", { name: "Open" }));
    expect(onOpen).toHaveBeenCalledWith("src/bucket.ts", 1);

    await user.click(within(selected).getByRole("button", { name: "Put in the middle" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("brain_dependency_graph", { repositoryId: "repo-1", target: "sym-bucket" }),
    );
    expect(await screen.findByRole("img", { name: "Dependencies of TokenBucket" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(await screen.findByRole("img", { name: "Dependencies of limit" })).toBeVisible();
  });

  it("explains a symbol and says how far a change to it reaches", async () => {
    const user = userEvent.setup();
    render(<CodeGraph index={index} initialTarget="sym-limit" onOpen={vi.fn()} repositoryId="repo-1" />);
    const selected = await screen.findByRole("complementary", { name: "Selected" });
    await screen.findByRole("img", { name: "Dependencies of limit" });

    await user.click(within(selected).getByRole("button", { name: "What a change reaches" }));
    expect(await within(selected).findByText(/1 symbol\(s\) in 1 file\(s\) depend on limit/)).toBeVisible();

    await user.click(within(selected).getByRole("button", { name: "Explain" }));
    expect(invoke).toHaveBeenCalledWith("brain_explain", { repositoryId: "repo-1", symbolId: "sym-limit" });
  });

  it("does not offer to open or explain what is outside the repository", async () => {
    const user = userEvent.setup();
    render(<CodeGraph index={index} initialTarget="sym-limit" onOpen={vi.fn()} repositoryId="repo-1" />);
    await user.click(await screen.findByRole("button", { name: "redis (package)" }));

    const selected = screen.getByRole("complementary", { name: "Selected" });
    expect(within(selected).getByText("outside the repository")).toBeVisible();
    expect(within(selected).queryByRole("button", { name: "Open" })).toBeNull();
    expect(within(selected).queryByRole("button", { name: "Explain" })).toBeNull();
  });

  it("asks where a concept lives", async () => {
    const user = userEvent.setup();
    render(<CodeGraph index={index} onOpen={vi.fn()} repositoryId="repo-1" />);
    await user.type(screen.getByRole("textbox", { name: "Where is" }), "rate limiting");
    await user.click(screen.getByRole("button", { name: "Find" }));

    expect(invoke).toHaveBeenCalledWith("brain_where_is", { repositoryId: "repo-1", concept: "rate limiting" });
  });

  it("offers the open file as a place to start, and explains an empty index", async () => {
    const user = userEvent.setup();
    const { unmount } = render(
      <CodeGraph index={index} onOpen={vi.fn()} repositoryId="repo-1" suggestion="src/ingest.ts" />,
    );
    await user.click(screen.getByRole("button", { name: "Start from src/ingest.ts" }));
    expect(invoke).toHaveBeenCalledWith("brain_dependency_graph", { repositoryId: "repo-1", target: "src/ingest.ts" });
    unmount();

    const onReindex = vi.fn();
    render(
      <CodeGraph
        index={{ ...index, counts: { ...index.counts, files: 0 } }}
        onOpen={vi.fn()}
        onReindex={onReindex}
        repositoryId="repo-1"
      />,
    );
    expect(screen.getByText("The index is empty")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Index now" }));
    expect(onReindex).toHaveBeenCalled();
  });
});
