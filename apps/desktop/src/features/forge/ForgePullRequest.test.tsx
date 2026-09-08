import type { ForgeMutation, PullRequestPlan, PullRequestSnapshot, WorktreePlan } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { ForgePullRequest } from "./ForgePullRequest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const task: WorktreePlan = {
  taskId: "task-1",
  repositoryId: "repository-1",
  title: "Fix sync",
  intent: "Preserve local state",
  branchName: "mars/fix-sync",
  baseRef: "main",
  worktreePath: "/tmp/fix-sync",
  state: "ready",
};

const plan: PullRequestPlan = {
  taskId: task.taskId,
  remoteId: "remote-1",
  remoteName: "origin",
  remoteUrl: "git@github.com:acme/mars.git",
  pushUrl: "git@github.com:acme/mars.git",
  repository: { host: "github.com", owner: "acme", name: "mars" },
  worktreePath: task.worktreePath,
  head: task.branchName,
  base: "main",
  headSha: "a".repeat(40),
  commitsAhead: 2,
  clean: true,
  title: task.title,
  body: task.intent,
  draft: false,
  reviewers: [],
  pushCommand: "git push git@github.com:acme/mars.git HEAD:refs/heads/mars/fix-sync",
  fingerprint: "f".repeat(64),
};

const snapshot: PullRequestSnapshot = {
  id: "pr-local-1",
  remoteId: plan.remoteId,
  taskId: task.taskId,
  issueId: null,
  providerId: "201",
  number: 9,
  title: task.title,
  body: task.intent,
  state: "open",
  url: "https://github.com/acme/mars/pull/9",
  head: task.branchName,
  base: "main",
  draft: false,
  mergeable: true,
  headSha: plan.headSha,
  updatedAt: "2026-08-30T00:00:00Z",
  checks: [{
    providerId: "check-1",
    name: "test",
    status: "completed",
    conclusion: "success",
    url: "https://github.com/check/1",
    startedAt: null,
    completedAt: null,
  }],
  comments: [{
    providerId: "comment-1",
    author: "reviewer",
    body: "Looks good",
    path: "src/sync.ts",
    line: 12,
    url: "https://github.com/comment/1",
    createdAt: "2026-08-30T00:00:00Z",
  }],
};

const interruptedMutation: ForgeMutation = {
  id: "mutation-1",
  taskId: task.taskId,
  remoteId: plan.remoteId,
  operation: "create_pull_request",
  status: "interrupted",
  planFingerprint: plan.fingerprint,
  pushApproved: true,
  forgeApproved: true,
  detailsJson: JSON.stringify({ stage: "approved" }),
  error: "application stopped before confirmation",
  startedAt: "2026-08-30T00:00:00Z",
  finishedAt: "2026-08-30T00:01:00Z",
};

afterEach(() => vi.clearAllMocks());

it("requires review and explicit approval before publishing the PR", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "get_task_pull_request") return Promise.resolve(null);
    if (command === "list_task_forge_mutations") return Promise.resolve([]);
    if (command === "preview_pull_request") return Promise.resolve(plan);
    if (command === "connect_github") return Promise.resolve({
      id: "connection-1",
      remoteId: plan.remoteId,
      provider: "github",
      apiBase: "https://api.github.com",
      accountLogin: "faye",
      credentialRef: "github:remote-1:faye",
    });
    if (command === "create_github_pull_request") return Promise.resolve(snapshot);
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });
  render(<ForgePullRequest task={task} />);

  await user.click(screen.getByRole("button", { name: "Preview push and PR" }));
  expect(await screen.findByText(plan.pushCommand)).toBeVisible();
  const publish = screen.getByRole("button", { name: "Push branch and create PR" });
  expect(publish).toBeDisabled();
  await user.type(screen.getByLabelText("GitHub access token"), "secret-token");
  await user.click(screen.getByRole("button", { name: "Save to OS keychain" }));
  expect(await screen.findByText("Connected as faye")).toBeVisible();
  await user.click(screen.getByText("I approve pushing this exact commit to the displayed push target and branch."));
  expect(publish).toBeDisabled();
  await user.click(screen.getByText("I separately approve creating this GitHub pull request with the displayed title, body, head, and base."));
  expect(publish).toBeEnabled();
  await user.click(publish);

  expect(mocks.invoke).toHaveBeenCalledWith("create_github_pull_request", {
    plan,
    pushApproved: true,
    forgeApproved: true,
  });
  expect(await screen.findByText("test · success")).toBeVisible();
  expect(screen.getByText("Looks good")).toBeVisible();
});

it("surfaces interrupted publish attempts with recovery evidence", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "get_task_pull_request") return Promise.resolve(null);
    if (command === "list_task_forge_mutations") return Promise.resolve([interruptedMutation]);
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });

  render(<ForgePullRequest task={task} />);

  expect(await screen.findByText("interrupted")).toBeVisible();
  expect(screen.getByText("application stopped before confirmation")).toBeVisible();
  expect(screen.getByText(`Plan ${plan.fingerprint.slice(0, 12)}`)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Refresh publish history" }));
  expect(mocks.invoke).toHaveBeenCalledWith("list_task_forge_mutations", { taskId: task.taskId });
});
