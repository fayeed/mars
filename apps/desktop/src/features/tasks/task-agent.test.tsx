import type { AgentTimelinePayload, ContextPack, WorktreePlan } from "@mars/shared-types";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TaskWorkspace } from "./TaskWorkspace";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listener: null as ((event: { payload: AgentTimelinePayload }) => void) | null,
  unlisten: vi.fn(),
}));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (_name: string, listener: (event: { payload: AgentTimelinePayload }) => void) => {
      mocks.listener = listener;
      return Promise.resolve(mocks.unlisten);
    },
  ),
}));

const plannedTask: WorktreePlan = {
  taskId: "task-1",
  repositoryId: "repository-1",
  title: "Repair parser",
  intent: "Make malformed records recoverable",
  branchName: "task/repair-parser",
  baseRef: "HEAD",
  worktreePath: "/worktrees/repair-parser",
  state: "planned",
};

const readyTask = { ...plannedTask, state: "ready" } satisfies WorktreePlan;

const contextPack: ContextPack = {
  taskId: readyTask.taskId,
  adapter: "claude",
  maxBytes: 24 * 1024,
  totalBytes: 184,
  sectionUsage: [
    { section: "instructions", maxBytes: 4915, usedBytes: 184 },
    { section: "context", maxBytes: 6144, usedBytes: 0 },
    { section: "code", maxBytes: 8601, usedBytes: 0 },
    { section: "history", maxBytes: 2457, usedBytes: 0 },
    { section: "skills", maxBytes: 2457, usedBytes: 0 },
  ],
  items: [
    {
      section: "instructions",
      entityRef: "context:constraint-1",
      title: "Parser constraint",
      content: "Malformed records must remain recoverable.",
      provenance: "HUMAN",
      freshness: "current task attachment",
      sourceLocation: "context:constraint-1",
      reason: "active task constraint",
      score: 1,
      byteSize: 184,
    },
  ],
  exclusions: [],
};

afterEach(() => {
  vi.clearAllMocks();
  mocks.listener = null;
});

