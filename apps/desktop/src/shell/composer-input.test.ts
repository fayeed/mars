import { describe, expect, it } from "vitest";

import { applyCompletion, filterCandidates, readTrigger } from "./composer-input";

describe("readTrigger", () => {
  it("opens mentions at the start of the message and after whitespace", () => {
    expect(readTrigger("@src", 4)).toEqual({ kind: "mention", start: 0, query: "src" });
    expect(readTrigger("look at @App", 12)).toEqual({ kind: "mention", start: 8, query: "App" });
  });

  it("does not treat an address or a suffix as a mention", () => {
    expect(readTrigger("mail me at fayeed@example", 25)).toBeNull();
  });

  it("opens skills only at the start of a line", () => {
    expect(readTrigger("/rev", 4)).toEqual({ kind: "skill", start: 0, query: "rev" });
    expect(readTrigger("first line\n/rev", 15)).toEqual({ kind: "skill", start: 11, query: "rev" });
  });

  it("leaves repository paths alone", () => {
    expect(readTrigger("edit src/shell/Composer.tsx", 27)).toBeNull();
    expect(readTrigger("@src/shell", 10)).toEqual({ kind: "mention", start: 0, query: "src/shell" });
  });

  it("closes once the query is finished by whitespace", () => {
    expect(readTrigger("@src/main.ts and then", 21)).toBeNull();
  });

  it("reads from the caret, not the end of the value", () => {
    expect(readTrigger("@app trailing text", 4)).toEqual({ kind: "mention", start: 0, query: "app" });
  });
});

describe("applyCompletion", () => {
  it("replaces the trigger and its query, leaving a trailing space", () => {
    const value = "look at @App and stop";
    const trigger = readTrigger(value, 12);
    expect(trigger).not.toBeNull();

    const result = applyCompletion(value, trigger!, "@src/App.tsx");
    expect(result.value).toBe("look at @src/App.tsx and stop");
    expect(result.value.slice(0, result.caret)).toBe("look at @src/App.tsx ");
  });
});

describe("filterCandidates", () => {
  const files = [
    { label: "App.tsx", detail: "src/App.tsx" },
    { label: "Composer.tsx", detail: "src/shell/Composer.tsx" },
    { label: "main.rs", detail: "src-tauri/src/main.rs" },
  ];

  it("matches on label and on path", () => {
    expect(filterCandidates(files, "compos").map((file) => file.label)).toEqual(["Composer.tsx"]);
    expect(filterCandidates(files, "src-tauri").map((file) => file.label)).toEqual(["main.rs"]);
  });

  it("returns everything up to the limit for an empty query", () => {
    expect(filterCandidates(files, "  ", 2)).toHaveLength(2);
  });
});
