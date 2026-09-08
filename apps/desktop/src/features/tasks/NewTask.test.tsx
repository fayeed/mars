import type { StoredIssue } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { NewTask } from "./NewTask";

const issue: StoredIssue = {
  id: "issue-1",
  remoteId: "remote-1",
  repositoryId: "repository-1",
  providerId: "7",
  number: 7,
  title: "Ingest accepts over the limit",
  body: "Two clients on one token get double the budget.",
  state: "open",
  url: "https://github.test/acme/mars/issues/7",
  author: "sam",
  labels: ["bug"],
  updatedAt: "2026-09-08T12:00:00Z",
};

function renderNewTask(props: Partial<Parameters<typeof NewTask>[0]> = {}) {
  return render(
    <NewTask
      agents={[
        { id: "claude", name: "Claude", permissionScope: "read files" },
        { id: "codex", name: "Codex", permissionScope: "read files" },
      ]}
      importState="idle"
      importedIssue={null}
      onDescribe={vi.fn()}
      onImportIssue={vi.fn()}
      onSelectAgent={vi.fn()}
      onSelectScope={vi.fn()}
      scopes={[
        { id: "narrow", label: "Files I mention", globs: [] },
        { id: "crates", label: "crates/**", globs: ["crates/**"] },
      ]}
      selectedAgentId="claude"
      selectedScopeId="narrow"
      {...props}
    />,
  );
}

describe("NewTask", () => {
  it("offers exactly the two ways work enters Mars", () => {
    renderNewTask();

    expect(screen.getAllByRole("tab")).toHaveLength(2);
    expect(screen.getByRole("tab", { name: /Describe it/ })).toBeVisible();
    expect(screen.getByRole("tab", { name: /Import an issue/ })).toBeVisible();
  });

  it("describes a task with the chosen agent and scope", async () => {
    const user = userEvent.setup();
    const onDescribe = vi.fn();
    const onSelectScope = vi.fn();
    renderNewTask({ onDescribe, onSelectScope });

    await user.type(
      screen.getByRole("textbox", { name: "Describe the task" }),
      "Rate limit the ingest endpoint.",
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "Scope" }), "crates");
    expect(onSelectScope).toHaveBeenCalledWith("crates");

    await user.click(screen.getByRole("button", { name: /Plan this task/ }));
    expect(onDescribe).toHaveBeenCalledWith("Rate limit the ingest endpoint.");
  });

  it("will not plan an empty description", () => {
    renderNewTask();

    expect(screen.getByRole("button", { name: /Plan this task/ })).toBeDisabled();
  });

  it("fetches an issue by number", async () => {
    const user = userEvent.setup();
    const onImportIssue = vi.fn();
    renderNewTask({ onImportIssue });

    await user.click(screen.getByRole("tab", { name: /Import an issue/ }));
    await user.type(screen.getByRole("textbox", { name: "Issue number" }), "7");
    await user.click(screen.getByRole("button", { name: "Fetch" }));

    expect(onImportIssue).toHaveBeenCalledWith("origin", 7);
  });

  it("pre-fills the description from an imported issue and keeps the link", async () => {
    const user = userEvent.setup();
    renderNewTask({ importedIssue: issue });

    await user.click(screen.getByRole("tab", { name: /Import an issue/ }));

    expect(screen.getByRole("textbox", { name: "Describe the task" })).toHaveValue(
      `${issue.title}\n\n${issue.body}`,
    );
    expect(screen.getByText(issue.url)).toBeVisible();
    expect(screen.getByText(/The task keeps this link/)).toBeVisible();
  });

  it("says nothing is created until the plan is approved", () => {
    renderNewTask();

    expect(screen.getByText(/nothing touches Git until you approve it/)).toBeVisible();
  });
});
