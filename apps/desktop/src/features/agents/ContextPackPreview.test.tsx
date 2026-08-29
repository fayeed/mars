import type { ContextPack } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import { ContextPackPreview } from "./ContextPackPreview";

const pack: ContextPack = {
  taskId: "task-1",
  adapter: "codex",
  maxBytes: 4096,
  totalBytes: 210,
  sectionUsage: [
    { section: "instructions", maxBytes: 819, usedBytes: 210 },
    { section: "context", maxBytes: 1024, usedBytes: 0 },
    { section: "code", maxBytes: 1433, usedBytes: 0 },
    { section: "history", maxBytes: 409, usedBytes: 0 },
    { section: "skills", maxBytes: 409, usedBytes: 0 },
  ],
  items: [
    {
      section: "instructions",
      entityRef: "context:rule",
      title: "Human constraint",
      content: "Keep keyboard behavior accessible.",
      provenance: "HUMAN",
      freshness: "current task attachment",
      sourceLocation: "context:rule",
      reason: "active task constraint",
      score: 1,
      byteSize: 210,
    },
  ],
  exclusions: [
    { entityRef: "context:removed", reason: "removed in pack preview" },
    { entityRef: "skill:blocked", reason: "skill is blocked" },
  ],
};

it("shows exact evidence and permits only user exclusions to be added back", async () => {
  const user = userEvent.setup();
  const onToggle = vi.fn();
  render(
    <ContextPackPreview
      busy={false}
      onContinue={vi.fn()}
      onOpenChange={vi.fn()}
      onToggle={onToggle}
      open
      pack={pack}
    />,
  );

  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText("active task constraint")).toBeVisible();
  expect(within(dialog).getByText("HUMAN")).toBeVisible();
  expect(within(dialog).getByText("context:rule")).toBeVisible();
  await user.click(within(dialog).getByRole("button", { name: "Remove Human constraint" }));
  expect(onToggle).toHaveBeenCalledWith("context:rule", false);
  await user.click(within(dialog).getByRole("button", { name: "Add" }));
  expect(onToggle).toHaveBeenCalledWith("context:removed", true);
  expect(within(dialog).getByText(/skill:blocked/)).toBeVisible();
  expect(within(dialog).getAllByRole("button", { name: "Add" })).toHaveLength(1);
});
