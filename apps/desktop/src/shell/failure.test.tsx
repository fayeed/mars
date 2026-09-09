import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  classifyFailure,
  failureCopy,
  failureMessage,
  isIndexCold,
  type FailureCause,
} from "./failure";
import { FailureBlock } from "./primitives";

const failure = (code: string, message = "something went wrong") => ({ code, message });

describe("classifyFailure", () => {
  const cases: [string, FailureCause][] = [
    // The four the reader is expected to meet, each from a real core code.
    ["adapter.not_installed", "agentUnavailable"],
    ["agent.not_running", "agentUnavailable"],
    ["index.walk", "indexCold"],
    ["permission.state", "permissionDenied"],
    ["skill.blocked", "permissionDenied"],
    ["forge.approval_required", "permissionDenied"],
    ["forge.http", "networkOffline"],
    ["skill.network", "networkOffline"],
    // And the two that decide whether a person should retry or rebuild.
    ["task.not_found", "notFound"],
    ["context_pack.changed", "conflict"],
    ["plan.changed", "conflict"],
  ];

  it.each(cases)("reads %s as %s", (code, cause) => {
    expect(classifyFailure(failure(code)).cause).toBe(cause);
  });

  it("matches on the code rather than the prose, which is meant to be rewritten", () => {
    expect(
      classifyFailure({ code: "adapter.probe_failed", message: "everything is fine really" }).cause,
    ).toBe("agentUnavailable");
  });

  it("keeps the core's own words whatever it decides they mean", () => {
    const classified = classifyFailure(failure("forge.http", "connection refused"));
    expect(classified.message).toBe("connection refused");
    expect(classified.code).toBe("forge.http");
  });

  it("admits it does not know rather than guessing", () => {
    // A code with no advice attached is better served by the core's message
    // than by an invented remedy.
    expect(classifyFailure(failure("task.timestamp")).cause).toBe("unknown");
    expect(failureCopy("unknown").remedy).toBeNull();
  });

  it("copes with what a rejected promise can actually be", () => {
    expect(classifyFailure(new Error("boom")).cause).toBe("unknown");
    expect(failureMessage(new Error("boom"))).toBe("boom");
    expect(failureMessage("plain string")).toBe("plain string");
    expect(classifyFailure(null).cause).toBe("unknown");
  });
});

describe("failureCopy", () => {
  it("says what was not changed, for every cause", () => {
    // The most useful thing a failure can report is how much of the reader's
    // work survived it.
    for (const cause of [
      "agentUnavailable",
      "indexCold",
      "permissionDenied",
      "networkOffline",
      "notFound",
      "conflict",
      "unknown",
    ] as FailureCause[]) {
      const copy = failureCopy(cause);
      expect(copy.description.length).toBeGreaterThan(0);
      expect(copy.title.length).toBeGreaterThan(0);
    }
  });

  it("does not draw a refused permission as a crash", () => {
    // Nothing broke: the broker did its job. `blocked` says a person has to act.
    expect(failureCopy("permissionDenied").state).toBe("blocked");
  });

  it("does not draw an unread index as a failure", () => {
    expect(failureCopy("indexCold").state).toBe("stale");
  });
});

describe("FailureBlock", () => {
  it("frames the failure and still shows the core's message verbatim", () => {
    render(<FailureBlock error={failure("adapter.not_installed", "codex: not found on PATH")} />);

    expect(screen.getByText("The agent is not available")).toBeVisible();
    expect(screen.getByText("codex: not found on PATH")).toBeVisible();
  });

  it("offers the remedy only when a screen can actually perform it", async () => {
    const user = userEvent.setup();
    const onRemedy = vi.fn();
    const { rerender } = render(<FailureBlock error={failure("forge.http")} />);
    expect(screen.queryByRole("button")).toBeNull();

    rerender(<FailureBlock error={failure("forge.http")} onRemedy={onRemedy} />);
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRemedy).toHaveBeenCalledTimes(1);
  });

  it("offers nothing for a cause with no honest move, even when handed a handler", () => {
    render(<FailureBlock error={failure("task.timestamp")} onRemedy={vi.fn()} />);
    expect(screen.queryByRole("button")).toBeNull();
  });
});

describe("isIndexCold", () => {
  it("treats an unread repository and an absent status the same way", () => {
    expect(isIndexCold(null)).toBe(true);
    expect(isIndexCold({ counts: { files: 0 } })).toBe(true);
    expect(isIndexCold({ counts: { files: 1 } })).toBe(false);
  });
});
