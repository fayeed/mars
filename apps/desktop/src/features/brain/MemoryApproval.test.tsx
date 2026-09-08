import type { DetectedConflict, Memory, MemoryProposal } from "@mars/shared-types";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { MemoryApproval } from "./MemoryApproval";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const proposal: MemoryProposal = {
  id: "proposal-1",
  projectId: "project-1",
  repositoryId: "repository-1",
  taskId: null,
  scope: "repo",
  kind: "convention",
  text: "run the formatter before committing",
  confidence: 0.8,
  origin: {
    conversationId: "session-1",
    messageId: "4",
    quotedText: "we always run the formatter before committing",
    verified: true,
  },
  proposedBy: "claude",
  proposedAt: "2026-09-01T10:00:00Z",
  status: "pending",
  resolvedBy: null,
  resolvedAt: null,
  rejectionReason: null,
  memoryId: null,
};

const memory: Memory = {
  id: "memory-1",
  projectId: "project-1",
  repositoryId: "repository-1",
  taskId: null,
  scope: "repo",
  kind: "caution",
  text: "the payment provider rate-limits at 100 requests a minute",
  confidence: 0.9,
  origin: {
    conversationId: "session-2",
    messageId: "7",
    quotedText: "careful, they rate-limit us",
    verified: false,
  },
  proposalId: "proposal-0",
  approvedBy: "fayeed",
  approvedAt: "2026-09-01T09:00:00Z",
  supersedes: null,
  status: "active",
  revokedBy: null,
  revokedAt: null,
  revocationReason: null,
  useCount: 6,
  usedInTasks: 4,
  createdAt: "2026-09-01T09:00:00Z",
  updatedAt: "2026-09-01T09:00:00Z",
};

const conflict: DetectedConflict = {
  other: { ...memory, id: "memory-2", text: "never call the payment provider directly" },
  similarity: 0.87,
  signal: "opposing_polarity",
  explanation:
    "This says nearly the same thing as an active repo memory (87% similar), but one of the two is phrased as a prohibition and the other is not.",
};

function respond(handlers: Record<string, unknown>) {
  mocks.invoke.mockImplementation((command: string) => {
    if (command in handlers) return Promise.resolve(handlers[command]);
    return Promise.reject(new Error(`unexpected command ${command}`));
  });
}

afterEach(() => vi.clearAllMocks());

it("shows a proposal with the excerpt that motivated it, and marks it as not yet in use", async () => {
  respond({ memory_pending: [proposal], memory_list: [memory] });
  render(<MemoryApproval projectId="project-1" />);

  const queue = await screen.findByLabelText("Awaiting approval");
  expect(within(queue).getByText(/nothing here is in use/i)).toBeVisible();
  expect(
    within(queue).getByText(/we always run the formatter before committing/),
  ).toBeVisible();
  expect(within(queue).getByText(/checked against the transcript/)).toBeVisible();
});

it("reports how many tasks a memory has actually shaped", async () => {
  respond({ memory_pending: [], memory_list: [memory] });
  render(<MemoryApproval projectId="project-1" />);

  const active = await screen.findByLabelText("Active memories");
  expect(within(active).getByText("used in 4 tasks")).toBeVisible();
  expect(within(active).getByText(/approved by fayeed/)).toBeVisible();
});

it("sends no edit when the text is accepted unchanged, and never claims an approver", async () => {
  const user = userEvent.setup();
  respond({ memory_pending: [proposal], memory_list: [], memory_approve: { outcome: "approved" } });
  render(<MemoryApproval projectId="project-1" />);

  await user.click(await screen.findByRole("button", { name: "Accept" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("memory_approve", {
      request: {
        proposalId: "proposal-1",
        editedText: null,
        resolutions: [],
      },
    }),
  );
});

it("sends the edited text when the human changes it first", async () => {
  const user = userEvent.setup();
  respond({ memory_pending: [proposal], memory_list: [], memory_approve: { outcome: "approved" } });
  render(<MemoryApproval projectId="project-1" />);

  const field = await screen.findByLabelText("Memory text for proposal-1");
  await user.clear(field);
  await user.type(field, "run the formatter and the linter before committing");
  await user.click(screen.getByRole("button", { name: "Edit and accept" }));

  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("memory_approve", {
      request: {
        proposalId: "proposal-1",
        editedText: "run the formatter and the linter before committing",
        resolutions: [],
      },
    }),
  );
});

it("makes the human choose when approval reports a conflict", async () => {
  const user = userEvent.setup();
  respond({
    memory_pending: [proposal],
    memory_list: [],
    memory_approve: { outcome: "conflictsRequireChoice", conflicts: [conflict] },
  });
  render(<MemoryApproval projectId="project-1" />);

  await user.click(await screen.findByRole("button", { name: "Accept" }));

  const conflicts = await screen.findByLabelText("Conflicts");
  expect(within(conflicts).getByText(/prohibition/)).toBeVisible();
  expect(within(conflicts).getByText(/never call the payment provider directly/)).toBeVisible();
  // The accept button is gone: there is no way past this without deciding.
  expect(screen.queryByRole("button", { name: "Accept" })).toBeNull();

  await user.click(screen.getByRole("button", { name: "Replace the old one" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("memory_approve", {
      request: {
        proposalId: "proposal-1",
        editedText: null,
        resolutions: [{ otherMemoryId: "memory-2", choice: "supersede_other" }],
      },
    }),
  );
});
