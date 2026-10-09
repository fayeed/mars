import type { CommitInfo, RepoStatus } from "@mars/shared-types";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { GitPanel, groupChanges } from "./GitPanel";

const target = { kind: "repository" as const, repositoryId: "repo" };

const dirty: RepoStatus = {
  root: "/repo",
  branch: "main",
  headOid: "abc123",
  upstream: "origin/main",
  ahead: 2,
  behind: 1,
  operation: null,
  changes: [
    { path: "src/app.ts", originalPath: null, staged: "M", unstaged: ".", untracked: false, conflicted: false },
    { path: "src/lib.ts", originalPath: null, staged: ".", unstaged: "M", untracked: false, conflicted: false },
    { path: "notes.md", originalPath: null, staged: ".", unstaged: "?", untracked: true, conflicted: false },
  ],
};

const commit: CommitInfo = {
  oid: "abc1234567",
  shortOid: "abc1234",
  subject: "Add rate limiting",
  author: "Fayeed",
  authoredAt: "2026-10-09T10:00:00Z",
  refs: ["main"],
};

const DIFF = "diff --git a/src/lib.ts b/src/lib.ts\n--- a/src/lib.ts\n+++ b/src/lib.ts\n@@ -1 +1 @@\n-old\n+new\n";

function routes(status: RepoStatus = dirty) {
  invoke.mockImplementation((command: string) => {
    switch (command) {
      case "git_status":
      case "git_stage":
      case "git_unstage":
      case "git_discard":
      case "git_push":
      case "git_pull":
      case "git_fetch":
      case "git_switch":
      case "git_create_branch":
        return Promise.resolve(status);
      case "git_remotes":
        return Promise.resolve(["origin"]);
      case "git_branches":
        return Promise.resolve([
          { name: "main", current: true, upstream: "origin/main", oid: "a", subject: "x", committedAt: "", checkedOutElsewhere: false },
          { name: "mars/task", current: false, upstream: null, oid: "b", subject: "y", committedAt: "", checkedOutElsewhere: true },
        ]);
      case "git_diff":
      case "git_show":
        return Promise.resolve(DIFF);
      case "git_log":
        return Promise.resolve([commit]);
      case "git_commit":
        return Promise.resolve(commit);
      default:
        return Promise.reject(new Error(command));
    }
  });
}

describe("GitPanel", () => {
  beforeEach(() => {
    invoke.mockReset();
    routes();
  });

  it("groups changes the way a person works through them", () => {
    const groups = groupChanges(dirty.changes);
    expect(groups.staged.map((change) => change.path)).toEqual(["src/app.ts"]);
    expect(groups.unstaged.map((change) => change.path)).toEqual(["src/lib.ts"]);
    expect(groups.untracked.map((change) => change.path)).toEqual(["notes.md"]);
  });

  it("shows where the branch stands and stages exactly the file named", async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    render(<GitPanel onChanged={onChanged} target={target} />);
    expect(await screen.findByText("origin/main · ↑2 ↓1")).toBeVisible();
    // A branch checked out in a task's worktree cannot be switched to here.
    expect(screen.getByRole("option", { name: /mars\/task \(in a worktree\)/ })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Stage src/lib.ts" }));
    expect(invoke).toHaveBeenCalledWith("git_stage", { target, paths: ["src/lib.ts"] });
    expect(onChanged).toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "src/lib.ts, modified" }));
    expect(await screen.findByText("new")).toBeVisible();
    expect(invoke).toHaveBeenCalledWith("git_diff", { target, path: "src/lib.ts", staged: false });
  });

  it("asks before throwing changes away", async () => {
    const user = userEvent.setup();
    render(<GitPanel target={target} />);
    await user.click(await screen.findByRole("button", { name: "Discard src/lib.ts" }));
    const dialog = screen.getByRole("alertdialog", { name: "Confirm discard" });
    expect(invoke).not.toHaveBeenCalledWith("git_discard", expect.anything());
    await user.click(within(dialog).getByRole("button", { name: "Discard" }));
    expect(invoke).toHaveBeenCalledWith("git_discard", { target, paths: ["src/lib.ts"] });
  });

  it("commits what is staged with a message, and only then", async () => {
    const user = userEvent.setup();
    render(<GitPanel target={target} />);
    const commitButton = await screen.findByRole("button", { name: "Commit" });
    expect(commitButton).toBeDisabled();
    await user.type(screen.getByRole("textbox", { name: "Commit message" }), "Add rate limiting");
    expect(commitButton).toBeEnabled();
    await user.click(commitButton);
    expect(invoke).toHaveBeenCalledWith("git_commit", {
      target,
      message: "Add rate limiting",
      amend: false,
    });
  });

  it("refuses to commit with nothing staged", async () => {
    routes({ ...dirty, changes: dirty.changes.filter((change) => change.staged === ".") });
    const user = userEvent.setup();
    render(<GitPanel target={target} />);
    await user.type(await screen.findByRole("textbox", { name: "Commit message" }), "x");
    expect(screen.getByRole("button", { name: "Commit" })).toBeDisabled();
  });

  it("names the remote and branch before publishing", async () => {
    const user = userEvent.setup();
    render(<GitPanel target={target} />);
    await user.click(await screen.findByRole("button", { name: "Push" }));
    const dialog = screen.getByRole("alertdialog", { name: "Confirm push" });
    expect(dialog).toHaveTextContent("Publish main to origin");
    await user.click(within(dialog).getByRole("button", { name: "Push" }));
    expect(invoke).toHaveBeenCalledWith("git_push", { target, remote: "origin", branch: "main" });
  });

  it("shows history and what a commit changed", async () => {
    const user = userEvent.setup();
    render(<GitPanel target={target} />);
    await user.click(await screen.findByRole("tab", { name: "History" }));
    const history = await screen.findByRole("list", { name: "History" });
    await user.click(within(history).getByRole("button", { name: /Add rate limiting/ }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("git_show", { target, oid: "abc1234567" }));
  });
});
