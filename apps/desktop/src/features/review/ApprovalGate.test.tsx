import type { ReviewFinding, TaskProjection } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ApprovalGate } from "./ApprovalGate";

const projection: TaskProjection = {
  taskId: "task-1",
  lastSequence: 40,
  eventCount: 40,
  runs: [],
  pendingPermissions: [],
  decidedPermissions: [],
  filesTouched: [
    { path: "crates/ingest/src/lib.rs", change: "modified", runId: "r", lastSequence: 10, edits: 2 },
  ],
  commands: [],
  tests: [
    { command: "cargo test", status: "passed", durationMs: 1200, resultDigest: null, sequence: 20 },
  ],
  errors: [],
  worktree: null,
};

function finding(overrides: Partial<ReviewFinding> = {}): ReviewFinding {
  return {
    id: "f-1",
    reviewId: "review-1",
    severity: "blocker",
    title: "Unbounded retry loop",
    body: "",
    relativePath: "crates/ingest/src/lib.rs",
    line: 91,
    evidence: [],
    status: "open",
    fingerprint: "abc",
    ...overrides,
  };
}

function renderGate(props: Partial<Parameters<typeof ApprovalGate>[0]> = {}) {
  return render(
    <ApprovalGate onApprove={vi.fn()} outstanding={[]} projection={projection} {...props} />,
  );
}

describe("ApprovalGate", () => {
  it("says nothing has left the machine yet", () => {
    renderGate();

    expect(screen.getByText("nothing has left this machine")).toBeVisible();
  });

  it("summarises the change and its verification", () => {
    renderGate();

    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    expect(screen.getByText(/test run\(s\) passed/)).toBeVisible();
  });

  it("says outright when nothing verified the change", () => {
    renderGate({ projection: { ...projection, tests: [] } });

    expect(
      screen.getByText(/Approving this releases work nothing checked/),
    ).toBeVisible();
  });

  it("calls out open blockers without pretending approval resolves them", () => {
    renderGate({ outstanding: [finding()] });

    expect(
      screen.getByText(/Approving does not resolve them; it releases the change with them open/),
    ).toBeVisible();
  });

  it("does not raise the alarm for findings that are not blocking", () => {
    renderGate({ outstanding: [finding({ severity: "info" })] });

    expect(screen.queryByText(/Approving does not resolve them/)).not.toBeInTheDocument();
  });

  it("records the approval as a message, in the approver's words", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn();
    renderGate({ onApprove });

    expect(screen.getByText(/recorded as a message in this conversation/)).toBeVisible();
    await user.type(
      screen.getByRole("textbox", { name: "What you are approving, in your words" }),
      "Read the diff, ran the tests locally.",
    );
    await user.click(screen.getByRole("button", { name: /Approve this change/ }));

    expect(onApprove).toHaveBeenCalledWith("Read the diff, ran the tests locally.");
  });

  it("allows approval with no note rather than demanding one", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn();
    renderGate({ onApprove });

    await user.click(screen.getByRole("button", { name: /Approve this change/ }));
    expect(onApprove).toHaveBeenCalledWith("");
  });

  it("becomes the record once approved, and stops offering the action", () => {
    renderGate({ approvedBy: "fayeed", approvedAt: "2026-09-08T13:00:00Z" });

    expect(screen.getByText("fayeed")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Approve this change/ })).not.toBeInTheDocument();
  });
});
