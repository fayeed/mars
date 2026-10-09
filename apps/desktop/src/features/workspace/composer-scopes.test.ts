import { describe, expect, it } from "vitest";

import { composerScopes } from "./ShellWorkspace";

describe("composerScopes", () => {
  it("offers asking first, the repository's own largest directories, then all of it", () => {
    const scopes = composerScopes([
      { name: "src", files: 1200 },
      { name: "docs", files: 40 },
    ]);

    expect(scopes.map((scope) => scope.label)).toEqual([
      "Files I mention",
      "src/**",
      "docs/**",
      "Whole repository",
    ]);
    expect(scopes[0].globs).toEqual([]);
    expect(scopes[1]).toMatchObject({ globs: ["src/**"], detail: "1,200 file(s)" });
    expect(scopes.at(-1)?.globs).toEqual(["**"]);
  });

  it("names no directory that is not there", () => {
    expect(composerScopes([]).map((scope) => scope.id)).toEqual(["touched", "repository"]);
  });

  it("offers at most five directories", () => {
    const many = Array.from({ length: 9 }, (_, index) => ({ name: `d${index}`, files: 10 - index }));
    expect(composerScopes(many)).toHaveLength(7);
  });
});
