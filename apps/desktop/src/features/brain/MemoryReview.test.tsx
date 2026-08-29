import type { KnowledgeRecord } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { MemoryReview } from "./MemoryReview";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const candidate: KnowledgeRecord = {
  id: "knowledge-1",
  projectId: "project-1",
  repositoryId: "repository-1",
  kind: "decision",
  title: "Keep validation strict",
  body: "The extraction schema rejects unknown fields.",
  status: "candidate",
  confidence: 0.91,
  provenanceId: "provenance-1",
  provenanceKind: "agent_inferred",
  resolutionProvenanceId: null,
  supersedesId: null,
  pinned: false,
  createdAt: "2026-08-29T00:00:00Z",
  updatedAt: "2026-08-29T00:00:00Z",
};

afterEach(() => vi.clearAllMocks());

it("requires visible extraction and human approval before memory is trusted", async () => {
  const user = userEvent.setup();
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "list_task_memory") return Promise.resolve([candidate]);
    if (command === "extract_task_memory") return Promise.resolve([candidate]);
    if (command === "resolve_memory") return Promise.resolve({ ...candidate, status: "approved" });
    return Promise.reject(new Error(`Unexpected command: ${command}`));
  });

  render(<MemoryReview adapter="codex" taskId="task-1" />);
  await user.click(screen.getByRole("button", { name: "Review project memory" }));
  const dialog = await screen.findByRole("dialog");

  expect(await within(dialog).findByText(candidate.title)).toBeVisible();
  expect(within(dialog).getByText(/Nothing becomes trusted project memory until you approve it/)).toBeVisible();

  await user.click(within(dialog).getByRole("button", { name: "Extract with codex" }));
  expect(mocks.invoke).toHaveBeenCalledWith("extract_task_memory", {
    taskId: "task-1",
    adapter: "codex",
    modelPermission: true,
  });

  await user.click(within(dialog).getByRole("button", { name: "Approve" }));
  expect(mocks.invoke).toHaveBeenCalledWith("resolve_memory", {
    request: {
      taskId: "task-1",
      recordId: candidate.id,
      action: "approve",
      title: null,
      body: null,
    },
  });
});
