import type { WorktreeProposal } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { WorktreePlanMessage } from "./WorktreePlanMessage";

const proposal: WorktreeProposal = {
  taskId: "task-1",
  repositoryId: "repository-1",
  title: "Rate limit the ingest endpoint",
  intent: "Add a per-token rate limit to the ingest endpoint.",
  baseRef: "main",
  baseOid: "511c628aa9c1d4f0e7b2c3",
  branchName: "mars/rate-limit-ingest",
  worktreePath: "/data/worktrees/rate-limit-ingest",
  expectedScope: ["crates/ingest/**"],
  setupCommands: [
    { command: "cargo fetch", purpose: "prime the dependency cache", classification: "read_only" },
  ],
  preflight: [],
  disk: { availableBytes: 42 * 1024 ** 3, thresholdBytes: 1024 ** 3, low: false },
  state: "planned",
  fingerprint: "abcdef0123456789",
  approvedBy: null,
  approvedAt: null,
};

function withPreflight(): WorktreeProposal {
  return {
    ...proposal,
    preflight: [
      {
        code: "branch_exists",
        severity: "blocking",
        detail: "a branch named mars/rate-limit-ingest already exists",
        resolutions: [
          { action: "Pick another name", consequence: "The existing branch is left alone." },
        ],
      },
    ],
  };
}

describe("WorktreePlanMessage disclosure", () => {
  it("states the base commit it pinned, the branch, the path and the disk", () => {
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={proposal} />);

    expect(screen.getByText("main")).toBeVisible();
    expect(screen.getByText("511c628")).toBeVisible();
    expect(screen.getByText("mars/rate-limit-ingest")).toBeVisible();
    expect(screen.getByText("/data/worktrees/rate-limit-ingest")).toBeVisible();
    expect(screen.getByText("42.0 GiB free")).toBeVisible();
  });

  it("shows the scope and the classification the core gave each setup command", () => {
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={proposal} />);

    expect(screen.getByText("crates/ingest/**")).toBeVisible();
    expect(screen.getByText("cargo fetch")).toBeVisible();
    expect(screen.getByText("read_only")).toBeVisible();
  });

  it("says nothing has touched Git yet", () => {
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={proposal} />);

    expect(screen.getByText(/Nothing has touched Git yet/)).toBeVisible();
  });
});

describe("WorktreePlanMessage preflight", () => {
  it("refuses approval while a finding blocks it, and offers the way out", () => {
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={withPreflight()} />);

    expect(screen.getByRole("button", { name: /Approve/ })).toBeDisabled();
    expect(screen.getByText("branch_exists")).toBeVisible();
    expect(screen.getByText("Pick another name")).toBeVisible();
    expect(screen.getByText(/The existing branch is left alone/)).toBeVisible();
  });

  it("lets an edit through, because editing may be the fix", async () => {
    const user = userEvent.setup();
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={withPreflight()} />);

    await user.click(screen.getByRole("button", { name: "Remove crates/ingest/**" }));
    expect(screen.getByRole("button", { name: /Save and approve/ })).toBeEnabled();
  });
});

describe("WorktreePlanMessage inline edit", () => {
  it("reports scope edits and says the approval covers them", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn();
    render(<WorktreePlanMessage onApprove={onApprove} proposal={proposal} />);

    await user.type(screen.getByLabelText("Add a scope glob"), "crates/api/**{Enter}");
    await user.click(screen.getByRole("button", { name: /Save and approve/ }));

    expect(onApprove).toHaveBeenCalledWith({
      expectedScope: ["crates/ingest/**", "crates/api/**"],
      setupCommands: [{ command: "cargo fetch", purpose: "prime the dependency cache" }],
      fingerprint: proposal.fingerprint,
      edited: true,
    });
  });

  it("marks a command added here as not yet classified", async () => {
    const user = userEvent.setup();
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={proposal} />);

    await user.type(screen.getByLabelText("Add a setup command"), "rm -rf target{Enter}");

    expect(screen.getByText("not yet classified")).toBeVisible();
  });

  it("discards edits back to the plan the core holds", async () => {
    const user = userEvent.setup();
    render(<WorktreePlanMessage onApprove={vi.fn()} proposal={proposal} />);

    await user.click(screen.getByRole("button", { name: "Remove crates/ingest/**" }));
    await user.click(screen.getByRole("button", { name: /Discard edits/ }));

    expect(screen.getByText("crates/ingest/**")).toBeVisible();
    expect(screen.getByRole("button", { name: /^Approve/ })).toBeInTheDocument();
  });

  it("approves an untouched plan without claiming it was edited", async () => {
    const user = userEvent.setup();
    const onApprove = vi.fn();
    render(<WorktreePlanMessage onApprove={onApprove} proposal={proposal} />);

    await user.click(screen.getByRole("button", { name: /^Approve/ }));

    expect(onApprove).toHaveBeenCalledWith(expect.objectContaining({ edited: false }));
  });
});

describe("WorktreePlanMessage after approval", () => {
  it("becomes the record of what was approved, and stops offering edits", () => {
    render(
      <WorktreePlanMessage
        onApprove={vi.fn()}
        proposal={{ ...proposal, approvedBy: "fayeed", approvedAt: "2026-09-08T12:00:00Z" }}
      />,
    );

    expect(screen.getByText("fayeed")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Approve/ })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Add a scope glob")).not.toBeInTheDocument();
  });
});
