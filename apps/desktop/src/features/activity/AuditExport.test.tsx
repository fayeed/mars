import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ShellProvider } from "@/shell/shell-store";

import { AuditExport, auditFileName } from "./AuditExport";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  save: vi.fn(),
}));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock(import("@tauri-apps/plugin-dialog"), () => ({ save: mocks.save }));

const DOCUMENT = "# Activity record for task task-1\n\n## Decisions\n\nrequest-1 allow_task\n";

function renderExport(props: Partial<Parameters<typeof AuditExport>[0]> = {}) {
  return render(
    <ShellProvider>
      <AuditExport
        onClose={vi.fn()}
        open
        taskId="task-1"
        taskTitle="Guard the divisor"
        {...props}
      />
    </ShellProvider>,
  );
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.save.mockReset();
});

describe("auditFileName", () => {
  it("names the file after the task and the day, so it reads without opening", () => {
    expect(auditFileName("Guard the divisor!", "task-1", new Date("2026-09-09T10:00:00Z"))).toBe(
      "2026-09-09-guard-the-divisor-activity.md",
    );
  });

  it("falls back to the task id when there is no title", () => {
    expect(auditFileName(undefined, "task-1", new Date("2026-09-09T10:00:00Z"))).toBe(
      "2026-09-09-task-1-activity.md",
    );
  });

  it("copes with a title that slugs down to nothing", () => {
    expect(auditFileName("···", "task-1", new Date("2026-09-09T10:00:00Z"))).toBe(
      "2026-09-09-task-activity.md",
    );
  });
});

describe("AuditExport", () => {
  it("shows the exact bytes it would write, before anything is written", async () => {
    mocks.invoke.mockResolvedValue(DOCUMENT);
    renderExport();

    // The artefact is what somebody hands to a reviewer; the moment to see what
    // is in it is before it leaves the machine.
    await waitFor(() => expect(screen.getByText(/request-1 allow_task/)).toBeVisible());
    expect(mocks.save).not.toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalledWith("task_audit", { taskId: "task-1" });
  });

  it("writes only where the person chose, and says what landed there", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockImplementation((command: string) =>
      command === "task_audit"
        ? Promise.resolve(DOCUMENT)
        : Promise.resolve({ path: "/home/f/record.md", byteSize: 4096 }),
    );
    mocks.save.mockResolvedValue("/home/f/record.md");
    renderExport();

    await waitFor(() => expect(screen.getByText(/request-1 allow_task/)).toBeVisible());
    await user.click(screen.getByRole("button", { name: /Save as Markdown/ }));

    await waitFor(() => expect(screen.getByText("/home/f/record.md")).toBeVisible());
    expect(mocks.invoke).toHaveBeenCalledWith("write_task_audit", {
      taskId: "task-1",
      destination: "/home/f/record.md",
    });
  });

  it("treats a dismissed save dialog as a choice rather than a failure", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockResolvedValue(DOCUMENT);
    mocks.save.mockResolvedValue(null);
    renderExport();

    await waitFor(() => expect(screen.getByText(/request-1 allow_task/)).toBeVisible());
    await user.click(screen.getByRole("button", { name: /Save as Markdown/ }));

    expect(mocks.invoke).not.toHaveBeenCalledWith("write_task_audit", expect.anything());
    expect(screen.queryByText(/could not be exported/)).toBeNull();
  });

  it("reports a failed render as a failure with a way to try again", async () => {
    mocks.invoke.mockRejectedValue({ code: "activity.storage", message: "database is locked" });
    renderExport();

    await waitFor(() => expect(screen.getByText(/could not be exported/)).toBeVisible());
    expect(screen.getByText("database is locked")).toBeVisible();
  });

  it("asks for nothing while it is closed", () => {
    renderExport({ open: false });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
