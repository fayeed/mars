import type { ReviewFinding, ReviewRecord } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ReviewFindings } from "./ReviewFindings";

function finding(overrides: Partial<ReviewFinding> = {}): ReviewFinding {
  return {
    id: "finding-1",
    reviewId: "review-1",
    severity: "warning",
    title: "The limiter is checked after the write",
    body: "A request that fails the limit has already been accepted by then.",
    relativePath: "crates/ingest/src/lib.rs",
    line: 82,
    evidence: ["self.inner.accept()?;"],
    status: "open",
    fingerprint: "abc123",
    ...overrides,
  };
}

const review: ReviewRecord = {
  id: "review-1",
  taskId: "task-1",
  implementationSessionId: "session-1",
  reviewerAdapter: "codex",
  status: "candidate",
  verdict: "changes_requested",
  summary: "Two problems with ordering, one cosmetic.",
  handoffDigest: "9f8e7d6c5b4a3928",
  findings: [
    finding({ id: "f-info", severity: "info", title: "Naming nit" }),
    finding({ id: "f-blocker", severity: "blocker", title: "Unbounded retry loop" }),
    finding(),
  ],
  createdAt: "2026-09-08T12:00:00Z",
  updatedAt: "2026-09-08T12:00:00Z",
};

function renderFindings(props: Partial<Parameters<typeof ReviewFindings>[0]> = {}) {
  return render(
    <ReviewFindings
      onAskForFix={vi.fn()}
      onDecide={vi.fn()}
      onOpenFinding={vi.fn()}
      review={review}
      state="idle"
      {...props}
    />,
  );
}

describe("ReviewFindings", () => {
  it("says which agent read the change and what it could do", () => {
    renderFindings({
      implementerLabel: "Claude",
      reviewerCeiling: {
        readFiles: true,
        writeFiles: false,
        commands: "read_only",
        network: false,
        gitPush: false,
      },
    });

    expect(screen.getByText("codex")).toBeVisible();
    expect(screen.getByText("Claude")).toBeVisible();
    expect(screen.getByText("read files, run read_only commands")).toBeVisible();
  });

  it("orders findings by severity, worst first", () => {
    renderFindings();

    const titles = screen
      .getAllByRole("listitem")
      .map((item) => item.textContent ?? "");
    expect(titles[0]).toContain("Unbounded retry loop");
    expect(titles[2]).toContain("Naming nit");
  });

  it("anchors each finding to exact lines and opens them", async () => {
    const user = userEvent.setup();
    const onOpenFinding = vi.fn();
    renderFindings({ onOpenFinding });

    await user.click(
      screen.getAllByRole("button", { name: /crates\/ingest\/src\/lib\.rs:82/ })[0],
    );
    expect(onOpenFinding).toHaveBeenCalledWith("crates/ingest/src/lib.rs", 82);
  });

  it("offers accept, dismiss and ask-for-a-fix on an open finding", async () => {
    const user = userEvent.setup();
    const onDecide = vi.fn();
    const onAskForFix = vi.fn();
    renderFindings({ onAskForFix, onDecide });

    await user.click(screen.getAllByRole("button", { name: "Accept" })[0]);
    expect(onDecide).toHaveBeenCalledWith(expect.objectContaining({ id: "f-blocker" }), "resolved");

    await user.click(screen.getAllByRole("button", { name: "Dismiss" })[0]);
    expect(onDecide).toHaveBeenCalledWith(
      expect.objectContaining({ id: "f-blocker" }),
      "dismissed",
    );

    await user.click(screen.getAllByRole("button", { name: "Ask for a fix" })[0]);
    expect(onAskForFix).toHaveBeenCalledWith(expect.objectContaining({ id: "f-blocker" }));
  });

  it("stops offering decisions once a finding is settled", () => {
    renderFindings({
      review: { ...review, findings: [finding({ status: "dismissed" })] },
    });

    expect(screen.queryByRole("button", { name: "Accept" })).not.toBeInTheDocument();
    expect(screen.getByText("dismissed")).toBeVisible();
  });

  it("counts what is still open", () => {
    renderFindings();

    expect(screen.getByText("3 still open")).toBeVisible();
  });

  it("reports a clean review as a result, not as an absence", () => {
    renderFindings({
      review: { ...review, findings: [], verdict: "pass", summary: "Reads correctly." },
    });

    expect(screen.getByText("Nothing to report")).toBeVisible();
    expect(screen.getByText("no changes requested")).toBeVisible();
  });

  it("says no review has run rather than showing an empty list", () => {
    renderFindings({ review: null });

    expect(screen.getByText("No review yet")).toBeVisible();
    expect(screen.getByText(/cannot alter anything while it reads/)).toBeVisible();
  });
});
