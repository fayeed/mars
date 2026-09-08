import { describe, expect, it } from "vitest";

import { INITIAL_LAYOUT, readStoredLayout, shellReducer } from "./shell-store";

describe("shellReducer", () => {
  it("changes destination without disturbing any other region", () => {
    const open = shellReducer(
      shellReducer(INITIAL_LAYOUT, { type: "openUtility", tab: "tests" }),
      { type: "openInspector", tab: "diff" },
    );

    const navigated = shellReducer(open, { type: "navigate", destination: "editor" });

    expect(navigated.destination).toBe("editor");
    expect(navigated.utilityOpen).toBe(true);
    expect(navigated.utilityTab).toBe("tests");
    expect(navigated.inspectorCollapsed).toBe(false);
    expect(navigated.inspectorTab).toBe("diff");
  });

  it("expands the inspector when a tab is picked from the collapsed rail", () => {
    const collapsed = shellReducer(INITIAL_LAYOUT, { type: "toggleInspector" });
    expect(collapsed.inspectorCollapsed).toBe(true);

    const expanded = shellReducer(collapsed, { type: "activateInspectorTab", tab: "plan" });
    expect(expanded.inspectorCollapsed).toBe(false);
    expect(expanded.inspectorTab).toBe("plan");
  });

  it("opens the utility panel on a tab request and closes it on the showing tab", () => {
    const opened = shellReducer(INITIAL_LAYOUT, { type: "activateUtilityTab", tab: "checks" });
    expect(opened).toMatchObject({ utilityOpen: true, utilityTab: "checks" });

    const switched = shellReducer(opened, { type: "activateUtilityTab", tab: "terminal" });
    expect(switched).toMatchObject({ utilityOpen: true, utilityTab: "terminal" });

    const closed = shellReducer(switched, { type: "activateUtilityTab", tab: "terminal" });
    expect(closed).toMatchObject({ utilityOpen: false, utilityTab: "terminal" });
  });

  it("returns the same object when a navigation changes nothing", () => {
    expect(shellReducer(INITIAL_LAYOUT, { type: "navigate", destination: "conversation" })).toBe(
      INITIAL_LAYOUT,
    );
  });
});

describe("readStoredLayout", () => {
  it("falls back to defaults for missing, unparsable, and unknown values", () => {
    expect(readStoredLayout(undefined)).toEqual(INITIAL_LAYOUT);
    expect(readStoredLayout({ getItem: () => null })).toEqual(INITIAL_LAYOUT);
    expect(readStoredLayout({ getItem: () => "{" })).toEqual(INITIAL_LAYOUT);
    expect(
      readStoredLayout({
        getItem: () => JSON.stringify({ destination: "settings", inspectorTab: "nope" }),
      }),
    ).toEqual(INITIAL_LAYOUT);
  });

  it("restores a layout it wrote", () => {
    const stored = {
      destination: "editor",
      taskRailCollapsed: true,
      inspectorCollapsed: true,
      inspectorTab: "diff",
      utilityOpen: true,
      utilityTab: "tests",
    };
    expect(readStoredLayout({ getItem: () => JSON.stringify(stored) })).toEqual({
      ...stored,
      // Which regions a hand has moved is a fact about this session, so it
      // starts clean however the geometry was restored.
      adjusted: { inspector: false, utility: false },
    });
  });
});

describe("suggestLayout", () => {
  it("opens the utility panel and folds the inspector away for a run", () => {
    const next = shellReducer(INITIAL_LAYOUT, {
      type: "suggestLayout",
      inspectorCollapsed: true,
      utilityOpen: true,
      utilityTab: "tests",
    });

    expect(next.inspectorCollapsed).toBe(true);
    expect(next.utilityOpen).toBe(true);
    expect(next.utilityTab).toBe("tests");
  });

  it("is refused for a region the person has already moved", () => {
    // A hand on the inspector outranks the machine's opinion about it.
    const touched = shellReducer(INITIAL_LAYOUT, { type: "toggleInspector" });
    expect(touched.adjusted.inspector).toBe(true);

    const suggested = shellReducer(touched, {
      type: "suggestLayout",
      inspectorCollapsed: false,
      utilityOpen: true,
    });

    expect(suggested.inspectorCollapsed).toBe(true);
    // The utility panel was never touched, so its half of the suggestion holds.
    expect(suggested.utilityOpen).toBe(true);
  });

  it("does not itself count as the person moving anything", () => {
    const suggested = shellReducer(INITIAL_LAYOUT, {
      type: "suggestLayout",
      inspectorCollapsed: true,
    });

    expect(suggested.adjusted.inspector).toBe(false);
  });

  it("forgets nothing and queues nothing: a refused suggestion is dropped", () => {
    const touched = shellReducer(INITIAL_LAYOUT, { type: "toggleUtility" });
    const refused = shellReducer(touched, { type: "suggestLayout", utilityTab: "tests" });

    expect(refused.utilityTab).toBe(INITIAL_LAYOUT.utilityTab);
  });
});
