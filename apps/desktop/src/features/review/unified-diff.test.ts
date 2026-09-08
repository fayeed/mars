import { describe, expect, it } from "vitest";

import { languageOf, parseUnifiedDiff, untrackedFiles } from "./unified-diff";

const MODIFIED = `diff --git a/crates/ingest/src/lib.rs b/crates/ingest/src/lib.rs
index 1a2b3c4..5d6e7f8 100644
--- a/crates/ingest/src/lib.rs
+++ b/crates/ingest/src/lib.rs
@@ -80,7 +80,9 @@ impl Ingest {
     pub fn accept(&self, token: &str) -> Result<(), Error> {
-        self.inner.accept(token)
+        self.limiter.check(token)?;
+        self.inner.accept(token)?;
+        Ok(())
     }
 }
`;

const ADDED = `diff --git a/crates/ingest/src/limit.rs b/crates/ingest/src/limit.rs
new file mode 100644
index 0000000..abcdef1
--- /dev/null
+++ b/crates/ingest/src/limit.rs
@@ -0,0 +1,2 @@
+pub struct Limiter;
+impl Limiter {}
`;

const REMOVED = `diff --git a/old.rs b/old.rs
deleted file mode 100644
index abcdef1..0000000
--- a/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-fn gone() {}
-
`;

const RENAMED = `diff --git a/src/a.rs b/src/b.rs
similarity index 92%
rename from src/a.rs
rename to src/b.rs
index 1a2b3c4..5d6e7f8 100644
--- a/src/a.rs
+++ b/src/b.rs
@@ -1,3 +1,3 @@
 fn keep() {}
-fn old() {}
+fn renamed() {}
`;

const BINARY = `diff --git a/logo.png b/logo.png
index 1a2b3c4..5d6e7f8 100644
Binary files a/logo.png and b/logo.png differ
`;

describe("parseUnifiedDiff", () => {
  it("numbers both sides of a modification correctly", () => {
    const [file] = parseUnifiedDiff(MODIFIED);

    expect(file.path).toBe("crates/ingest/src/lib.rs");
    expect(file.change).toBe("modified");
    expect(file.added).toBe(3);
    expect(file.removed).toBe(1);

    const [hunk] = file.hunks;
    expect(hunk.oldStart).toBe(80);
    expect(hunk.newStart).toBe(80);
    // The context line before the change is line 80 on both sides.
    expect(hunk.lines[0]).toMatchObject({ kind: "context", oldLine: 80, newLine: 80 });
    // A removed line has no line number on the right, and vice versa.
    expect(hunk.lines[1]).toMatchObject({ kind: "removed", oldLine: 81, newLine: null });
    expect(hunk.lines[2]).toMatchObject({ kind: "added", oldLine: null, newLine: 81 });
    expect(hunk.lines[4]).toMatchObject({ kind: "added", newLine: 83 });
    // Context after the change has advanced on both sides, by different amounts.
    expect(hunk.lines[5]).toMatchObject({ kind: "context", oldLine: 82, newLine: 84 });
  });

  it("keeps the hunk header verbatim beside the parsed numbers", () => {
    const [file] = parseUnifiedDiff(MODIFIED);
    expect(file.hunks[0].header).toBe("@@ -80,7 +80,9 @@ impl Ingest {");
  });

  it("reads a new file as added", () => {
    const [file] = parseUnifiedDiff(ADDED);

    expect(file.path).toBe("crates/ingest/src/limit.rs");
    expect(file.change).toBe("added");
    expect(file.added).toBe(2);
    expect(file.removed).toBe(0);
  });

  it("reads a deleted file as removed", () => {
    const [file] = parseUnifiedDiff(REMOVED);

    expect(file.path).toBe("old.rs");
    expect(file.change).toBe("removed");
    expect(file.removed).toBe(2);
  });

  it("reads a rename, and says where the file came from", () => {
    const [file] = parseUnifiedDiff(RENAMED);

    expect(file.change).toBe("renamed");
    expect(file.path).toBe("src/b.rs");
    expect(file.previousPath).toBe("src/a.rs");
  });

  it("marks a binary file rather than rendering it as empty", () => {
    const [file] = parseUnifiedDiff(BINARY);

    expect(file.path).toBe("logo.png");
    expect(file.binary).toBe(true);
    expect(file.hunks).toHaveLength(0);
  });

  it("separates every file in a multi-file diff", () => {
    const files = parseUnifiedDiff(`${MODIFIED}${ADDED}${REMOVED}`);

    expect(files.map((file) => file.path)).toEqual([
      "crates/ingest/src/lib.rs",
      "crates/ingest/src/limit.rs",
      "old.rs",
    ]);
  });

  it("does not count a no-newline marker as a line", () => {
    const [file] = parseUnifiedDiff(`diff --git a/x.txt b/x.txt
--- a/x.txt
+++ b/x.txt
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
`);

    expect(file.added).toBe(1);
    expect(file.removed).toBe(1);
    expect(file.hunks[0].lines).toHaveLength(2);
  });

  it("handles a single-line hunk, where Git omits the count", () => {
    const [file] = parseUnifiedDiff(`diff --git a/x.txt b/x.txt
--- a/x.txt
+++ b/x.txt
@@ -4 +4 @@
-a
+b
`);

    expect(file.hunks[0].oldLines).toBe(1);
    expect(file.hunks[0].newLines).toBe(1);
    expect(file.hunks[0].oldStart).toBe(4);
  });

  it("returns nothing for an empty diff rather than an empty file", () => {
    expect(parseUnifiedDiff("")).toEqual([]);
  });
});

describe("untrackedFiles", () => {
  it("lists files Git has never seen as additions with no hunks", () => {
    const [file] = untrackedFiles(["notes.md"]);

    expect(file).toMatchObject({ path: "notes.md", change: "added", hunks: [] });
  });
});

describe("languageOf", () => {
  it("maps an extension Monaco knows", () => {
    expect(languageOf("src/lib.rs")).toBe("rust");
    expect(languageOf("src/App.tsx")).toBe("typescript");
    expect(languageOf("Cargo.toml")).toBe("ini");
  });

  it("gives up on an extension it does not know, and on a file without one", () => {
    expect(languageOf("data.zzz")).toBeNull();
    expect(languageOf("Makefile")).toBeNull();
  });
});
