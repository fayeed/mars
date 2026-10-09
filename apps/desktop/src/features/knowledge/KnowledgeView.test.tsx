import type { ActivityEvent } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AgentKnowledgeEvent, isKnowledgeEvent } from "./AgentKnowledgeEvent";
import { KnowledgeView } from "./KnowledgeView";

describe("KnowledgeView", () => {
  it("opens on decisions and counts what waits in each tab", async () => {
    const user = userEvent.setup();
    render(
      <KnowledgeView
        code={<p>the graph</p>}
        decisions={<p>the decisions</p>}
        memory={<p>the memories</p>}
        waiting={{ decisions: 2, memory: 1 }}
      />,
    );

    expect(screen.getByText("the decisions")).toBeVisible();
    expect(screen.getByLabelText("2 waiting")).toBeVisible();
    await user.click(screen.getByRole("tab", { name: /Memory/ }));
    expect(screen.getByText("the memories")).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "Code graph" }));
    expect(screen.getByText("the graph")).toBeVisible();
  });

  it("opens on memory when only memory is waiting, or where it is asked to", () => {
    const { unmount } = render(
      <KnowledgeView decisions={<p>the decisions</p>} memory={<p>the memories</p>} waiting={{ decisions: 0, memory: 3 }} />,
    );
    expect(screen.getByText("the memories")).toBeVisible();
    expect(screen.queryByRole("tab", { name: "Code graph" })).toBeNull();
    unmount();

    render(
      <KnowledgeView
        decisions={<p>the decisions</p>}
        initialTab="decisions"
        memory={<p>the memories</p>}
        waiting={{ decisions: 0, memory: 3 }}
      />,
    );
    expect(screen.getByText("the decisions")).toBeVisible();
  });
});

function toolCall(name: string, input: unknown): ActivityEvent {
  return {
    id: "e1",
    taskId: "task-1",
    sequence: 7,
    runId: "run-1",
    kind: "tool_call",
    actor: "agent",
    actorLabel: "claude",
    summary: name,
    payload: { name, input, callId: "c1" },
    payloadDigest: null,
    occurredAt: "2026-10-09T10:00:00Z",
    durationMs: null,
  };
}

describe("AgentKnowledgeEvent", () => {
  it("shows a recorded decision and sends the person to review it", async () => {
    const user = userEvent.setup();
    const onReview = vi.fn();
    const event = toolCall("mcp__mars__record_decision", {
      title: "Use a token bucket",
      decision: "Rate limit with a token bucket.",
      rationale: "Bursts are normal.",
      alternatives: ["fixed window"],
      files: ["src/ingest.ts"],
    });
    expect(isKnowledgeEvent(event)).toBe(true);
    render(<AgentKnowledgeEvent event={event} onReview={onReview} />);

    expect(screen.getByText("Use a token bucket")).toBeVisible();
    expect(screen.getByText("Why: Bursts are normal.")).toBeVisible();
    expect(screen.getByText("not chosen: fixed window")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Review" }));
    expect(onReview).toHaveBeenCalledWith("decisions");
  });

  it("shows a proposed memory and sends the person to the memory queue", async () => {
    const user = userEvent.setup();
    const onReview = vi.fn();
    render(
      <AgentKnowledgeEvent
        event={toolCall("mcp__mars__remember", { text: "Tests run with pnpm test", kind: "convention" })}
        onReview={onReview}
      />,
    );

    expect(screen.getByText("Tests run with pnpm test")).toBeVisible();
    expect(screen.getByText("convention · repo")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Review" }));
    expect(onReview).toHaveBeenCalledWith("memory");
  });

  it("leaves every other tool call to the plain row", () => {
    expect(isKnowledgeEvent(toolCall("Read", { file_path: "a.ts" }))).toBe(false);
    expect(isKnowledgeEvent(toolCall("mcp__mars__search", { query: "x" }))).toBe(false);
  });
});
