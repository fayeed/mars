import type { CheckRun, TaskProjection } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  ChecksPanel,
  ProblemsPanel,
  TestsPanel,
  problemsFromProjection,
} from "./UtilityPanels";

const projection: TaskProjection = {
  taskId: "task-1",
  lastSequence: 30,
  eventCount: 30,
  runs: [],
  pendingPermissions: [],
  decidedPermissions: [],
  filesTouched: [],
  commands: [
    {
      command: "cargo build",
      status: "failed",
      exitCode: 101,
      durationMs: 900,
      outputDigest: null,
      runId: "run-1",
      sequence: 10,
    },
  ],
  tests: [
    {
      command: "cargo test",
      status: "failed",
      durationMs: 4200,
      resultDigest: "digest-1",
      sequence: 20,
    },
  ],
  errors: ["the adapter exited unexpectedly"],
  worktree: null,
};

describe("problemsFromProjection", () => {
  it("reports what actually ran and failed, and invents nothing else", () => {
    const problems = problemsFromProjection(projection);

    expect(problems).toHaveLength(2);
    expect(problems[0]).toMatchObject({ source: "command", message: "exited 101" });
    expect(problems[1]).toMatchObject({ source: "agent" });
  });

  it("has nothing to say about a task with no record", () => {
    expect(problemsFromProjection(null)).toEqual([]);
  });
});

describe("ProblemsPanel", () => {
  it("groups by the file a problem is about and links each row", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(
      <ProblemsPanel
        onOpen={onOpen}
        problems={[
          {
            path: "crates/ingest/src/lib.rs",
            line: 82,
            severity: "error",
            message: "mismatched types",
            source: "cargo",
          },
        ]}
        state="idle"
      />,
    );

    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    await user.click(screen.getByRole("button", { name: /mismatched types/ }));
    expect(onOpen).toHaveBeenCalledWith("crates/ingest/src/lib.rs", 82);
  });

  it("does not present an empty list as a clean bill of health", () => {
    render(<ProblemsPanel onOpen={vi.fn()} problems={[]} state="idle" />);

    expect(screen.getByText(/not a clean bill of health/)).toBeVisible();
  });
});

describe("TestsPanel", () => {
  it("shows each run with its status, duration and a way to the output", async () => {
    const user = userEvent.setup();
    const onOpenOutput = vi.fn();
    const onRerun = vi.fn();
    render(
      <TestsPanel
        onOpenOutput={onOpenOutput}
        onRerun={onRerun}
        projection={projection}
        state="idle"
      />,
    );

    expect(screen.getByText("cargo test")).toBeVisible();
    expect(screen.getByText("4.2s")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Output" }));
    expect(onOpenOutput).toHaveBeenCalledWith("digest-1");

    await user.click(screen.getByRole("button", { name: /Re-run/ }));
    expect(onRerun).toHaveBeenCalledWith("cargo test");
  });

  it("says an unsupported claim is unsupported", () => {
    render(
      <TestsPanel
        onOpenOutput={vi.fn()}
        onRerun={vi.fn()}
        projection={{ ...projection, tests: [] }}
        state="idle"
      />,
    );

    expect(screen.getByText(/making a claim the record does not support/)).toBeVisible();
  });
});

describe("ChecksPanel", () => {
  const check: CheckRun = {
    providerId: "check-1",
    name: "CI",
    status: "completed",
    conclusion: "failure",
    url: "https://github.test/checks/1",
    startedAt: null,
    completedAt: null,
  };

  it("shows each check with a way to its logs", async () => {
    const user = userEvent.setup();
    const onOpenUrl = vi.fn();
    render(
      <ChecksPanel checks={[check]} onOpenUrl={onOpenUrl} onRefresh={vi.fn()} state="idle" />,
    );

    expect(screen.getByText("CI")).toBeVisible();
    expect(screen.getByText("failure")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Logs" }));
    expect(onOpenUrl).toHaveBeenCalledWith(check.url);
  });

  it("says checks only exist once a pull request does", () => {
    render(<ChecksPanel checks={[]} onOpenUrl={vi.fn()} onRefresh={vi.fn()} state="idle" />);

    expect(screen.getByText(/once a pull request exists/)).toBeVisible();
  });
});