describe("task agent workflow", () => {
  it("creates a task, approves its exact worktree plan, and opens the worktree", async () => {
    const user = userEvent.setup();
    const onOpenWorktree = vi.fn();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_tasks") return Promise.resolve([]);
      if (command === "create_task") return Promise.resolve(plannedTask);
      if (command === "provision_task") return Promise.resolve(readyTask);
      if (command === "search_context") return Promise.resolve([]);
      if (command === "list_task_skills") return Promise.resolve([]);
      if (command === "build_context_pack") return Promise.resolve(contextPack);
      if (command === "open_task_worktree") {
        return Promise.resolve({
          status: "opened",
          repository: {
            projectId: "project-task",
            repositoryId: "repository-task",
            name: "repair-parser",
            rootPath: readyTask.worktreePath,
            gitCommonDir: "/repo/.git",
          },
        });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<TaskWorkspace onOpenWorktree={onOpenWorktree} repositoryId="repository-1" />);

    await user.click(await screen.findByRole("button", { name: "New task" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Title"), plannedTask.title);
    await user.type(within(dialog).getByLabelText("Intent"), plannedTask.intent);
    await user.click(within(dialog).getByRole("button", { name: "Plan task" }));

    expect(await screen.findByText(plannedTask.worktreePath)).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith("create_task", {
      repositoryId: "repository-1",
      title: plannedTask.title,
      intent: plannedTask.intent,
      baseRef: "HEAD",
    });

    await user.click(screen.getByRole("button", { name: "Approve worktree plan" }));
    expect(await screen.findByRole("button", { name: /Open worktree/ })).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith("provision_task", { taskId: plannedTask.taskId });

    await user.click(screen.getByRole("button", { name: /Open worktree/ }));
    expect(onOpenWorktree).toHaveBeenCalledWith(
      expect.objectContaining({ rootPath: readyTask.worktreePath }),
    );
  });

  it("keeps adapter probe failures visible and does not ask for permissions", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_tasks") return Promise.resolve([readyTask]);
      if (command === "search_context") return Promise.resolve([]);
      if (command === "list_task_skills") return Promise.resolve([]);
      if (command === "build_context_pack") return Promise.resolve(contextPack);
      if (command === "probe_agent") {
        return Promise.reject({ message: "claude executable was not found" });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<TaskWorkspace onOpenWorktree={vi.fn()} repositoryId="repository-1" />);

    await user.click(await screen.findByRole("button", { name: "Review permissions" }));
    const packDialog = await screen.findByRole("dialog");
    expect(within(packDialog).getByText("Parser constraint")).toBeVisible();
    await user.click(
      within(packDialog).getByRole("button", { name: "Continue to permissions" }),
    );

    expect(await screen.findByRole("alert")).toHaveTextContent("claude executable was not found");
    expect(screen.queryByText("Approve agent capabilities")).not.toBeInTheDocument();
    expect(mocks.invoke).not.toHaveBeenCalledWith("launch_agent", expect.anything());
  });

  it("streams events, cancels, reports crashes, and resumes the provider session", async () => {
    const user = userEvent.setup();
    let launchCount = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_tasks") return Promise.resolve([readyTask]);
      if (command === "search_context") return Promise.resolve([]);
      if (command === "list_task_skills") return Promise.resolve([]);
      if (command === "build_context_pack") return Promise.resolve(contextPack);
      if (command === "probe_agent") {
        return Promise.resolve({
          kind: "claude",
          executable: "claude",
          version: "2.1.247",
          supportsResume: true,
        });
      }
      if (command === "launch_agent") {
        launchCount += 1;
        return Promise.resolve({
          sessionId: `local-session-${launchCount}`,
          adapter: "claude",
          version: "2.1.247",
        });
      }
      if (command === "cancel_agent") return Promise.resolve(undefined);
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<TaskWorkspace onOpenWorktree={vi.fn()} repositoryId="repository-1" />);

    await user.click(await screen.findByRole("button", { name: "Review permissions" }));
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "Continue to permissions",
      }),
    );
    const permissionDialog = await screen.findByRole("dialog");
    expect(within(permissionDialog).getByText("Write worktree files")).toBeVisible();
    await user.click(within(permissionDialog).getByRole("button", { name: "Approve and launch" }));

    expect(await screen.findByRole("button", { name: "Cancel agent" })).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith(
      "launch_agent",
      expect.objectContaining({
        request: expect.objectContaining({
          taskId: readyTask.taskId,
          resumeSessionId: null,
          permissions: { writeFiles: true, runCommands: true, network: false },
          contextPack,
        }),
      }),
    );

    emit({ type: "checkpoint", sessionId: "provider-session-1" });
    emit({ type: "command", command: "pnpm test", status: "running" });
    expect(await screen.findByText("provider-session-1")).toBeVisible();
    expect(screen.getByText("pnpm test")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Cancel agent" }));
    expect(mocks.invoke).toHaveBeenCalledWith("cancel_agent", {
      sessionId: "local-session-1",
    });

    emit({ type: "end", success: false, message: "Agent process crashed" });
    expect(await screen.findByText("Agent process crashed")).toBeVisible();
    expect(screen.getByText("failed")).toBeVisible();

    await user.type(screen.getByLabelText("Follow-up"), "Recover and rerun the tests");
    await user.click(screen.getByRole("button", { name: "Review follow-up" }));
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "Continue to permissions",
      }),
    );
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "Approve and launch",
      }),
    );

    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "launch_agent",
      expect.objectContaining({
        request: expect.objectContaining({
          prompt: "Recover and rerun the tests",
          resumeSessionId: "provider-session-1",
        }),
      }),
    );
  });
});

function emit(event: AgentTimelinePayload["event"]) {
  act(() => {
    mocks.listener?.({ payload: { sessionId: "local-session-1", event } });
  });
}
