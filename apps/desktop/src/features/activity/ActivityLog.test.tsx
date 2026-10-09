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

describe("ActivityLog focus", () => {
  it("brings a turn into view, marks it, and says when it has", () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const onFocused = vi.fn();
    const events = [
      event("message", { payload: { text: "first" } }),
      event("message", { payload: { text: "the origin" } }),
    ];
    render(<ActivityLog events={events} focusSequence={events[1].sequence} onFocused={onFocused} />);

    const rows = within(screen.getByRole("list", { name: "Activity" })).getAllByRole("listitem");
    expect(rows[1]).toHaveAttribute("data-focused", "true");
    expect(rows[0]).not.toHaveAttribute("data-focused");
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "center" });
    expect(onFocused).toHaveBeenCalledTimes(1);
  });

  it("waits for a turn that has not loaded yet", () => {
    const onFocused = vi.fn();
    render(<ActivityLog events={[event("message")]} focusSequence={9999} onFocused={onFocused} />);
    expect(onFocused).not.toHaveBeenCalled();
  });
});

describe("ActivityLog failure action", () => {
  const failing = event("test_run", {
    summary: "cargo test -p mars-ingest",
    payload: { command: "cargo test", status: "failed", output: "assertion failed" },
  });

  it("offers exactly one next step on a failure, not a menu", async () => {
    const user = userEvent.setup();
    const onAct = vi.fn();
    render(
      <ActivityLog
        events={[failing]}
        failureAction={() => ({ label: "Ask Claude to fix this", onAct })}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Ask Claude to fix this" }));
    expect(onAct).toHaveBeenCalled();
  });

  it("offers nothing when the screen has no next step to give", () => {
    render(<ActivityLog events={[failing]} failureAction={() => null} />);

    expect(screen.queryByRole("button", { name: /fix this/ })).not.toBeInTheDocument();
  });
});

describe("a long transcript", () => {
  function manyEvents(count: number): ActivityEvent[] {
    return Array.from({ length: count }, (_, index) => ({
      id: `event-${index}`,
      taskId: "task-1",
      sequence: index + 1,
      runId: "run-1",
      kind: "command" as const,
      actor: "agent" as const,
      actorLabel: "claude",
      summary: `step ${index}`,
      payload: { command: `step-${index}`, status: "completed" },
      payloadDigest: null,
      occurredAt: "2026-09-09T09:00:00Z",
      durationMs: 10,
    }));
  }

  /** jsdom reports no layout, so the scroller's box has to be stated. */
  function scrollerStub(height: number): HTMLElement {
    const element = document.createElement("div");
    Object.defineProperty(element, "clientHeight", { value: height, configurable: true });
    Object.defineProperty(element, "scrollTop", { value: 0, writable: true });
    document.body.append(element);
    return element;
  }

  it("renders a window rather than the whole record", () => {
    // 2,000 rows is an ordinary day. Rendering all of them costs a second of
    // layout on every append.
    const { container } = render(
      <ActivityLog events={manyEvents(2_000)} scroller={scrollerStub(600)} />,
    );

    const rows = container.querySelectorAll("li");
    expect(rows.length).toBeLessThan(200);
    expect(rows.length).toBeGreaterThan(0);
  });

  it("leaves the scrollbar describing the whole record, not the part on screen", () => {
    const { container } = render(
      <ActivityLog events={manyEvents(2_000)} scroller={scrollerStub(600)} />,
    );

    const spacers = Array.from(container.querySelectorAll('li[aria-hidden="true"]'));
    const reserved = spacers.reduce(
      (total, node) => total + Number.parseFloat((node as HTMLElement).style.height || "0"),
      0,
    );
    expect(reserved).toBeGreaterThan(0);
  });

  it("renders a short log whole, so find-in-page and screen readers reach all of it", () => {
    // Virtualizing has a real price: a row that is not in the document cannot
    // be found or announced. Forty rows is not worth paying it for.
    const events = manyEvents(40);
    const { container } = render(<ActivityLog events={events} scroller={scrollerStub(600)} />);

    expect(container.querySelectorAll("li")).toHaveLength(40);
    expect(container.querySelectorAll('li[aria-hidden="true"]')).toHaveLength(0);
  });

  it("renders everything when it has no scroller to measure against", () => {
    const { container } = render(<ActivityLog events={manyEvents(300)} />);
    expect(container.querySelectorAll("li")).toHaveLength(300);
  });
});
