import type { WorktreePlan } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

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

function renderSourceControl(props: Partial<Parameters<typeof SourceControl>[0]> = {}) {
  return render(
    <SourceControl
      diff={<p>the diff</p>}
      onSelectTask={vi.fn()}
      selectedTaskId="task-1"
      tasks={tasks}
      worktreeState={worktreeActivityState}
      {...props}
    />,
  );
}

describe("SourceControl", () => {
  it("lists every worktree with its branch, base and state", () => {
    renderSourceControl();

    expect(screen.getByText("mars/rate-limit")).toBeVisible();
    expect(screen.getAllByText("from main")).toHaveLength(2);
    expect(screen.getByText("failed")).toBeVisible();
  });

  it("shows the change for the selected worktree beside the list", () => {
    renderSourceControl();

    expect(screen.getByText("the diff")).toBeVisible();
  });

  it("switches worktree", async () => {
    const user = userEvent.setup();
    const onSelectTask = vi.fn();
    renderSourceControl({ onSelectTask });

    await user.click(screen.getByRole("button", { name: /Split the broker/ }));
    expect(onSelectTask).toHaveBeenCalledWith("task-2");
  });

  it("explains an empty list rather than showing an empty rail", () => {
    renderSourceControl({ tasks: [] });

    expect(screen.getByText(/Each one gets its own branch and its own checkout/)).toBeVisible();
  });

  it("offers no staging control, because nothing stages", () => {
    renderSourceControl();

    expect(screen.queryByRole("button", { name: /stage/i })).not.toBeInTheDocument();
  });
});
