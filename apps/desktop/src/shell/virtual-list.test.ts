import { describe, expect, it } from "vitest";

import { windowChanged, windowFor, type WindowResult } from "./virtual-list";

const base = {
  scrollTop: 0,
  viewportHeight: 500,
  estimatedRowHeight: 50,
  overscan: 0,
};

describe("windowFor", () => {
  it("renders only what the viewport can show", () => {
    const result = windowFor({ ...base, count: 1_000 });
    expect(result.start).toBe(0);
    // 500px of viewport over 50px rows, plus the row straddling the edge.
    expect(result.end).toBeLessThanOrEqual(12);
    expect(result.totalHeight).toBe(50_000);
  });

  it("leaves the right amount of space for what it did not render", () => {
    const result = windowFor({ ...base, count: 1_000, scrollTop: 5_000 });
    expect(result.start).toBe(100);
    expect(result.paddingTop).toBe(5_000);
    // The spacers and the rendered rows always add up to the whole list, or the
    // scrollbar would lie about how much is there.
    const rendered = (result.end - result.start) * 50;
    expect(result.paddingTop + rendered + result.paddingBottom).toBe(result.totalHeight);
  });

  it("keeps rows either side so a fast scroll never shows blank space", () => {
    const tight = windowFor({ ...base, count: 1_000, scrollTop: 5_000, overscan: 0 });
    const loose = windowFor({ ...base, count: 1_000, scrollTop: 5_000, overscan: 8 });
    expect(loose.start).toBe(tight.start - 8);
    expect(loose.end).toBe(tight.end + 8);
  });

  it("uses a measured height wherever it has one", () => {
    // Log rows are of wildly different heights: a one-line command beside a
    // forty-line failure. Assuming a fixed height would make the scrollbar jump
    // every time a tall row came into view.
    const measured = new Map([
      [0, 400],
      [1, 400],
    ]);
    const result = windowFor({ ...base, count: 10, measured });
    expect(result.totalHeight).toBe(400 + 400 + 8 * 50);
    expect(result.start).toBe(0);
    expect(result.end).toBe(2);
  });

  it("has nothing to say about an empty list", () => {
    const result = windowFor({ ...base, count: 0 });
    expect(result).toEqual({
      start: 0,
      end: 0,
      paddingTop: 0,
      paddingBottom: 0,
      totalHeight: 0,
    });
  });

  it("shows the last page rather than nothing when scrolled past the end", () => {
    // Happens for one frame after rows are removed: the scroll position still
    // points beyond a list that just got shorter.
    const result = windowFor({ ...base, count: 10, scrollTop: 100_000 });
    expect(result.end).toBe(10);
    expect(result.start).toBeLessThan(10);
  });

  it("never renders past the ends of the list", () => {
    const first = windowFor({ ...base, count: 5, overscan: 50 });
    expect(first.start).toBe(0);
    expect(first.end).toBe(5);
    expect(first.paddingTop).toBe(0);
    expect(first.paddingBottom).toBe(0);
  });
});

describe("windowChanged", () => {
  const result = (over: Partial<WindowResult>): WindowResult => ({
    start: 10,
    end: 20,
    paddingTop: 500,
    paddingBottom: 500,
    totalHeight: 1_500,
    ...over,
  });

  it("ignores a scroll that moved nothing worth re-rendering", () => {
    // Scroll fires far more often than the window changes; re-rendering a few
    // hundred rows to move a spacer by a pixel is the cost being avoided here.
    expect(windowChanged(result({}), result({ paddingTop: 500.5 }))).toBe(false);
  });

  it("notices a new range or a real shift", () => {
    expect(windowChanged(result({}), result({ start: 11 }))).toBe(true);
    expect(windowChanged(result({}), result({ end: 21 }))).toBe(true);
    expect(windowChanged(result({}), result({ paddingTop: 550 }))).toBe(true);
    expect(windowChanged(result({}), result({ paddingBottom: 550 }))).toBe(true);
  });
});
