import type { CleanupPlan, IntegrationPreview } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { IntegratePanel } from "./IntegratePanel";

const preview: IntegrationPreview = {
  root: "/repo",
  base: "main",
  baseOid: "aaa",
  head: "mars/rate-limit",
  headOid: "bbb",
  commits: [
    { oid: "bbb", shortOid: "bbb", subject: "Limit ingest", author: "claude", authoredAt: "", refs: [] },
  ],
  filesChanged: ["M  src/ingest.ts"],
  conflicts: [],
  fastForward: false,
  blockers: [],
  fingerprint: "fp-1",
};

const cleanup: CleanupPlan = {
  taskId: "task-1",
  worktreePath: "/w/rate-limit",
  branchName: "mars/rate-limit",
  branchOid: "bbb",
  archiveRef: "refs/mars/archive/mars/rate-limit",
  uncommitted: ["scratch.txt"],
  commitsAheadOfBase: 1,
  reversible: false,
  fingerprint: "cleanup-1",
};

function routes(overrides: Record<string, unknown> = {}) {
  const table: Record<string, unknown> = {
    integration_preview: preview,
    integration_apply: { oid: "ccc", shortOid: "ccc", subject: "Rate limit", author: "me", authoredAt: "", refs: [] },
    worktree_last_cleanup: null,
    worktree_cleanup_plan: cleanup,
    worktree_cleanup: { id: "c1" },
    ...overrides,
  };
  invoke.mockImplementation((command: string) =>
    command in table ? Promise.resolve(table[command]) : Promise.reject(new Error(command)),
  );
}

describe("IntegratePanel", () => {
  beforeEach(() => {
    invoke.mockReset();
    routes();
  });

  it("previews, then merges only against that preview after a confirmation", async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    render(<IntegratePanel onChanged={onChanged} taskId="task-1" taskTitle="Rate limit" worktreeReady />);
    await user.click(screen.getByRole("button", { name: "Preview the merge" }));
    expect(await screen.findByText("merges cleanly")).toBeVisible();
    // Fast-forward is not possible: the base moved.
    expect(screen.getByRole("radio", { name: /Fast-forward/ })).toBeDisabled();
    await user.click(screen.getByRole("radio", { name: /Squash/ }));

    await user.click(screen.getByRole("button", { name: "Merge into main" }));
    const confirm = screen.getByRole("alertdialog", { name: "Confirm merge" });
    expect(invoke).not.toHaveBeenCalledWith("integration_apply", expect.anything());
    await user.click(within(confirm).getByRole("button", { name: "Merge" }));
    expect(invoke).toHaveBeenCalledWith("integration_apply", {
      taskId: "task-1",
      fingerprint: "fp-1",
      strategy: "squash",
      message: "Rate limit",
    });
    expect(await screen.findByText(/landed as ccc/)).toBeVisible();
    expect(onChanged).toHaveBeenCalled();
  });

  it("names the files that would conflict and offers no merge", async () => {
    routes({ integration_preview: { ...preview, conflicts: ["src/ingest.ts"] } });
    const user = userEvent.setup();
    render(<IntegratePanel taskId="task-1" taskTitle="Rate limit" worktreeReady />);
    await user.click(screen.getByRole("button", { name: "Preview the merge" }));
    expect(await screen.findByText("1 conflict(s)")).toBeVisible();
    expect(screen.getByText("src/ingest.ts")).toBeVisible();
    expect(screen.queryByRole("button", { name: /Merge into/ })).not.toBeInTheDocument();
  });

  it("says why a merge is blocked", async () => {
    routes({ integration_preview: { ...preview, blockers: ["/repo has uncommitted changes; commit or stash them first"] } });
    const user = userEvent.setup();
    render(<IntegratePanel taskId="task-1" taskTitle="Rate limit" worktreeReady />);
    await user.click(screen.getByRole("button", { name: "Preview the merge" }));
    expect(await screen.findByText(/uncommitted changes/)).toBeVisible();
    expect(screen.queryByRole("button", { name: /Merge into/ })).not.toBeInTheDocument();
  });

  it("removes a worktree only after the loss is acknowledged", async () => {
    const user = userEvent.setup();
    render(<IntegratePanel taskId="task-1" taskTitle="Rate limit" worktreeReady />);
    await user.click(screen.getByRole("button", { name: /Remove this worktree/ }));
    expect(await screen.findByText(/cannot be brought back: scratch.txt/)).toBeVisible();
    const remove = screen.getByRole("button", { name: "Remove worktree" });
    expect(remove).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "Discard them" }));
    await user.click(remove);
    expect(invoke).toHaveBeenCalledWith("worktree_cleanup", {
      taskId: "task-1",
      fingerprint: "cleanup-1",
      discardUncommitted: true,
    });
  });

  it("offers to restore a removed worktree", async () => {
    routes({
      worktree_last_cleanup: {
        id: "c1",
        removedPath: "/w/rate-limit",
        branchName: "mars/rate-limit",
        archivedRef: "refs/mars/archive/mars/rate-limit",
        state: "removed",
      },
      worktree_restore: { id: "c1" },
    });
    const user = userEvent.setup();
    render(<IntegratePanel taskId="task-1" taskTitle="Rate limit" worktreeReady={false} />);
    await user.click(await screen.findByRole("button", { name: "Restore it" }));
    expect(invoke).toHaveBeenCalledWith("worktree_restore", { cleanupId: "c1" });
  });
});
