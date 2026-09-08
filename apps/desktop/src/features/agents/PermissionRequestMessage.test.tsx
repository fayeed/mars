import type { PendingPermissionRequest } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PermissionRequestMessage } from "./PermissionRequestMessage";

const request: PendingPermissionRequest = {
  id: "req-1",
  taskId: "task-1",
  runId: "run-1",
  effect: {
    kind: "shell",
    command: "cargo test -p mars-ingest",
    cwd: "/data/worktrees/rate-limit-ingest",
  },
  classification: "mutating",
  summary: "run `cargo test -p mars-ingest` in the task worktree",
  reason: "cargo test compiles and executes code from the worktree",
  suggestedPattern: "cargo test*",
  requestedAt: "2026-09-08T12:00:00Z",
};

const destructive: PendingPermissionRequest = {
  ...request,
  id: "req-2",
  effect: { kind: "shell", command: "rm -rf target", cwd: "/data/worktrees/x" },
  classification: "destructive",
  summary: "run `rm -rf target`",
  reason: "rm -rf deletes without confirmation and cannot be undone",
  suggestedPattern: "rm -rf *",
};

describe("PermissionRequestMessage", () => {
  it("shows the exact command and where it would run", () => {
    render(<PermissionRequestMessage onDecide={vi.fn()} request={request} />);

    expect(screen.getByText("cargo test -p mars-ingest")).toBeVisible();
    expect(screen.getByText("/data/worktrees/rate-limit-ingest")).toBeVisible();
  });

  it("states the classification and why it was classified that way", () => {
    render(<PermissionRequestMessage onDecide={vi.fn()} request={request} />);

    expect(screen.getByText("mutating")).toBeVisible();
    expect(screen.getByText("cargo test compiles and executes code from the worktree")).toBeVisible();
  });

  it("says the agent is stopped and everything else is not", () => {
    render(<PermissionRequestMessage onDecide={vi.fn()} request={request} />);

    expect(screen.getByText("the agent is stopped")).toBeVisible();
    expect(screen.getByText(/Nothing else is blocked/)).toBeVisible();
  });

  it("offers the four decisions, each saying what it would grant", async () => {
    const user = userEvent.setup();
    const onDecide = vi.fn();
    render(<PermissionRequestMessage onDecide={onDecide} request={request} />);

    expect(screen.getByText("This one effect, this one time.")).toBeVisible();
    expect(screen.getByText("Every matching effect until this task ends.")).toBeVisible();
    // A pattern grant that does not show its pattern is a blank cheque.
    expect(screen.getByText(/Anything matching cargo test\*, until revoked/)).toBeVisible();
    expect(screen.getByText(/The agent is told no/)).toBeVisible();

    await user.click(screen.getByRole("button", { name: /Allow for this task/ }));
    expect(onDecide).toHaveBeenCalledWith("allow_task");
  });

  it("reports a denial as its own decision", async () => {
    const user = userEvent.setup();
    const onDecide = vi.fn();
    render(<PermissionRequestMessage onDecide={onDecide} request={request} />);

    await user.click(screen.getByRole("button", { name: /Deny/ }));
    expect(onDecide).toHaveBeenCalledWith("deny");
  });

  it("marks a destructive request without crying wolf over an ordinary one", () => {
    const { container: ordinary } = render(
      <PermissionRequestMessage onDecide={vi.fn()} request={request} />,
    );
    expect(
      ordinary.querySelector('[data-slot="permission-request"]')?.className,
    ).not.toContain("border-l-destructive");

    const { container: risky } = render(
      <PermissionRequestMessage onDecide={vi.fn()} request={destructive} />,
    );
    expect(risky.querySelector('[data-slot="permission-request"]')?.className).toContain(
      "border-l-destructive",
    );
  });

  it("renders a file write against the scope it falls outside of", () => {
    render(
      <PermissionRequestMessage
        onDecide={vi.fn()}
        request={{
          ...request,
          effect: {
            kind: "fileWrite",
            path: "/repo/secrets.env",
            declaredScope: ["crates/ingest/**"],
          },
        }}
      />,
    );

    expect(screen.getByText("/repo/secrets.env")).toBeVisible();
    expect(screen.getByText("crates/ingest/**")).toBeVisible();
  });

  it("renders a push with the remote and branch it would reach", () => {
    render(
      <PermissionRequestMessage
        onDecide={vi.fn()}
        request={{
          ...request,
          effect: {
            kind: "gitPush",
            remote: "origin",
            branch: "mars/rate-limit",
            cwd: "/data/worktrees/x",
          },
        }}
      />,
    );

    expect(screen.getByText("origin")).toBeVisible();
    expect(screen.getByText("mars/rate-limit")).toBeVisible();
  });

  it("stops taking answers while one is in flight", () => {
    render(<PermissionRequestMessage busy onDecide={vi.fn()} request={request} />);

    expect(screen.getByRole("button", { name: /Allow once/ })).toBeDisabled();
  });
});
