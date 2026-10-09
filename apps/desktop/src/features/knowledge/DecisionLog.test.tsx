import type { KnowledgeItem, KnowledgeRecord } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DecisionLog, type DecisionLogProps } from "./DecisionLog";

function record(overrides: Partial<KnowledgeRecord>): KnowledgeRecord {
  return {
    id: "k1",
    projectId: "p1",
    repositoryId: "r1",
    kind: "decision",
    title: "Use a token bucket",
    body: "Rate limit ingest with a token bucket.\n\nWhy: bursts are normal.\n\nConsidered and not chosen:\n- fixed window\n- leaky bucket",
    status: "candidate",
    confidence: 0.8,
    provenanceId: "pv1",
    provenanceKind: "agent_inferred",
    resolutionProvenanceId: null,
    supersedesId: null,
    pinned: false,
    createdAt: "2026-10-09T10:00:00Z",
    updatedAt: "2026-10-09T10:00:00Z",
    ...overrides,
  };
}

function item(
  overrides: Partial<Omit<KnowledgeItem, "record">> & { record?: Partial<KnowledgeRecord> } = {},
): KnowledgeItem {
  const { record: recordOverrides, ...rest } = overrides;
  return {
    record: record(recordOverrides ?? {}),
    taskId: "task-1",
    taskTitle: "Rate limit ingest",
    agent: "claude",
    runId: "run-1",
    via: "agent",
    alternatives: ["fixed window", "leaky bucket"],
    files: ["src/ingest.ts"],
    ...rest,
  };
}

const approved = item({
  record: {
    id: "k2",
    title: "Keep SQLite",
    body: "Stay on SQLite.\n\nWhy: one process.",
    status: "approved",
    createdAt: "2026-10-08T10:00:00Z",
  },
  alternatives: [],
  files: [],
  via: "extraction",
  agent: "codex",
});

const summary = item({
  record: { id: "k3", kind: "summary", title: "Summary of the run", body: "Did things.", status: "candidate" },
  alternatives: [],
});

function renderLog(props: Partial<DecisionLogProps> = {}) {
  const onResolve = vi.fn();
  const view = render(
    <DecisionLog
      includeResolved={false}
      items={[item(), approved, summary]}
      onIncludeResolved={vi.fn()}
      onResolve={onResolve}
      state="idle"
      {...props}
    />,
  );
  return { ...view, onResolve };
}

describe("DecisionLog", () => {
  it("puts what is waiting on the person first, with what made it checkable", () => {
    renderLog();

    const waiting = screen.getByRole("region", { name: "Waiting on you" });
    const decision = within(waiting).getByRole("listitem", { name: "Use a token bucket" });
    expect(decision).toHaveTextContent("needs your review");
    expect(decision).toHaveTextContent("Why: bursts are normal.");
    expect(decision).toHaveTextContent("claude, while working");
    expect(decision).toHaveTextContent("in Rate limit ingest");
    // The alternatives are listed once, not repeated inside the body.
    expect(within(decision).getAllByText("fixed window")).toHaveLength(1);
    expect(within(decision).getByRole("button", { name: "src/ingest.ts" })).toBeVisible();

    const settled = screen.getByRole("region", { name: "Settled" });
    expect(within(settled).getByRole("listitem", { name: "Keep SQLite" })).toHaveTextContent(
      "read from codex's session",
    );
  });

  it("hides records of what happened unless everything is asked for", async () => {
    const user = userEvent.setup();
    renderLog();
    expect(screen.queryByText("Summary of the run")).toBeNull();

    await user.selectOptions(screen.getByRole("combobox", { name: "Show" }), "everything");
    expect(screen.getByText("Summary of the run")).toBeVisible();
  });

  it("approves, rejects and rewords a candidate", async () => {
    const user = userEvent.setup();
    const { onResolve } = renderLog();
    const decision = screen.getByRole("listitem", { name: "Use a token bucket" });

    await user.click(within(decision).getByRole("button", { name: "Approve" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.objectContaining({ taskId: "task-1" }), "approve");

    await user.click(within(decision).getByRole("button", { name: "Reject" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.anything(), "reject");

    await user.click(within(decision).getByRole("button", { name: "Reword…" }));
    const save = within(decision).getByRole("button", { name: "Approve as reworded" });
    expect(save).toBeDisabled();
    const title = within(decision).getByRole("textbox", { name: "Title" });
    await user.clear(title);
    await user.type(title, "Use a token bucket per tenant");
    await user.click(save);
    expect(onResolve).toHaveBeenLastCalledWith(expect.anything(), "correct", {
      title: "Use a token bucket per tenant",
      body: expect.stringContaining("Rate limit ingest with a token bucket."),
    });
  });

  it("corrects, retires and pins what was approved", async () => {
    const user = userEvent.setup();
    const { onResolve } = renderLog();
    const decision = screen.getByRole("listitem", { name: "Keep SQLite" });

    expect(within(decision).queryByRole("button", { name: "Approve" })).toBeNull();
    await user.click(within(decision).getByRole("button", { name: "No longer true" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.anything(), "mark_stale");
    await user.click(within(decision).getByRole("button", { name: "Pin" }));
    expect(onResolve).toHaveBeenLastCalledWith(expect.anything(), "pin");
    expect(within(decision).getByRole("button", { name: "Correct…" })).toBeVisible();
  });

  it("opens the task and the files a decision names", async () => {
    const user = userEvent.setup();
    const onOpenTask = vi.fn();
    const onOpenFile = vi.fn();
    renderLog({ onOpenFile, onOpenTask });
    const decision = screen.getByRole("listitem", { name: "Use a token bucket" });

    await user.click(within(decision).getByRole("button", { name: "in Rate limit ingest" }));
    expect(onOpenTask).toHaveBeenCalledWith("task-1");
    await user.click(within(decision).getByRole("button", { name: "src/ingest.ts" }));
    expect(onOpenFile).toHaveBeenCalledWith("src/ingest.ts");
  });

  it("filters by words and file paths", async () => {
    const user = userEvent.setup();
    renderLog();
    await user.type(screen.getByRole("textbox", { name: "Filter decisions" }), "ingest.ts");

    expect(screen.getByText("Use a token bucket")).toBeVisible();
    expect(screen.queryByText("Keep SQLite")).toBeNull();
  });

  it("explains an empty log", () => {
    renderLog({ items: [] });
    expect(screen.getByText("No decisions recorded yet")).toBeVisible();
  });

  it("asks for resolved records when the person wants them", async () => {
    const user = userEvent.setup();
    const onIncludeResolved = vi.fn();
    renderLog({ onIncludeResolved });
    await user.click(screen.getByRole("checkbox", { name: "Include rejected and superseded" }));
    expect(onIncludeResolved).toHaveBeenCalledWith(true);
  });
});
