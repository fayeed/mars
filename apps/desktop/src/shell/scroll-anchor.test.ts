import { describe, expect, it } from "vitest";

import { anchoredScrollTop, isPinnedToBottom } from "./scroll-anchor";

describe("isPinnedToBottom", () => {
  it("treats the last few pixels as following the stream", () => {
    expect(isPinnedToBottom({ scrollTop: 800, scrollHeight: 1000, clientHeight: 200 })).toBe(true);
    expect(isPinnedToBottom({ scrollTop: 790, scrollHeight: 1000, clientHeight: 200 })).toBe(true);
    expect(isPinnedToBottom({ scrollTop: 400, scrollHeight: 1000, clientHeight: 200 })).toBe(false);
  });
});

describe("anchoredScrollTop", () => {
  it("keeps the reader's line in place when the utility panel opens", () => {
    // Reading mid-history: 400px above the bottom edge of a 1000px history.
    const previous = { scrollTop: 400, scrollHeight: 1000, clientHeight: 200 };
    // The panel takes 80px of height; the history itself did not change.
    const next = { scrollHeight: 1000, clientHeight: 120 };

    expect(anchoredScrollTop(previous, next)).toBe(480);
    const distanceBefore = previous.scrollHeight - previous.scrollTop - previous.clientHeight;
    const distanceAfter = next.scrollHeight - 480 - next.clientHeight;
    expect(distanceAfter).toBe(distanceBefore);
  });

  it("stays pinned to the newest turn for a reader who was at the bottom", () => {
    const previous = { scrollTop: 800, scrollHeight: 1000, clientHeight: 200 };
    expect(anchoredScrollTop(previous, { scrollHeight: 1000, clientHeight: 120 })).toBe(880);
  });

  it("never scrolls past the end or before the start", () => {
    const previous = { scrollTop: 100, scrollHeight: 1000, clientHeight: 200 };
    expect(anchoredScrollTop(previous, { scrollHeight: 1000, clientHeight: 950 })).toBe(0);
    expect(anchoredScrollTop(previous, { scrollHeight: 300, clientHeight: 400 })).toBe(0);
  });
});
