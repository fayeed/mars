import type { PlannedCheck, PullRequestPlan, TaskCommit } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PublishPreview } from "./PublishPreview";

const plan: PullRequestPlan = {
  taskId: "task-1",
  remoteId: "remote-1",
  remoteName: "origin",
  remoteUrl: "git@github.com:acme/mars.git",
  pushUrl: "git@github.com:acme/mars.git",
  repository: { host: "github.com", owner: "acme", name: "mars" },
  worktreePath: "/data/worktrees/rate-limit",
  head: "mars/rate-limit",
  base: "main",
  headSha: "511c628aa9c1d4f0",
  commitsAhead: 2,
  clean: true,
  title: "Rate limit the ingest endpoint",
  body: "Adds a per-token limiter.",
  draft: false,
  reviewers: [],
  pushCommand: "git push git@github.com:acme/mars.git HEAD:refs/heads/mars/rate-limit",
  fingerprint: "abcdef0123456789",
};

const commits: TaskCommit[] = [
  { oid: "511c628aaaa", subject: "Add the limiter", author: "fayeed", date: "2026-09-08" },
  { oid: "463d9cfbbbb", subject: "Wire it into accept", author: "fayeed", date: "2026-09-08" },
];

const checks: PlannedCheck[] = [
  { path: ".github/workflows/ci.yml", name: "CI", onPullRequest: true },
  { path: ".github/workflows/release.yml", name: "Release", onPullRequest: false },
];

function renderPreview(props: Partial<Parameters<typeof PublishPreview>[0]> = {}) {
  return render(
    <PublishPreview
      checks={checks}
      commits={commits}
      onPublish={vi.fn()}
      onRevise={vi.fn()}
      plan={plan}
      state="idle"
      {...props}
    />,
  );
}

describe("PublishPreview: the exact push", () => {
  it("shows the command that will run, not a description of it", () => {
    renderPreview();

    expect(screen.getByText(plan.pushCommand)).toBeVisible();
    expect(screen.getByText("mars/rate-limit")).toBeVisible();
    expect(screen.getByText("main")).toBeVisible();
  });

  it("lists the commits the push would carry", () => {
    renderPreview();

    expect(screen.getByText("Add the limiter")).toBeVisible();
    expect(screen.getByText("Wire it into accept")).toBeVisible();
  });

  it("says uncommitted work is not part of the push", () => {
    renderPreview({ plan: { ...plan, clean: false } });

    expect(screen.getByText(/They are not part of this push/)).toBeVisible();
  });

  it("says nothing has been pushed yet", () => {
    renderPreview();

    expect(screen.getByText("nothing has been pushed")).toBeVisible();
  });
});

describe("PublishPreview: the exact pull request", () => {
  it("makes the title and body editable", async () => {
    const user = userEvent.setup();
    const onRevise = vi.fn();
    renderPreview({ onRevise });

    const title = screen.getByRole("textbox", { name: "Pull request title" });
    await user.clear(title);
    await user.type(title, "Rate limit ingest by token");
    await user.click(screen.getByRole("button", { name: /Re-check this plan/ }));

    expect(onRevise).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Rate limit ingest by token" }),
    );
  });

  it("re-checks rather than publishing while the plan is edited", async () => {
    const user = userEvent.setup();
    const onPublish = vi.fn();
    renderPreview({ onPublish });

    await user.type(screen.getByRole("textbox", { name: "Pull request title" }), "!");

    // An edited plan cannot be published: its fingerprint no longer matches.
    expect(screen.queryByRole("button", { name: /Push and open/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Re-check this plan/ })).toBeVisible();
    expect(onPublish).not.toHaveBeenCalled();
  });

  it("collects reviewers and reports them as an edit to the plan", async () => {
    const user = userEvent.setup();
    const onRevise = vi.fn();
    renderPreview({ onRevise });

    expect(screen.getByText("Nobody will be asked to review.")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Add a reviewer" }), "@sam{Enter}");
    await user.click(screen.getByRole("button", { name: /Re-check this plan/ }));

    expect(onRevise).toHaveBeenCalledWith(expect.objectContaining({ reviewers: ["sam"] }));
  });

  it("removes a reviewer", async () => {
    const user = userEvent.setup();
    renderPreview({ plan: { ...plan, reviewers: ["sam"] } });

    await user.click(screen.getByRole("button", { name: "Remove sam" }));
    expect(screen.getByText("Nobody will be asked to review.")).toBeVisible();
  });
});

describe("PublishPreview: checks", () => {
  it("separates workflows that run on a pull request from those that do not", () => {
    renderPreview();

    expect(screen.getByText("on pull_request")).toBeVisible();
    expect(screen.getByText("not on pull_request")).toBeVisible();
  });

  it("does not present configuration as a scheduled result", () => {
    renderPreview();

    expect(
      screen.getByText(/Nothing is scheduled until the pull request exists/),
    ).toBeVisible();
  });

  it("says plainly when there are no workflows to read", () => {
    renderPreview({ checks: [] });

    expect(screen.getByText(/No workflow files were found/)).toBeVisible();
  });
});

describe("PublishPreview: publishing", () => {
  it("pushes only on the deliberate action, with the reviewed plan", async () => {
    const user = userEvent.setup();
    const onPublish = vi.fn();
    renderPreview({ onPublish });

    await user.click(screen.getByRole("button", { name: /Push and open the pull request/ }));
    expect(onPublish).toHaveBeenCalledWith(plan);
  });

  it("reports a failure without claiming anything was pushed", () => {
    renderPreview({ plan: null, state: "failed", error: "remote rejected the push" });

    expect(screen.getByText("The publish plan could not be built")).toBeVisible();
    expect(screen.getByText("remote rejected the push")).toBeVisible();
  });
});
