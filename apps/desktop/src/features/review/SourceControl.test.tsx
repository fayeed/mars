import type { RepoStatus, WorktreePlan } from "@mars/shared-types";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { SourceControl } from "./SourceControl";
import { worktreeActivityState } from "@/features/workspace/ShellWorkspace";

const tasks: WorktreePlan[] = [
  {
    taskId: "task-1",
    repositoryId: "repository-1",
    title: "Rate limit ingest",
    intent: "",
    branchName: "mars/rate-limit",
    baseRef: "main",
    worktreePath: "/data/worktrees/rate-limit",
    state: "ready",
  },
  {
    taskId: "task-2",
    repositoryId: "repository-1",
    title: "Split the broker",
    intent: "",
    branchName: "mars/broker",
    baseRef: "main",
    worktreePath: "/data/worktrees/broker",
    state: "failed",
  },
];

const clean: RepoStatus = {
  root: "/repo",
  branch: "main",
  headOid: "abc",
  upstream: null,
  ahead: 0,
  behind: 0,
  changes: [],
  operation: null,
};

function renderSourceControl(props: Partial<Parameters<typeof SourceControl>[0]> = {}) {
  return render(
    <SourceControl
      baseBranch="main"
      diff={<p>the diff</p>}
      repositoryId="repository-1"
      repositoryName="mars"
      onSelectTask={vi.fn()}
      selectedTaskId="task-1"
      tasks={tasks}
      worktreeState={worktreeActivityState}
      {...props}
    />,
  );
}

describe("SourceControl", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((command: string) => {
      switch (command) {
        case "git_status":
          return Promise.resolve(clean);
        case "git_remotes":
        case "git_branches":
        case "git_log":
          return Promise.resolve([]);
        case "worktree_last_cleanup":
          return Promise.resolve(null);
        default:
          return Promise.reject(new Error(command));
      }
    });
  });

  it("lists the repository and every worktree with its branch, base and state", () => {
    renderSourceControl();

    expect(screen.getByRole("button", { name: /Repository/ })).toBeVisible();
    expect(screen.getAllByText("from main")).toHaveLength(2);
    expect(screen.getByText("failed")).toBeVisible();
  });

  it("shows the task's review first, with Git and Merge beside it", async () => {
    const user = userEvent.setup();
    renderSourceControl();

    expect(screen.getByText("the diff")).toBeVisible();
    const tabs = screen.getByRole("tablist", { name: "Worktree views" });
    await user.click(within(tabs).getByRole("tab", { name: "Git" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_status", {
        target: { kind: "task", taskId: "task-1" },
      }),
    );
    await user.click(within(tabs).getByRole("tab", { name: "Merge" }));
    expect(screen.getByRole("button", { name: "Preview the merge" })).toBeVisible();
  });

  it("opens the repository's own Git when the repository is chosen", async () => {
    const user = userEvent.setup();
    renderSourceControl();
    await user.click(screen.getByRole("button", { name: /Repository/ }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("git_status", {
        target: { kind: "repository", repositoryId: "repository-1" },
      }),
    );
    expect(await screen.findByText("Nothing changed")).toBeVisible();
  });

  it("switches worktree", async () => {
    const user = userEvent.setup();
    const onSelectTask = vi.fn();
    renderSourceControl({ onSelectTask });

    await user.click(screen.getByRole("button", { name: /Split the broker/ }));
    expect(onSelectTask).toHaveBeenCalledWith("task-2");
  });

  it("opens GitHub beside the repository and its worktrees", async () => {
    const user = userEvent.setup();
    renderSourceControl();
    await user.click(screen.getByRole("button", { name: /Issues and pull requests/ }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("forge_status", { repositoryId: "repository-1" }),
    );
  });

  it("can open on GitHub when another screen asks for it", async () => {
    renderSourceControl({ initialScope: "github" });

    expect(screen.getByRole("button", { name: /Issues and pull requests/ })).toHaveAttribute(
      "aria-current",
      "true",
    );
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("forge_status", { repositoryId: "repository-1" }),
    );
  });

  it("explains an empty list rather than showing an empty rail", () => {
    renderSourceControl({ tasks: [] });

    expect(screen.getByText(/Each one gets its own branch and its own checkout/)).toBeVisible();
  });
});
