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
    expect(readStoredLayout({ getItem: () => JSON.stringify(stored) })).toEqual(stored);
  });
});
