import type { InterruptedRun } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { InterruptedRunRecovery } from "./InterruptedRunRecovery";

const run: InterruptedRun = {
  runId: "run-abcdef123456",
  taskId: "task-1",
  agentId: "claude",
  adapterKind: "claude",
  role: "implementer",
  statusWhenStopped: "running",
  startedAt: "2026-09-08T12:00:00Z",
  lastEventAt: null,
  lastSequence: 42,
  inFlight: [
    {
      kind: "command",
      summary: "cargo test -p mars-ingest",
      sequence: 41,
      startedAt: "2026-09-08T12:04:00Z",
    },
  ],
  filesWritten: [
    { path: "crates/ingest/src/lib.rs", change: "modified", edits: 3, lastSequence: 40, presentOnDisk: true },
    { path: "crates/ingest/src/gone.rs", change: "created", edits: 1, lastSequence: 39, presentOnDisk: false },
    { path: "crates/ingest/src/x.rs", change: "modified", edits: 1, lastSequence: 38, presentOnDisk: null },
  ],
  unverified: [
    "1 operation(s) started and never reported an outcome",
    "no test run was recorded for this work",
  ],
  options: [
    {
      action: "resume",
      label: "Resume in the same worktree",
      consequence: "Starts a new run against the files already on disk.",
    },
    {
      action: "discard",
      label: "Discard the run, keep the files",
      consequence: "Marks the run abandoned and changes nothing on disk.",
    },
    {
      action: "rerun",
      label: "Reset the worktree and run again",
      consequence: "Discards every uncommitted change. This cannot be undone.",
    },
  ],
};

const completed = [{ label: "cargo fmt", detail: "exit 0" }];

describe("InterruptedRunRecovery", () => {
  it("separates what completed, what was in flight, and what is on disk", () => {
    render(<InterruptedRunRecovery completed={completed} onChoose={vi.fn()} run={run} />);

    expect(screen.getByText("Completed")).toBeVisible();
    expect(screen.getByText("cargo fmt")).toBeVisible();
    expect(screen.getByText("In flight when it stopped")).toBeVisible();
    expect(screen.getByText("cargo test -p mars-ingest")).toBeVisible();
    expect(screen.getByText("Written to disk")).toBeVisible();
  });

  it("says of each written file whether it is actually there", () => {
    render(<InterruptedRunRecovery completed={completed} onChoose={vi.fn()} run={run} />);

    expect(screen.getByText("on disk")).toBeVisible();
    expect(screen.getByText("missing")).toBeVisible();
    // A path that could not be checked says so instead of claiming either.
    expect(screen.getByText("not checked")).toBeVisible();
  });

  it("states what it cannot confirm, verbatim", () => {
    render(<InterruptedRunRecovery completed={completed} onChoose={vi.fn()} run={run} />);

    expect(screen.getByText("Unverified")).toBeVisible();
    expect(screen.getByText("no test run was recorded for this work")).toBeVisible();
  });

  it("says plainly that nothing has been resumed", () => {
    render(<InterruptedRunRecovery completed={completed} onChoose={vi.fn()} run={run} />);

    expect(screen.getByText(/Nothing has been resumed, and nothing will be until you choose/)).toBeVisible();
  });

  it("offers every way out with its consequence, and defaults to none of them", async () => {
    const user = userEvent.setup();
    const onChoose = vi.fn();
    render(<InterruptedRunRecovery completed={completed} onChoose={onChoose} run={run} />);

    expect(screen.getByText("Discards every uncommitted change. This cannot be undone.")).toBeVisible();
    // Three choices, no primary: recovery is not something to press through.
    expect(screen.getAllByRole("button")).toHaveLength(3);
    expect(onChoose).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Reset the worktree/ }));
    expect(onChoose).toHaveBeenCalledWith("rerun");
  });

  it("copes with a run that had finished nothing", () => {
    render(
      <InterruptedRunRecovery
        completed={[]}
        onChoose={vi.fn()}
        run={{ ...run, inFlight: [], filesWritten: [] }}
      />,
    );

    expect(screen.getByText("Nothing had finished.")).toBeVisible();
    expect(screen.getByText("Nothing was mid-operation.")).toBeVisible();
    expect(screen.getByText("No file writes were recorded.")).toBeVisible();
  });
});
