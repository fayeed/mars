import { describe, expect, it } from "vitest";

import { routeMessage, type RouteInput } from "./message-route";

const ready = { taskId: "t1", state: "ready" as const };

function route(overrides: Partial<RouteInput>) {
  return routeMessage({
    text: "make retries jittered",
    composeNew: false,
    task: ready,
    working: false,
    runs: [],
    ...overrides,
  });
}

describe("routeMessage", () => {
  it("answers /ask as a question wherever it is typed", () => {
    expect(route({ text: "/ask where is retry handled?" })).toEqual({
      kind: "ask",
      question: "where is retry handled?",
    });
    expect(route({ text: "/ask how?", working: true })).toEqual({ kind: "ask", question: "how?" });
  });

  it("starts a new task when none is open, or when one was asked for", () => {
    expect(route({ task: null })).toEqual({ kind: "new-task" });
    expect(route({ composeNew: true, runs: [{ providerSessionId: "s1" }] })).toEqual({
      kind: "new-task",
    });
  });

  it("queues behind an agent that is still working", () => {
    expect(route({ working: true, runs: [{ providerSessionId: "s1" }] })).toEqual({
      kind: "queue",
    });
  });

  it("refines a plan that is not approved yet", () => {
    expect(route({ task: { taskId: "t1", state: "planned" } })).toEqual({ kind: "refine-plan" });
  });

  it("opens the context review for a ready worktree no agent has worked in", () => {
    expect(route({ runs: [] })).toEqual({ kind: "first-run" });
    // A run that never reached the provider left nothing to resume.
    expect(route({ runs: [{ providerSessionId: null }] })).toEqual({ kind: "first-run" });
  });

  it("continues the session the task's agent already has", () => {
    expect(route({ runs: [{ providerSessionId: null }, { providerSessionId: "s1" }] })).toEqual({
      kind: "continue",
    });
  });
});
