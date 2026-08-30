import type { OverlapWarning } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { OverlapWarnings } from "./OverlapWarnings";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const warnings: OverlapWarning[] = [{
  kind: "exact_file",
  severity: "error",
  message: "Both tasks change `src/sync.ts`",
  evidence: ["file:src/sync.ts"],
  otherTaskId: "task-2",
  otherTaskTitle: "Parallel sync change",
  otherWorktreePath: "/tmp/parallel-sync",
  refreshedAt: "2026-08-30T00:00:00Z",
}];

afterEach(() => vi.clearAllMocks());

it("shows evidence-bearing overlap warnings and refreshes without blocking work", async () => {
  mocks.invoke.mockResolvedValue(warnings);
  const user = userEvent.setup();
  render(<OverlapWarnings taskId="task-1" />);
  expect(await screen.findByText(warnings[0].message)).toBeVisible();
  expect(screen.getByText("file:src/sync.ts", { exact: false })).toBeVisible();
  expect(screen.getByText(warnings[0].otherWorktreePath)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Refresh overlap warnings" }));
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
  expect(mocks.invoke).toHaveBeenLastCalledWith("task_overlap_warnings", { taskId: "task-1" });
});
