import { describe, expect, it } from "vitest";

import {
  anchorPaths,
  anchorVerdict,
  shortSha,
  summarizeAnchors,
  summarizeVerdicts,
  verdictLabel,
  verdictState,
  WORKING_TREE_SHA,
  type BlobAnchor,
} from "./staleness";

const anchor = (path: string, sha: string): BlobAnchor => ({ path, sha });

describe("anchorVerdict", () => {
  it("calls a claim fresh only when the recorded sha still matches the tree", () => {
    const shas = new Map([["src/a.ts", "aaa"]]);
    expect(anchorVerdict(anchor("src/a.ts", "aaa"), shas)).toBe("fresh");
    expect(anchorVerdict(anchor("src/a.ts", "bbb"), shas)).toBe("stale");
  });

  it("reports a deleted file as missing rather than stale", () => {
    const shas = new Map<string, string | null>([["src/a.ts", null]]);
    expect(anchorVerdict(anchor("src/a.ts", "aaa"), shas)).toBe("missing");
  });

  it("refuses to guess when nothing has been looked up yet", () => {
    // The distinction that matters: an unchecked claim is not a fresh one, and
    // a surface rendering it must not be able to imply that it is.
    expect(anchorVerdict(anchor("src/a.ts", "aaa"), new Map())).toBe("unverified");
  });

  it("refuses to guess for a span that was read live and carries no version", () => {
    const shas = new Map([["src/a.ts", "aaa"]]);
    expect(anchorVerdict(anchor("src/a.ts", WORKING_TREE_SHA), shas)).toBe("unverified");
    expect(anchorVerdict(anchor("src/a.ts", ""), shas)).toBe("unverified");
  });
});

describe("summarizeVerdicts", () => {
  it("lets the worst member speak for the group", () => {
    expect(summarizeVerdicts(["fresh", "stale", "fresh"])).toBe("stale");
    expect(summarizeVerdicts(["stale", "missing"])).toBe("missing");
    expect(summarizeVerdicts(["fresh", "unverified"])).toBe("unverified");
    expect(summarizeVerdicts(["fresh", "fresh"])).toBe("fresh");
  });

  it("treats an answer with no citations as nothing to warn about", () => {
    expect(summarizeVerdicts([])).toBe("fresh");
    expect(summarizeAnchors([], new Map())).toBe("fresh");
  });

  it("summarizes anchors against the tree", () => {
    const shas = new Map([
      ["src/a.ts", "aaa"],
      ["src/b.ts", "changed"],
    ]);
    expect(
      summarizeAnchors([anchor("src/a.ts", "aaa"), anchor("src/b.ts", "bbb")], shas),
    ).toBe("stale");
  });
});

describe("verdictState", () => {
  it("spends a state mark only on something worth re-checking", () => {
    expect(verdictState("stale")).toBe("stale");
    expect(verdictState("missing")).toBe("stale");
    expect(verdictState("fresh")).toBeNull();
    expect(verdictState("unverified")).toBeNull();
  });

  it("says why in words for everything except a fresh claim", () => {
    expect(verdictLabel("fresh")).toBeNull();
    expect(verdictLabel("stale")).toMatch(/changed/);
    expect(verdictLabel("missing")).toMatch(/gone/);
    expect(verdictLabel("unverified")).toMatch(/not re-checked/);
  });
});

describe("shortSha", () => {
  it("abbreviates a real sha and leaves a non-sha alone", () => {
    expect(shortSha("0123456789abcdef")).toBe("0123456");
    expect(shortSha("abc")).toBe("abc");
    expect(shortSha(WORKING_TREE_SHA)).toBe(WORKING_TREE_SHA);
    expect(shortSha("")).toBe("");
  });
});

describe("anchorPaths", () => {
  it("collapses to one lookup per file", () => {
    expect(
      anchorPaths([
        anchor("src/a.ts", "1"),
        anchor("src/a.ts", "2"),
        anchor("src/b.ts", "3"),
        anchor("", "4"),
      ]),
    ).toEqual(["src/a.ts", "src/b.ts"]);
  });
});
