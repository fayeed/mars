import type { AgentAccount, ContextPack, WorktreePlan } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TaskDetail } from "./TaskDetail";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}));

const task: WorktreePlan = {
  taskId: "task-1",
  repositoryId: "repository-1",
  title: "Repair parser",
  intent: "Make malformed records recoverable",
  branchName: "task/repair-parser",
  baseRef: "HEAD",
  worktreePath: "/worktrees/repair-parser",
  state: "ready",
};

const contextPack: ContextPack = {
  taskId: task.taskId,
  adapter: "claude",
  maxBytes: 24 * 1024,
  totalBytes: 0,
  sectionUsage: [],
  items: [],
  exclusions: [],
};

const plan: AgentAccount = {
  kind: "claude",
  billing: { type: "plan", plan: "team" },
  signIn: { type: "plan", plan: "team" },
  overrides: [],
  useSignIn: false,
  signInCommand: "claude auth login",
};

const signedOut: AgentAccount = {
  ...plan,
  billing: { type: "signedOut" },
  signIn: { type: "signedOut" },
};

const keyOverPlan: AgentAccount = {
  ...plan,
  billing: { type: "apiKey", source: "ANTHROPIC_API_KEY" },
  overrides: ["ANTHROPIC_API_KEY"],
};

afterEach(() => {
  vi.clearAllMocks();
});

function answer(responses: Record<string, (args: Record<string, unknown>) => unknown>) {
  mocks.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    if (command === "search_context" || command === "list_task_skills") return Promise.resolve([]);
    if (command === "build_context_pack") return Promise.resolve(contextPack);
    if (command === "probe_agent") {
      return Promise.resolve({
        kind: "claude",
        executable: "claude",
        version: "2.1.292",
        supportsResume: true,
      });
    }
    const respond = responses[command];
    if (respond) {
      try {
        return Promise.resolve(respond(args));
      } catch (caught) {
        return Promise.reject(caught);
      }
    }
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });
}

async function openPermissions(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: "Review permissions" }));
  await user.click(
    within(await screen.findByRole("dialog")).getByRole("button", {
      name: "Continue to permissions",
    }),
  );
  return screen.findByRole("dialog", { name: "Approve agent capabilities" });
}

describe("signing in to the agent before a run", () => {
  it("will not approve a run for a signed-out agent, and offers its own sign-in", async () => {
    const user = userEvent.setup();
    const onSignIn = vi.fn();
    let current = signedOut;
    answer({ agent_account: () => current });
    render(
      <TaskDetail
        onOpenWorktree={vi.fn()}
        onSignIn={onSignIn}
        onTaskChanged={vi.fn()}
        task={task}
      />,
    );

    const dialog = await openPermissions(user);
    expect(within(dialog).getByText("Sign in to the agent first")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Approve and launch" })).toBeDisabled();

    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));
    expect(onSignIn).toHaveBeenCalledWith("claude");

    current = plan;
    await user.click(within(dialog).getByRole("button", { name: "Check again" }));
    expect(await within(dialog).findByText("your Claude Team plan")).toBeVisible();
    expect(within(dialog).getByRole("button", { name: "Approve and launch" })).toBeEnabled();
  });

  it("asks before a key in the environment bills a run, and remembers the answer", async () => {
    const user = userEvent.setup();
    answer({
      agent_account: () => keyOverPlan,
      set_agent_sign_in_preference: ({ useSignIn }) => ({
        ...keyOverPlan,
        useSignIn,
        billing: useSignIn ? plan.billing : keyOverPlan.billing,
      }),
    });
    render(<TaskDetail onOpenWorktree={vi.fn()} onTaskChanged={vi.fn()} task={task} />);

    const dialog = await openPermissions(user);
    expect(within(dialog).getByText("the API key in ANTHROPIC_API_KEY")).toBeVisible();

    await user.click(
      within(dialog).getByRole("checkbox", { name: "Use your Claude Team plan instead" }),
    );
    expect(mocks.invoke).toHaveBeenCalledWith("set_agent_sign_in_preference", {
      adapter: "claude",
      useSignIn: true,
    });
    expect(await within(dialog).findByText("your Claude Team plan")).toBeVisible();
  });

  it("turns a signed-out refusal at launch into a sign-in", async () => {
    const user = userEvent.setup();
    const onSignIn = vi.fn();
    answer({
      // The account could not be read beforehand, so the launch is what finds out.
      agent_account: () => {
        throw new Error("status unavailable");
      },
      launch_agent: () => {
        throw {
          code: "agent.signed_out",
          message:
            "Claude Code is installed but not signed in. Run `claude auth login` in a terminal to sign in with your own account.",
        };
      },
    });
    render(
      <TaskDetail
        onOpenWorktree={vi.fn()}
        onSignIn={onSignIn}
        onTaskChanged={vi.fn()}
        task={task}
      />,
    );

    const dialog = await openPermissions(user);
    await user.click(within(dialog).getByRole("button", { name: "Approve and launch" }));

    // Other panels on this view raise alerts of their own; this is the launch's.
    const title = await screen.findByText("The agent is not signed in");
    const alert = title.closest<HTMLElement>('[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert).toHaveTextContent("claude auth login");
    await user.click(within(alert!).getByRole("button", { name: "Sign in" }));
    expect(onSignIn).toHaveBeenCalledWith("claude");
  });
});
