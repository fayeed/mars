import type { ReviewFinding, ReviewHandoff, ReviewRecord } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { CrossAgentReview } from "./CrossAgentReview";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const finding: ReviewFinding = {
  id: "finding-1",
  reviewId: "review-1",
  severity: "error",
  title: "Unsafe operand slips through",
  body: "Validate operands before adding them.",
  relativePath: "src/math.ts",
  line: 8,
  evidence: ["diff:src/math.ts:7"],
  status: "open",
  fingerprint: "fingerprint-1",
};

const review: ReviewRecord = {
  id: "review-1",
  taskId: "task-1",
  implementationSessionId: "session-claude",
  reviewerAdapter: "codex",
  status: "candidate",
  verdict: "changes_requested",
  diffSha256: "diff-sha-at-review-time",
  summary: "One correctness issue remains.",
  handoffDigest: "digest-1",
  findings: [finding],
  createdAt: "2026-08-30T00:00:00Z",
  updatedAt: "2026-08-30T00:00:00Z",
};

const handoff: ReviewHandoff = {
  taskId: "task-1",
  implementationSessionId: "session-claude",
  implementationAdapter: "claude",
  reviewerAdapter: "codex",
  contextPack: {
    taskId: "task-1",
    adapter: "codex",
    maxBytes: 4096,
    totalBytes: 100,
    sectionUsage: [],
    items: [],
    exclusions: [],
  },
  implementationEvents: [{
    sequence: 1,
    eventType: "test",
    summary: "pnpm test",
    sourceLocation: "agent_session:session-claude:normalized:1",
  }],
  diff: "diff --git a/src/math.ts b/src/math.ts\n+unsafe guard",
  diffSha256: "a".repeat(64),
  maxBytes: 64 * 1024,
  totalBytes: 2048,
};

afterEach(() => vi.clearAllMocks());

it("previews the exact cross-agent handoff and sends findings back to the implementer", async () => {
  const user = userEvent.setup();
  const onSendFollowup = vi.fn();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "list_task_reviews") return Promise.resolve([]);
    if (command === "build_review_handoff") return Promise.resolve(handoff);
    if (command === "run_cross_agent_review") return Promise.resolve(review);
    if (command === "review_followup") return Promise.resolve("Fix the unsafe operand finding.");
    if (command === "set_review_finding_status") return Promise.resolve({ ...finding, status: "sent" });
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });

  render(
    <CrossAgentReview
      implementationAdapter="claude"
      onSendFollowup={onSendFollowup}
      taskId="task-1"
    />,
  );
  await user.click(screen.getByRole("button", { name: "Review with Codex" }));
  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByText(/Claude Code implementation → read-only Codex review/)).toBeVisible();

  await user.click(within(dialog).getByRole("button", { name: "Prepare shared handoff" }));
  expect(await within(dialog).findByText("pnpm test")).toBeVisible();
  await user.click(within(dialog).getByText("Exact diff sent to reviewer"));
  expect(within(dialog).getByText(/unsafe guard/)).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("build_review_handoff", {
    taskId: "task-1",
    reviewerAdapter: "codex",
    maxBytes: 64 * 1024,
  });

  await user.click(within(dialog).getByRole("button", { name: "Run read-only Codex review" }));
  expect(await within(dialog).findByText(finding.title)).toBeVisible();
  expect(within(dialog).getByText("src/math.ts:8 · diff:src/math.ts:7")).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("run_cross_agent_review", {
    handoff,
    modelPermission: true,
  });

  await user.click(within(dialog).getByRole("button", { name: /Send 1 finding to Claude Code/ }));
  expect(mocks.invoke).toHaveBeenCalledWith("review_followup", { reviewId: review.id });
  expect(mocks.invoke).toHaveBeenCalledWith("set_review_finding_status", {
    findingId: finding.id,
    status: "sent",
  });
  expect(onSendFollowup).toHaveBeenCalledWith("Fix the unsafe operand finding.", "claude");
});
