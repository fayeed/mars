import type { WhyAnswer as WhyAnswerValue } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { WhyAnswer } from "./WhyAnswer";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const answer: WhyAnswerValue = {
  status: "answered_with_conflicts",
  subject: { kind: "symbol", id: "symbol-1" },
  facts: [{
    knowledgeId: "knowledge-1",
    kind: "decision",
    title: "Keep validation strict",
    body: "Unknown extraction fields are rejected.",
    confidence: 0.91,
    provenance: "agent inferred",
  }],
  inferences: ["The validator exists to preserve the evidence boundary."],
  warnings: ["conflicting approved decision facts require human resolution"],
  sources: [{ label: "Evidence", location: "diff:src/schema.ts:17" }],
  paths: [{
    relation: "AFFECTS",
    steps: [
      { kind: "symbol", label: "parse", location: "symbol:symbol-1" },
      { kind: "knowledge", label: "Keep validation strict", location: "knowledge:knowledge-1" },
      { kind: "evidence", label: "diff:src/schema.ts:17", location: "diff:src/schema.ts:17" },
    ],
  }],
};

afterEach(() => vi.clearAllMocks());

it("separates facts from inference and opens exact diff evidence", async () => {
  const user = userEvent.setup();
  const onOpenCode = vi.fn();
  mocks.invoke.mockResolvedValue(answer);
  render(
    <WhyAnswer
      onOpenCode={onOpenCode}
      repositoryId="repository-1"
      subject={{ kind: "symbol", id: "symbol-1" }}
    />,
  );

  await user.click(screen.getByRole("button", { name: "Ask why this exists" }));
  expect(await screen.findByText(/FACT · agent inferred · 91%/)).toBeVisible();
  expect(screen.getByText(/INFERENCE · The validator exists/)).toBeVisible();
  expect(screen.getByText(/conflicting approved decision facts/)).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("why_query", {
    repositoryId: "repository-1",
    subject: { kind: "symbol", id: "symbol-1" },
    maxDepth: 5,
    maxPaths: 10,
  });

  await user.click(screen.getByRole("button", { name: "diff:src/schema.ts:17" }));
  expect(onOpenCode).toHaveBeenCalledWith("src/schema.ts", 17);
  expect(screen.getByLabelText("Opened evidence source")).toHaveTextContent("diff:src/schema.ts:17");
});
