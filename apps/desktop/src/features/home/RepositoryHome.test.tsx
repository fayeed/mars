import type { IndexStatus, StartingPoint } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { RepositoryHome, indexActivityState } from "./RepositoryHome";

const status: IndexStatus = {
  repositoryId: "repository-1",
  rootPath: "/work/mars",
  branch: "main",
  headOid: "511c628",
  indexedHeadOid: "511c628",
  headMatchesIndex: true,
  embeddingModel: "local",
  embeddingDimensions: 384,
  counts: { files: 412, symbols: 5120, edges: 900, chunks: 2400, commits: 300, ignoredPaths: 12 },
  lastRun: null,
  runningRuns: 0,
};

const points: StartingPoint[] = [
  {
    id: "working-tree",
    title: "Pick up the change in your working tree",
    reason: "3 file(s) are modified and uncommitted.",
    source: "git status",
    prompt: "Explain what is changing in a.rs, b.rs and what is left to finish.",
  },
];

function renderHome(props: Partial<Parameters<typeof RepositoryHome>[0]> = {}) {
  return render(
    <RepositoryHome
      index={status}
      indexState="idle"
      onIndex={vi.fn()}
      onSelectTask={vi.fn()}
      onUseStartingPoint={vi.fn()}
      repositoryName="mars"
      startingPoints={points}
      startingPointsState="idle"
      tasks={[]}
      {...props}
    />,
  );
}

describe("indexActivityState", () => {
  it("calls an unread index idle, not done", () => {
    expect(indexActivityState({ ...status, counts: { ...status.counts, files: 0 } })).toBe("idle");
  });

  it("calls a moved-on index stale, not failed", () => {
    expect(indexActivityState({ ...status, headMatchesIndex: false })).toBe("stale");
  });

  it("reports a run in progress as loading", () => {
    expect(indexActivityState({ ...status, runningRuns: 1 })).toBe("loading");
  });
});

describe("RepositoryHome", () => {
  it("opens on a conversation, not a file picker", () => {
    renderHome();

    expect(screen.getByText("Ask for a change, or pick up where you left off.")).toBeVisible();
    expect(screen.queryByText(/select a file/i)).not.toBeInTheDocument();
  });

  it("states what the index knows", () => {
    renderHome();

    expect(screen.getByText("412 files · 5,120 symbols")).toBeVisible();
    expect(screen.getByText("main")).toBeVisible();
  });

  it("says a stale index is still answering, from what it read", () => {
    renderHome({ index: { ...status, headMatchesIndex: false } });

    expect(screen.getByText(/Answers will be drawn from what was read then/)).toBeVisible();
  });

  it("offers to index when nothing has been read", () => {
    renderHome({ index: { ...status, counts: { ...status.counts, files: 0 } } });

    expect(screen.getByRole("button", { name: /Index now/ })).toBeVisible();
    expect(screen.getByText(/Nothing has been read yet/)).toBeVisible();
  });

  it("shows each starting point with the evidence behind it", () => {
    renderHome();

    expect(screen.getByText("Pick up the change in your working tree")).toBeVisible();
    expect(screen.getByText("3 file(s) are modified and uncommitted.")).toBeVisible();
    expect(screen.getByText("git status")).toBeVisible();
  });

  it("puts a starting point in the composer rather than sending it", async () => {
    const user = userEvent.setup();
    const onUseStartingPoint = vi.fn();
    renderHome({ onUseStartingPoint });

    await user.click(screen.getByRole("button", { name: /Pick up the change/ }));
    expect(onUseStartingPoint).toHaveBeenCalledWith(points[0].prompt);
  });

  it("selects a recent task", async () => {
    const user = userEvent.setup();
    const onSelectTask = vi.fn();
    renderHome({
      onSelectTask,
      tasks: [{ id: "task-1", title: "Rate limit ingest", state: "idle", branch: "mars/rate" }],
    });

    await user.click(screen.getByRole("button", { name: /Rate limit ingest/ }));
    expect(onSelectTask).toHaveBeenCalledWith("task-1");
  });
});
