import type { StoredIssue, WorktreePlan } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { ImportGitHubIssueDialog } from "./ImportGitHubIssueDialog";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const issue: StoredIssue = {
  id: "issue-local-1",
  remoteId: "remote-1",
  repositoryId: "repository-1",
  providerId: "101",
  number: 7,
  title: "Fix sync",
  body: "Preserve local state",
  state: "open",
  url: "https://github.com/acme/mars/issues/7",
  author: "faye",
  labels: ["bug"],
  updatedAt: "2026-08-30T00:00:00Z",
};

const task: WorktreePlan = {
  taskId: "task-1",
  repositoryId: issue.repositoryId,
  title: issue.title,
  intent: issue.body,
  branchName: "mars/fix-sync",
  baseRef: "main",
  worktreePath: "/tmp/fix-sync",
  state: "planned",
};

afterEach(() => vi.clearAllMocks());

it("inspects a GitHub issue before creating its linked task plan", async () => {
  const user = userEvent.setup();
  const onCreated = vi.fn();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "connect_github") return Promise.resolve({
      id: "connection-1",
      remoteId: issue.remoteId,
      provider: "github",
      apiBase: "https://api.github.com",
      accountLogin: "faye",
      credentialRef: "github:remote-1:faye",
    });
    if (command === "import_github_issue") return Promise.resolve(issue);
    if (command === "create_task_from_issue") return Promise.resolve(task);
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });
  render(<ImportGitHubIssueDialog onCreated={onCreated} repositoryId="repository-1" />);

  await user.click(screen.getByRole("button", { name: "Import issue" }));
  const dialog = screen.getByRole("dialog");
  await user.type(within(dialog).getByLabelText("Issue number"), "7");
  await user.type(within(dialog).getByLabelText("GitHub access token"), "secret-token");
  await user.click(within(dialog).getByRole("button", { name: "Connect and inspect issue" }));
  expect(await within(dialog).findByText(issue.title)).toBeVisible();
  await user.click(within(dialog).getByRole("button", { name: "Create task plan" }));

  expect(mocks.invoke).toHaveBeenNthCalledWith(1, "connect_github", {
    repositoryId: "repository-1",
    remoteName: "origin",
    token: "secret-token",
  });
  expect(mocks.invoke).toHaveBeenNthCalledWith(2, "import_github_issue", {
    repositoryId: "repository-1",
    remoteName: "origin",
    issueNumber: 7,
  });
  expect(mocks.invoke).toHaveBeenNthCalledWith(3, "create_task_from_issue", {
    issueId: issue.id,
    baseRef: "main",
  });
  expect(onCreated).toHaveBeenCalledWith(task);
});
