import type { BaseDrift, OverlapWarning } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BaseDriftMessage } from "./BaseDriftMessage";

const drift: BaseDrift = {
  baseRef: "main",
  plannedOid: "511c628aaaaaaaaa",
  currentOid: "9f8e7d6cbbbbbbbb",
  commitsBehind: 4,
  options: [
    {
      choice: "rebase",
      label: "Rebase onto the new base",
      consequence: "Replays this task's commits on top of the new base.",
    },
    {
      choice: "recreate",
      label: "Recreate the worktree",
      consequence: "Discards every uncommitted change. This cannot be undone.",
    },
    {
      choice: "continue_and_diverge",
      label: "Continue from the old base",
      consequence: "The branch keeps its base and diverges from main.",
    },
  ],
};

const overlap: OverlapWarning = {
  kind: "exact_file",
  severity: "error",
  message: "crates/ingest/src/lib.rs changed on the new base and in this task",
  evidence: ["crates/ingest/src/lib.rs"],
  otherTaskId: "task-2",
  otherTaskTitle: "Split the broker",
  otherWorktreePath: "/data/worktrees/broker",
  refreshedAt: "2026-09-08T12:00:00Z",
};

function renderDrift(props: Partial<Parameters<typeof BaseDriftMessage>[0]> = {}) {
  return render(<BaseDriftMessage drift={drift} onResolve={vi.fn()} {...props} />);
}

describe("BaseDriftMessage", () => {
  it("shows what the plan pinned and where the ref points now", () => {
    renderDrift();

    expect(screen.getByText("511c628aaaaa")).toBeVisible();
    expect(screen.getByText("9f8e7d6cbbbb")).toBeVisible();
    expect(screen.getByText("4 commit(s) behind")).toBeVisible();
  });

  it("says what the move touches, with the evidence", () => {
    renderDrift({ overlaps: [overlap] });

    expect(screen.getByText(overlap.message)).toBeVisible();
    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    expect(screen.getByText(/also in Split the broker/)).toBeVisible();
  });

  it("says so rather than staying silent when nothing overlaps", () => {
    renderDrift();

    expect(screen.getByText(/Nothing this task has touched overlaps/)).toBeVisible();
  });

  it("writes out the consequence of all three resolutions", () => {
    renderDrift();

    expect(screen.getByText(/Replays this task's commits/)).toBeVisible();
    expect(screen.getByText(/This cannot be undone/)).toBeVisible();
    expect(screen.getByText(/diverges from main/)).toBeVisible();
  });

  it("chooses nothing on its own", async () => {
    const user = userEvent.setup();
    const onResolve = vi.fn();
    renderDrift({ onResolve });

    expect(screen.getByText(/Nothing happens until you choose/)).toBeVisible();
    expect(onResolve).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: /Rebase onto the new base/ }));
    expect(onResolve).toHaveBeenCalledWith("rebase");
  });

  it("stops offering choices while one is being carried out", () => {
    renderDrift({ busy: true });

    expect(screen.getByRole("button", { name: /Rebase onto the new base/ })).toBeDisabled();
  });
});
