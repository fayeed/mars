import type { PullRequestSnapshot } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PullRequestActivity } from "./PullRequestActivity";

const snapshot: PullRequestSnapshot = {
  id: "pr-1",
  remoteId: "remote-1",
  taskId: "task-1",
  issueId: null,
  providerId: "9",
  number: 9,
  title: "Rate limit the ingest endpoint",
  body: "Adds a per-token limiter.",
  state: "open",
  url: "https://github.test/acme/mars/pull/9",
  head: "mars/rate-limit",
  base: "main",
  draft: false,
  mergeable: true,
  headSha: "511c628aaaa",
  updatedAt: "2026-09-08T12:00:00Z",
  comments: [
    {
      providerId: "comment-1",
      author: "sam",
      body: "This retries forever if the limiter throws.",
      path: "crates/ingest/src/lib.rs",
      line: 91,
      url: "https://github.test/acme/mars/pull/9#c1",
      createdAt: "2026-09-08T12:30:00Z",
    },
  ],
  checks: [
    {
      providerId: "check-1",
      name: "CI",
      status: "completed",
      conclusion: "failure",
      url: "https://github.test/checks/1",
      startedAt: null,
      completedAt: null,
    },
    {
      providerId: "check-2",
      name: "Lint",
      status: "in_progress",
      conclusion: null,
      url: "https://github.test/checks/2",
      startedAt: null,
      completedAt: null,
    },
  ],
};

function renderActivity(props: Partial<Parameters<typeof PullRequestActivity>[0]> = {}) {
  return render(
    <PullRequestActivity
      onOpenFile={vi.fn()}
      onOpenUrl={vi.fn()}
      onRefresh={vi.fn()}
      onReplyWithFix={vi.fn()}
      snapshot={snapshot}
      state="idle"
      {...props}
    />,
  );
}

describe("PullRequestActivity checks", () => {
  it("shows live status and counts what is failing", () => {
    renderActivity();

    expect(screen.getByText("CI")).toBeVisible();
    expect(screen.getByText("in_progress")).toBeVisible();
    expect(screen.getByText("1 failing")).toBeVisible();
  });

  it("refreshes only when asked", async () => {
    const user = userEvent.setup();
    const onRefresh = vi.fn();
    renderActivity({ onRefresh });

    await user.click(screen.getByRole("button", { name: /Refresh/ }));
    expect(onRefresh).toHaveBeenCalled();
  });
});

describe("PullRequestActivity comments", () => {
  it("renders a review comment as a message, with its author and anchor", () => {
    renderActivity();

    expect(screen.getByText("sam")).toBeVisible();
    expect(screen.getByText("This retries forever if the limiter throws.")).toBeVisible();
    expect(screen.getByText("crates/ingest/src/lib.rs:91")).toBeVisible();
  });

  it("opens the line a comment is about", async () => {
    const user = userEvent.setup();
    const onOpenFile = vi.fn();
    renderActivity({ onOpenFile });

    await user.click(screen.getByRole("button", { name: /crates\/ingest\/src\/lib\.rs:91/ }));
    expect(onOpenFile).toHaveBeenCalledWith("crates/ingest/src/lib.rs", 91);
  });

  it("threads a fix to the comment it answers", async () => {
    const user = userEvent.setup();
    const onReplyWithFix = vi.fn();
    renderActivity({ onReplyWithFix });

    await user.click(screen.getByRole("button", { name: /Answer this with a fix/ }));
    expect(onReplyWithFix).toHaveBeenCalledWith(
      expect.objectContaining({ providerId: "comment-1" }),
    );
  });

  it("says nobody has commented rather than showing an empty list", () => {
    renderActivity({ snapshot: { ...snapshot, comments: [] } });

    expect(screen.getByText("Nobody has commented yet.")).toBeVisible();
  });
});

describe("PullRequestActivity state", () => {
  it("reports an unmergeable pull request", () => {
    renderActivity({ snapshot: { ...snapshot, mergeable: false } });

    expect(screen.getByText(/cannot merge as it stands/)).toBeVisible();
  });

  it("shows the branches and the commit the checks ran against", () => {
    renderActivity();

    expect(screen.getByText(/mars\/rate-limit → main · 511c628/)).toBeVisible();
  });
});
