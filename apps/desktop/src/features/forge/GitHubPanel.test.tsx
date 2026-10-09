import type {
  ForgeIssue,
  ForgePullRequest,
  ForgePullRequestDetail,
  RemoteForge,
  WorktreeProposal,
} from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { GitHubPanel } from "./GitHubPanel";

const connected: RemoteForge = {
  name: "origin",
  url: "https://github.com/acme/api.git",
  host: "github.com",
  owner: "acme",
  repository: "api",
  provider: "github",
  connected: true,
  account: "octo",
  connection: {
    id: "c1",
    remoteId: "r1",
    provider: "github",
    apiBase: "https://api.github.com",
    accountLogin: "octo",
    credentialRef: "github:r1:octo",
  },
};

const issue: ForgeIssue = {
  providerId: "1",
  number: 12,
  title: "Rate limit the ingest endpoint",
  body: "It falls over under load.",
  state: "open",
  url: "https://github.com/acme/api/issues/12",
  author: "ana",
  labels: ["bug"],
  updatedAt: "2026-10-01T00:00:00Z",
};

const pull: ForgePullRequest = {
  providerId: "2",
  number: 30,
  title: "Limit ingest",
  body: "Adds a token bucket.",
  state: "open",
  url: "https://github.com/acme/api/pull/30",
  head: "mars/rate-limit",
  base: "main",
  draft: false,
  mergeable: true,
  headSha: "abcdef1234567",
  updatedAt: "2026-10-02T00:00:00Z",
};

const pullDetail: ForgePullRequestDetail = {
  pull,
  checks: [
    {
      providerId: "k1",
      name: "test",
      status: "completed",
      conclusion: "success",
      url: "https://github.com/acme/api/runs/1",
      startedAt: null,
      completedAt: null,
    },
  ],
  reviewComments: [],
  comments: [
    {
      providerId: "m1",
      author: "ana",
      body: "Looks good.",
      url: "https://github.com/acme/api/pull/30#c1",
      createdAt: "2026-10-02T01:00:00Z",
    },
  ],
};

function routes(overrides: Record<string, unknown> = {}) {
  const table: Record<string, unknown> = {
    forge_status: [connected],
    github_issues: [issue],
    github_pull_requests: [pull],
    github_issue: { issue, comments: [] },
    github_pull_request: pullDetail,
    github_comment: { providerId: "m2", author: "octo", body: "On it", url: "", createdAt: "" },
    github_merge: { merged: true, sha: "fff", message: "Pull Request successfully merged" },
    start_task_from_issue: { taskId: "task-9" } as WorktreeProposal,
    open_external_url: null,
    ...overrides,
  };
  invoke.mockImplementation((command: string) =>
    command in table ? Promise.resolve(table[command]) : Promise.reject(new Error(command)),
  );
}

