import type { MemoryProposal, TaskProjection } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { CompletionSummary } from "./CompletionSummary";

const projection: TaskProjection = {
  taskId: "task-1",
  lastSequence: 42,
  eventCount: 42,
  runs: [],
  pendingPermissions: [],
  decidedPermissions: [],
  filesTouched: [
    { path: "crates/ingest/src/lib.rs", change: "modified", runId: "run-1", lastSequence: 20, edits: 3 },
    { path: "crates/ingest/tests/rate.rs", change: "created", runId: "run-1", lastSequence: 21, edits: 1 },
  ],
  commands: [],
  tests: [
    { command: "cargo test -p mars-ingest", status: "passed", durationMs: 4200, resultDigest: null, sequence: 30 },
  ],
  errors: [],
  worktree: null,
};

const proposal: MemoryProposal = {
  id: "proposal-1",
  projectId: "project-1",
  repositoryId: "repository-1",
  taskId: "task-1",
  scope: "repo",
  kind: "convention",
  text: "Rate limits are keyed on the API token, never the IP.",
  confidence: 0.8,
  origin: { conversationId: "c-1", messageId: "m-1", quotedText: "keyed on the token", verified: true },
  proposedBy: "claude",
  proposedAt: "2026-09-08T12:00:00Z",
  status: "pending",
  resolvedBy: null,
  resolvedAt: null,
  rejectionReason: null,
  memoryId: null,
};

function renderSummary(props: Partial<Parameters<typeof CompletionSummary>[0]> = {}) {
  return render(
    <CompletionSummary
      onPublish={vi.fn()}
      onReviewDiff={vi.fn()}
      onRunChecks={vi.fn()}
      projection={projection}
      {...props}
    />,
  );
}

describe("CompletionSummary", () => {
  it("lists what changed and what was verified", () => {
    renderSummary();

    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    expect(screen.getByText("3 edits")).toBeVisible();
    expect(screen.getByText("cargo test -p mars-ingest")).toBeVisible();
    expect(screen.getByText("4.2s")).toBeVisible();
  });

  it("counts a file edited after the last test as not covered by it", () => {
    renderSummary({
      projection: {
        ...projection,
        filesTouched: [
          { path: "late.rs", change: "modified", runId: "run-1", lastSequence: 40, edits: 1 },
        ],
      },
    });

    expect(screen.getByText(/1 file\(s\) changed after the last test run: late\.rs/)).toBeVisible();
  });

  it("says outright when nothing was tested", () => {
    renderSummary({ projection: { ...projection, tests: [] } });

    expect(screen.getByText("No test run was recorded for this work.")).toBeVisible();
    expect(screen.getByText("Nothing was verified by a recorded test run.")).toBeVisible();
  });

  it("counts failed tests and recorded errors among what was not verified", () => {
    renderSummary({
      projection: {
        ...projection,
        tests: [
          { command: "cargo test", status: "failed", durationMs: 10, resultDigest: null, sequence: 30 },
        ],
        errors: ["the adapter exited unexpectedly"],
      },
    });

    expect(screen.getByText("1 test run(s) did not pass.")).toBeVisible();
    expect(screen.getByText("1 error(s) were recorded during the run.")).toBeVisible();
  });

  it("only claims full coverage when it is actually true", () => {
    renderSummary({
      projection: {
        ...projection,
        filesTouched: [
          { path: "early.rs", change: "modified", runId: "run-1", lastSequence: 10, edits: 1 },
        ],
      },
    });

    expect(
      screen.getByText(/Every changed file was covered by a test run that passed after it changed/),
    ).toBeVisible();
  });

  it("offers exactly three next moves", async () => {
    const user = userEvent.setup();
    const onReviewDiff = vi.fn();
    const onRunChecks = vi.fn();
    const onPublish = vi.fn();
    renderSummary({ onPublish, onReviewDiff, onRunChecks });

    expect(screen.getAllByRole("button")).toHaveLength(3);

    await user.click(screen.getByRole("button", { name: /Review the diff/ }));
    await user.click(screen.getByRole("button", { name: /Run checks/ }));
    await user.click(screen.getByRole("button", { name: /Publish/ }));

    expect(onReviewDiff).toHaveBeenCalled();
    expect(onRunChecks).toHaveBeenCalled();
    expect(onPublish).toHaveBeenCalled();
  });

  it("surfaces memory awaiting approval and says it is not in effect", () => {
    renderSummary({ proposals: [proposal], onReviewMemory: vi.fn() });

    expect(screen.getByText("Rate limits are keyed on the API token, never the IP.")).toBeVisible();
    expect(screen.getByText(/None of it is in effect/)).toBeVisible();
  });

  it("does not count memory that was already resolved", () => {
    renderSummary({ proposals: [{ ...proposal, status: "approved" }] });

    expect(screen.queryByText(/awaiting your approval/i)).not.toBeInTheDocument();
  });
});
