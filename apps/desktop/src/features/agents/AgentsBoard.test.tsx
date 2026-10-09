import type { RunOverview } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AgentsBoard, formatDuration, lanesOf, runOverviewState } from "./AgentsBoard";

const NOW = Date.parse("2026-10-09T12:00:00Z");

function run(overrides: Partial<RunOverview>): RunOverview {
  return {
    id: "run-1",
    taskId: "task-a",
    sessionId: "s1",
    agentId: "claude",
    adapterKind: "claude",
    role: "implementer",
    capabilities: {},
    contextPackDigest: null,
    status: "completed",
    lastSequence: 0,
    error: null,
    startedAt: "2026-10-09T11:50:00Z",
    endedAt: "2026-10-09T11:55:00Z",
    providerSessionId: "p1",
    resumesRunId: null,
    usage: null,
    taskTitle: "Tune retries",
    branchName: "mars/tune-retries",
    worktreePath: "/w/tune",
    live: false,
    pendingPermissions: 0,
    filesTouched: 0,
    ...overrides,
  };
}

const runs: RunOverview[] = [
  run({ id: "a1" }),
  run({
    id: "a2",
    resumesRunId: "a1",
    startedAt: "2026-10-09T11:56:00Z",
    endedAt: null,
    status: "running",
    live: true,
    pendingPermissions: 1,
    usage: { inputTokens: 1500, outputTokens: 500, costUsd: 0.25 },
  }),
  run({
    id: "b1",
    taskId: "task-b",
    taskTitle: "Fix the parser",
    branchName: "mars/fix-parser",
    adapterKind: "codex",
    status: "running",
    live: true,
    endedAt: null,
    startedAt: "2026-10-09T11:58:00Z",
    filesTouched: 3,
  }),
  run({
    id: "c1",
    taskId: "task-c",
    taskTitle: "Review the parser",
    role: "reviewer",
    status: "failed",
    error: "rate limited",
    startedAt: "2026-10-09T11:40:00Z",
    endedAt: "2026-10-09T11:41:00Z",
  }),
];

describe("AgentsBoard", () => {
  it("reads each run in the shell's state vocabulary", () => {
    expect(runOverviewState(runs[0])).toBe("done");
    expect(runOverviewState(runs[1])).toBe("blocked");
    expect(runOverviewState(runs[2])).toBe("streaming");
    expect(runOverviewState(runs[3])).toBe("failed");
    // Recorded as running, but nothing is reading it.
    expect(runOverviewState(run({ status: "running", live: false }))).toBe("stale");
  });

  it("puts one lane per task, waiting before working before finished", () => {
    const lanes = lanesOf(runs);
    expect(lanes.map((lane) => lane.taskId)).toEqual(["task-a", "task-b", "task-c"]);
    expect(lanes[0].runs.map((candidate) => candidate.id)).toEqual(["a1", "a2"]);
  });

  it("says who is working, who is waiting, and what it cost", () => {
    render(
      <AgentsBoard
        now={NOW}
        onOpenTask={vi.fn()}
        onStop={vi.fn()}
        runs={runs}
        state="idle"
      />,
    );
    const summary = screen.getByRole("group", { name: "Summary" });
    expect(within(summary).getByText("1 working")).toBeInTheDocument();
    expect(within(summary).getByText("1 waiting on you")).toBeInTheDocument();
    expect(within(summary).getByText("2.0k tokens")).toBeInTheDocument();
    expect(within(summary).getByText("$0.25 reported")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Tune retries: 2 run(s)" })).toBeInTheDocument();
    expect(screen.getByText("failed: rate limited")).toBeInTheDocument();
  });

  it("lists the run waiting on you first and stops a live run by its id", async () => {
    const user = userEvent.setup();
    const onStop = vi.fn();
    const onOpenTask = vi.fn();
    render(
      <AgentsBoard
        now={NOW}
        onOpenTask={onOpenTask}
        onStop={onStop}
        runs={runs}
        state="idle"
      />,
    );
    const list = screen.getByRole("region", { name: "Runs" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows[0]).toHaveAttribute("data-run", "a2");
    expect(within(rows[0]).getByText("waiting on you")).toBeInTheDocument();

    await user.click(within(rows[0]).getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalledWith("a2");
    // Finished runs offer no Stop.
    expect(within(rows[3]).queryByRole("button", { name: "Stop" })).not.toBeInTheDocument();

    await user.click(within(rows[1]).getByRole("button", { name: "Fix the parser" }));
    expect(onOpenTask).toHaveBeenCalledWith("task-b");
  });

  it("says plainly when nothing has run", () => {
    render(<AgentsBoard now={NOW} onOpenTask={vi.fn()} onStop={vi.fn()} runs={[]} state="idle" />);
    expect(screen.getByText("No agent has run here yet")).toBeInTheDocument();
  });

  it("formats durations a person reads at a glance", () => {
    expect(formatDuration(42_000)).toBe("42s");
    expect(formatDuration(125_000)).toBe("2m 5s");
    expect(formatDuration(3_720_000)).toBe("1h 2m");
  });
});