describe("GitHubPanel", () => {
  beforeEach(() => {
    invoke.mockReset();
    routes();
  });

  it("says so when the repository has no GitHub remote", async () => {
    routes({ forge_status: [{ ...connected, provider: "unknown", connected: false }] });
    render(<GitHubPanel baseBranch="main" repositoryId="repo-1" />);
    expect(await screen.findByText("No GitHub remote")).toBeVisible();
  });

  it("offers the GitHub CLI sign-in or a token when nothing is connected", async () => {
    const user = userEvent.setup();
    let signedIn = false;
    invoke.mockImplementation((command: string) => {
      if (command === "forge_status") {
        return Promise.resolve([signedIn ? connected : { ...connected, connected: false, account: null, connection: null }]);
      }
      if (command === "connect_github_cli") {
        signedIn = true;
        return Promise.resolve(connected.connection);
      }
      if (command === "github_issues") return Promise.resolve([issue]);
      return Promise.reject(new Error(command));
    });
    render(<GitHubPanel baseBranch="main" repositoryId="repo-1" />);

    const connect = await screen.findByRole("region", { name: "Connect GitHub" });
    expect(within(connect).getByRole("button", { name: "Connect" })).toBeDisabled();
    await user.click(within(connect).getByRole("button", { name: "Use my GitHub CLI sign-in" }));

    expect(invoke).toHaveBeenCalledWith("connect_github_cli", { repositoryId: "repo-1", remoteName: "origin" });
    expect(await screen.findByText("as @octo")).toBeVisible();
    expect(await screen.findByText("Rate limit the ingest endpoint")).toBeVisible();
  });

  it("reads an issue and plans a task from it on the base branch", async () => {
    const user = userEvent.setup();
    const onTaskPlanned = vi.fn();
    render(<GitHubPanel baseBranch="main" onTaskPlanned={onTaskPlanned} repositoryId="repo-1" />);

    await user.click(await screen.findByRole("button", { name: /Rate limit the ingest endpoint/ }));
    expect(await screen.findByText("It falls over under load.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Plan a task from #12" }));

    expect(invoke).toHaveBeenCalledWith("start_task_from_issue", {
      repositoryId: "repo-1",
      remoteName: "origin",
      number: 12,
      baseRef: "main",
      expectedScope: [],
    });
    expect(onTaskPlanned).toHaveBeenCalledWith({ taskId: "task-9" });
  });

  it("posts a comment only after confirming who it is posted as", async () => {
    const user = userEvent.setup();
    render(<GitHubPanel baseBranch="main" repositoryId="repo-1" />);

    await user.click(await screen.findByRole("button", { name: /Rate limit the ingest endpoint/ }));
    await user.type(await screen.findByRole("textbox", { name: "Comment" }), "On it");
    await user.click(screen.getByRole("button", { name: "Comment…" }));

    const confirm = screen.getByRole("alertdialog", { name: "Confirm comment" });
    expect(confirm).toHaveTextContent("Post this publicly on #12 as @octo?");
    expect(invoke).not.toHaveBeenCalledWith("github_comment", expect.anything());
    await user.click(within(confirm).getByRole("button", { name: "Post" }));

    expect(invoke).toHaveBeenCalledWith("github_comment", {
      repositoryId: "repo-1",
      remoteName: "origin",
      number: 12,
      body: "On it",
    });
  });

  it("merges a pull request pinned to the head on screen, after confirming", async () => {
    const user = userEvent.setup();
    render(<GitHubPanel baseBranch="main" repositoryId="repo-1" />);

    await user.click(await screen.findByRole("tab", { name: "Pull requests" }));
    await user.click(await screen.findByRole("button", { name: /Limit ingest/ }));
    const merge = await screen.findByRole("region", { name: "Merge" });
    expect(screen.getByRole("region", { name: "Checks" })).toHaveTextContent("test");
    expect(screen.getByText("Looks good.")).toBeVisible();

    await user.selectOptions(within(merge).getByRole("combobox", { name: "Merge method" }), "rebase");
    await user.click(within(merge).getByRole("button", { name: "Merge…" }));
    const confirm = within(merge).getByRole("alertdialog", { name: "Confirm merge" });
    expect(confirm).toHaveTextContent("Merge #30 into main on GitHub as @octo, at abcdef1?");
    await user.click(within(confirm).getByRole("button", { name: "Merge" }));

    expect(invoke).toHaveBeenCalledWith("github_merge", {
      repositoryId: "repo-1",
      remoteName: "origin",
      number: 30,
      method: "rebase",
      expectedHead: "abcdef1234567",
    });
    expect(await screen.findByRole("status")).toHaveTextContent("Merged #30");
  });

  it("will not offer to merge a draft", async () => {
    const user = userEvent.setup();
    routes({ github_pull_request: { ...pullDetail, pull: { ...pull, draft: true } } });
    render(<GitHubPanel baseBranch="main" repositoryId="repo-1" />);

    await user.click(await screen.findByRole("tab", { name: "Pull requests" }));
    await user.click(await screen.findByRole("button", { name: /Limit ingest/ }));
    const merge = await screen.findByRole("region", { name: "Merge" });
    expect(within(merge).getByRole("button", { name: "Merge…" })).toBeDisabled();
  });
});
