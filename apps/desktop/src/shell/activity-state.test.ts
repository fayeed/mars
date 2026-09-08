import { describe, expect, it } from "vitest";

import {
  ACTIVITY_STATE,
  ACTIVITY_STATES,
  attentionStates,
  summarizeStates,
} from "./activity-state";

describe("activity states", () => {
  it("separates every state by mark, pulse or tone rather than hue alone", () => {
    const signatures = ACTIVITY_STATES.map((state) => {
      const presentation = ACTIVITY_STATE[state];
      return `${presentation.tone}/${presentation.mark}/${presentation.pulse}`;
    });
    expect(new Set(signatures).size).toBe(ACTIVITY_STATES.length);
  });

  it("reserves the danger tone for failure", () => {
    const danger = ACTIVITY_STATES.filter((state) => ACTIVITY_STATE[state].tone === "danger");
    expect(danger).toEqual(["failed"]);
  });

  it("marks exactly the states that stop until a human answers", () => {
    const needsHuman = ACTIVITY_STATES.filter((state) => ACTIVITY_STATE[state].needsHuman);
    expect(needsHuman).toEqual(["blocked", "failed"]);
  });

  it("summarizes a group by its most urgent member", () => {
    expect(summarizeStates(["idle", "streaming", "failed"])).toBe("failed");
    expect(summarizeStates(["done", "streaming", "queued"])).toBe("streaming");
    expect(summarizeStates(["done", "idle"])).toBe("done");
    expect(summarizeStates([])).toBe("idle");
  });

  it("orders attention states most urgent first", () => {
    expect(attentionStates(["stale", "blocked", "streaming", "failed"])).toEqual([
      "failed",
      "blocked",
      "stale",
    ]);
    expect(attentionStates(["idle", "done"])).toEqual([]);
  });
});
