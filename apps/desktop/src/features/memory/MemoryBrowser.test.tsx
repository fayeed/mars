import type { Memory, MemoryProposal } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MemoryBrowser } from "./MemoryBrowser";

const memory: Memory = {
  id: "memory-1",
  projectId: "project-1",
  repositoryId: "repository-1",
  taskId: null,
  scope: "repo",
  kind: "convention",
  text: "Rate limits are keyed on the API token, never the IP.",
  confidence: 0.9,
  origin: {
    conversationId: "task-7",
    messageId: "msg-12",
    quotedText: "we always key limits on the token",
    verified: true,
  },
  proposalId: "proposal-1",
  approvedBy: "fayeed",
  approvedAt: "2026-09-01T10:00:00Z",
  supersedes: null,
  status: "active",
  revokedBy: null,
  revokedAt: null,
  revocationReason: null,
  useCount: 4,
  usedInTasks: 3,
  createdAt: "2026-09-01T10:00:00Z",
  updatedAt: "2026-09-01T10:00:00Z",
};

const proposal: MemoryProposal = {
  id: "proposal-2",
  projectId: "project-1",
  repositoryId: "repository-1",
  taskId: "task-1",
  scope: "repo",
  kind: "caution",
  text: "The ingest tests need a running Redis.",
  confidence: 0.6,
  origin: {
    conversationId: "task-1",
    messageId: "msg-3",
    quotedText: "they fail without redis",
    verified: false,
  },
  proposedBy: "claude",
  proposedAt: "2026-09-08T12:00:00Z",
  status: "pending",
  resolvedBy: null,
  resolvedAt: null,
  rejectionReason: null,
  memoryId: null,
};

function renderBrowser(props: Partial<Parameters<typeof MemoryBrowser>[0]> = {}) {
  return render(
    <MemoryBrowser
      filters={{ scopes: [], kinds: [], statuses: [], contains: "" }}
      memories={[memory]}
      onApprove={vi.fn()}
      onFilters={vi.fn()}
      onJumpToOrigin={vi.fn()}
      onReject={vi.fn()}
      onRevise={vi.fn()}
      onRevoke={vi.fn()}
      proposals={[]}
      state="idle"
      {...props}
    />,
  );
}

describe("MemoryBrowser rows", () => {
  it("shows the text, the origin excerpt and the approval metadata", () => {
    renderBrowser();

    expect(screen.getByText(memory.text)).toBeVisible();
    expect(screen.getByText("we always key limits on the token")).toBeVisible();
    expect(screen.getByText(/approved by fayeed/)).toBeVisible();
  });

  it("shows how much a memory has actually been used", () => {
    renderBrowser();

    expect(screen.getByText("used 4×")).toBeVisible();
    expect(screen.getByText("in 3 task(s)")).toBeVisible();
  });

  it("jumps to the turn the memory came from", async () => {
    const user = userEvent.setup();
    const onJumpToOrigin = vi.fn();
    renderBrowser({ onJumpToOrigin });

    await user.click(screen.getByRole("button", { name: /task-7:msg-12/ }));
    expect(onJumpToOrigin).toHaveBeenCalledWith("task-7", "msg-12");
  });

  it("says when an excerpt was never checked against its turn", () => {
    renderBrowser({
      memories: [{ ...memory, origin: { ...memory.origin, verified: false } }],
    });

    expect(screen.getByText("excerpt not verified against that turn")).toBeVisible();
  });
});

describe("MemoryBrowser editing", () => {
  it("supersedes rather than overwriting, and says so", async () => {
    const user = userEvent.setup();
    const onRevise = vi.fn();
    renderBrowser({ onRevise });

    await user.click(screen.getByRole("button", { name: /Edit/ }));
    expect(screen.getByText(/supersedes this memory rather than overwriting it/)).toBeVisible();

    const box = screen.getByRole("textbox", { name: `Revise memory ${memory.id}` });
    await user.clear(box);
    await user.type(box, "Limits are keyed on the token.");
    await user.click(screen.getByRole("button", { name: "Supersede" }));

    expect(onRevise).toHaveBeenCalledWith(memory, "Limits are keyed on the token.");
  });

  it("requires a reason before revoking", async () => {
    const user = userEvent.setup();
    const onRevoke = vi.fn();
    renderBrowser({ onRevoke });

    await user.click(screen.getByRole("button", { name: /Revoke/ }));
    expect(screen.getByRole("button", { name: "Revoke" })).toBeDisabled();

    await user.type(
      screen.getByRole("textbox", { name: `Why revoke memory ${memory.id}` }),
      "We key on IP now.",
    );
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(onRevoke).toHaveBeenCalledWith(memory, "We key on IP now.");
  });

  it("offers no edits on a memory that is no longer active", () => {
    renderBrowser({ memories: [{ ...memory, status: "revoked" }] });

    expect(screen.queryByRole("button", { name: /Edit/ })).not.toBeInTheDocument();
    // "revoked" is also a filter toggle, so read the row rather than the page.
    const row = screen.getByText(memory.text).closest("li");
    expect(within(row as HTMLElement).getByText("revoked")).toBeVisible();
  });
});

describe("MemoryBrowser proposals", () => {
  it("keeps proposals separate and says they are not in effect", () => {
    renderBrowser({ proposals: [proposal] });

    expect(screen.getByText("Awaiting your approval")).toBeVisible();
    expect(screen.getByText("none of this is in effect")).toBeVisible();
  });

  it("approves as written, or as edited", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn();
    renderBrowser({ onApprove, proposals: [proposal] });

    expect(screen.getByRole("button", { name: "Approve" })).toBeVisible();
    await user.type(
      screen.getByRole("textbox", { name: `Proposed memory ${proposal.id}` }),
      " Use the test compose file.",
    );
    await user.click(screen.getByRole("button", { name: "Edit and approve" }));

    expect(onApprove).toHaveBeenCalledWith(
      proposal,
      "The ingest tests need a running Redis. Use the test compose file.",
    );
  });

  it("requires a reason before rejecting", async () => {
    const user = userEvent.setup();
    const onReject = vi.fn();
    renderBrowser({ onReject, proposals: [proposal] });

    expect(screen.getByRole("button", { name: "Reject" })).toBeDisabled();
    await user.type(
      screen.getByRole("textbox", { name: `Why reject proposal ${proposal.id}` }),
      "Not a convention.",
    );
    await user.click(screen.getByRole("button", { name: "Reject" }));

    expect(onReject).toHaveBeenCalledWith(proposal, "Not a convention.");
  });
});

describe("MemoryBrowser filters", () => {
  it("toggles a scope filter", async () => {
    const user = userEvent.setup();
    const onFilters = vi.fn();
    renderBrowser({ onFilters });

    await user.click(screen.getByRole("button", { name: "repo" }));
    expect(onFilters).toHaveBeenCalledWith(expect.objectContaining({ scopes: ["repo"] }));
  });

  it("says what no status selected means", () => {
    renderBrowser();

    expect(screen.getByText("No status selected means active only.")).toBeVisible();
  });

  it("explains an empty list rather than just showing one", () => {
    renderBrowser({ memories: [] });

    expect(screen.getByText("No memories match")).toBeVisible();
    expect(screen.getByText(/only ever created by you approving a proposal/)).toBeVisible();
  });
});
