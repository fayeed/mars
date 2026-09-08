import type { ActivityEvent, ReplayedRun } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { RunStatusLine, runActivityState } from "./RunStatusLine";

const START = Date.parse("2026-09-08T12:00:00Z");

const run: ReplayedRun = {
  runId: "run-1",
  actorLabel: "Claude",
  state: "running",
  startedAt: "2026-09-08T12:00:00Z",
  endedAt: null,
  firstSequence: 1,
  lastSequence: 42,
  messageCount: 3,
  blockedOnRequest: null,
  failure: null,
};

const event: ActivityEvent = {
  id: "event-1",
  taskId: "task-1",
  sequence: 42,
  runId: "run-1",
  kind: "command",
  actor: "agent",
  actorLabel: "Claude",
  summary: "cargo test -p mars-ingest",
  payload: { command: "cargo test -p mars-ingest", status: "running" },
  payloadDigest: null,
  occurredAt: "2026-09-08T12:00:10Z",
  durationMs: null,
};

describe("runActivityState", () => {
  it("maps the log's view of a run onto the shell's vocabulary", () => {
    expect(runActivityState({ ...run, state: "running" })).toBe("streaming");
    expect(runActivityState({ ...run, state: "blocked" })).toBe("blocked");
    expect(runActivityState({ ...run, state: "failed" })).toBe("failed");
    expect(runActivityState(null)).toBe("idle");
  });
});

describe("RunStatusLine", () => {
  it("says what is happening now, with elapsed time and where the log is", () => {
    render(
      <RunStatusLine latest={event} now={START + 65_000} onInterrupt={vi.fn()} run={run} />,
    );

    expect(screen.getByText("cargo test -p mars-ingest")).toBeVisible();
    expect(screen.getByText("1m 5s")).toBeVisible();
    expect(screen.getByText("seq 42")).toBeVisible();
  });

  it("distinguishes what is still running from what merely happened last", () => {
    render(
      <RunStatusLine
        latest={{ ...event, payload: { command: "cargo build", status: "completed" } }}
        now={START}
        onInterrupt={vi.fn()}
        run={run}
      />,
    );

    expect(screen.getByText(/^Last:/)).toBeVisible();
  });

  it("interrupts on request", async () => {
    const user = userEvent.setup();
    const onInterrupt = vi.fn();
    render(<RunStatusLine latest={event} now={START} onInterrupt={onInterrupt} run={run} />);

    await user.click(screen.getByRole("button", { name: "Interrupt" }));
    expect(onInterrupt).toHaveBeenCalled();
  });

  it("stays offering the interrupt while the run is blocked, and says what on", () => {
    render(
      <RunStatusLine
        blockedSummary="run `rm -rf target` — waiting on you"
        latest={event}
        now={START}
        onInterrupt={vi.fn()}
        run={{ ...run, state: "blocked" }}
      />,
    );

    expect(screen.getByText("run `rm -rf target` — waiting on you")).toBeVisible();
    expect(screen.getByRole("button", { name: "Interrupt" })).toBeEnabled();
  });

  it("reports a failure rather than the last thing that happened before it", () => {
    render(
      <RunStatusLine
        latest={event}
        now={START}
        onInterrupt={vi.fn()}
        run={{ ...run, state: "failed", failure: "the adapter exited with code 1" }}
      />,
    );

    expect(screen.getByText("the adapter exited with code 1")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Interrupt" })).not.toBeInTheDocument();
  });

  it("takes no room when there is no run, or when the run finished", () => {
    const { container: idle } = render(
      <RunStatusLine latest={null} onInterrupt={vi.fn()} run={null} />,
    );
    expect(idle.querySelector('[data-slot="run-status"]')).toBeNull();

    const { container: finished } = render(
      <RunStatusLine latest={event} onInterrupt={vi.fn()} run={{ ...run, state: "completed" }} />,
    );
    expect(finished.querySelector('[data-slot="run-status"]')).toBeNull();
  });
});
