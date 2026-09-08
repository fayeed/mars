import type { ActivityEvent, ActivityKind } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ActivityLog, eventActivityState, formatDuration } from "./ActivityLog";

let sequence = 0;

function event(kind: ActivityKind, overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  sequence += 1;
  return {
    id: `event-${sequence}`,
    taskId: "task-1",
    sequence,
    runId: "run-1",
    kind,
    actor: "agent",
    actorLabel: "Claude",
    summary: `${kind} happened`,
    payload: {},
    payloadDigest: null,
    occurredAt: "2026-09-08T12:00:00Z",
    durationMs: null,
    ...overrides,
  };
}

describe("formatDuration", () => {
  it("uses the precision the number deserves", () => {
    expect(formatDuration(420)).toBe("420ms");
    expect(formatDuration(4200)).toBe("4.2s");
    expect(formatDuration(125_000)).toBe("2m 5s");
    expect(formatDuration(null)).toBeNull();
  });
});

describe("eventActivityState", () => {
  it("calls a command with no reported status streaming, not idle", () => {
    expect(eventActivityState(event("command", { payload: { status: "running" } }))).toBe(
      "streaming",
    );
  });

  it("reads a failed test as failed and a passed one as done", () => {
    expect(eventActivityState(event("test_run", { payload: { status: "failed" } }))).toBe("failed");
    expect(eventActivityState(event("test_run", { payload: { status: "passed" } }))).toBe("done");
  });

  it("treats a permission request as blocked", () => {
    expect(eventActivityState(event("permission_request"))).toBe("blocked");
  });
});

describe("ActivityLog", () => {
  it("interleaves prose turns and machine rows in log order", () => {
    render(
      <ActivityLog
        events={[
          event("message", { actor: "human", actorLabel: "fayeed", payload: { text: "Rate limit it." } }),
          event("command", { payload: { command: "cargo test", status: "completed", exitCode: 0 } }),
          event("message", { payload: { text: "Done — one test added." } }),
        ]}
      />,
    );

    const rows = within(screen.getByRole("list", { name: "Activity" })).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent("Rate limit it.");
    expect(rows[1]).toHaveTextContent("cargo test");
    expect(rows[2]).toHaveTextContent("Done — one test added.");
  });

  it("carries the result and the duration on the machine line itself", () => {
    render(
      <ActivityLog
        events={[
          event("command", {
            payload: { command: "cargo build", status: "completed", exitCode: 0 },
            durationMs: 4200,
          }),
        ]}
      />,
    );

    expect(screen.getByText("cargo build")).toBeVisible();
    expect(screen.getByText("exit 0")).toBeVisible();
    expect(screen.getByText("4.2s")).toBeVisible();
  });

  it("summarises a file change with its added and removed counts", () => {
    render(
      <ActivityLog
        events={[
          event("file_edit", {
            payload: { path: "crates/ingest/src/lib.rs", change: "modified", added: 42, removed: 8 },
          }),
        ]}
      />,
    );

    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    expect(screen.getByText("+42 −8")).toBeVisible();
  });

  it("collapses output by default and expands it on request", async () => {
    const user = userEvent.setup();
    const loadPayload = vi.fn().mockResolvedValue({
      digest: "abc",
      byteSize: 20,
      text: "   Compiling mars-ingest",
    });
    render(
      <ActivityLog
        events={[
          event("command", {
            payload: { command: "cargo build", status: "completed" },
            payloadDigest: "abc",
          }),
        ]}
        loadPayload={loadPayload}
      />,
    );

    expect(screen.queryByText(/Compiling mars-ingest/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { expanded: false }));

    expect(await screen.findByText(/Compiling mars-ingest/)).toBeVisible();
    expect(loadPayload).toHaveBeenCalledWith("abc");
  });

  it("resolves a payload once, however often the row is toggled", async () => {
    const user = userEvent.setup();
    const loadPayload = vi.fn().mockResolvedValue({ digest: "abc", byteSize: 1, text: "output" });
    render(
      <ActivityLog
        events={[event("command", { payload: { command: "ls" }, payloadDigest: "abc" })]}
        loadPayload={loadPayload}
      />,
    );

    const row = screen.getByRole("button", { expanded: false });
    await user.click(row);
    await screen.findByText("output");
    await user.click(row);
    await user.click(row);

    expect(loadPayload).toHaveBeenCalledTimes(1);
  });

  it("opens a failure without being asked, and links where it happened", async () => {
    const user = userEvent.setup();
    const onOpenFile = vi.fn();
    render(
      <ActivityLog
        events={[
          event("test_run", {
            payload: {
              command: "cargo test",
              status: "failed",
              output: "assertion `left == right` failed\n  left: 429\n right: 200\nat src/lib.rs:82:9",
            },
          }),
        ]}
        onOpenFile={onOpenFile}
      />,
    );

    // Evidence is expanded because the row failed, not because anyone clicked.
    // The assertion is lifted out *and* left in the raw output below it, so
    // recognising it never costs the reader the original text.
    expect(screen.getAllByText(/assertion `left == right` failed/)).toHaveLength(2);
    expect(screen.getByText("429")).toBeVisible();

    await user.click(screen.getByRole("button", { name: /src\/lib\.rs:82:9/ }));
    expect(onOpenFile).toHaveBeenCalledWith("src/lib.rs", 82);
  });

  it("says the output could not be read rather than showing an empty row", async () => {
    const user = userEvent.setup();
    render(
      <ActivityLog
        events={[event("command", { payload: { command: "ls" }, payloadDigest: "gone" })]}
        loadPayload={() => Promise.reject(new Error("payload is corrupt"))}
      />,
    );

    await user.click(screen.getByRole("button", { expanded: false }));
    expect(await screen.findByText("the recorded output could not be read")).toBeVisible();
  });

  it("gives a row to a kind it has no special rendering for", () => {
    render(<ActivityLog events={[event("recovery_noted", { summary: "Run was interrupted" })]} />);

    expect(screen.getByText("Run was interrupted")).toBeVisible();
  });

  it("lets a screen put its own card where the event happened", () => {
    render(
      <ActivityLog
        events={[
          event("message", { payload: { text: "first" } }),
          event("worktree_planned"),
          event("message", { payload: { text: "last" } }),
        ]}
        renderEvent={(candidate) =>
          candidate.kind === "worktree_planned" ? <p>the plan card</p> : null
        }
      />,
    );

    const rows = within(screen.getByRole("list", { name: "Activity" })).getAllByRole("listitem");
    expect(rows[1]).toHaveTextContent("the plan card");
  });
});
