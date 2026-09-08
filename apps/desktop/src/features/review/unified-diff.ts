/**
 * Reading Git's unified diff into something a screen can render.
 *
 * The parser is deliberately literal: it reports what the diff says and never
 * infers what it does not. A file Git marked binary has no lines to show and
 * says so rather than rendering as empty; a hunk header Git wrote is kept
 * verbatim next to the parsed numbers, so a reader can always check the
 * rendering against the source.
 */

export type DiffLineKind = "context" | "added" | "removed";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
  /** Line number on the left (before). Null for an added line. */
  oldLine: number | null;
  /** Line number on the right (after). Null for a removed line. */
  newLine: number | null;
}

export interface DiffHunk {
  /** The `@@ ... @@` line, verbatim. */
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export type FileChange = "added" | "removed" | "modified" | "renamed";

export interface DiffFile {
  /** The path as it stands after the change. */
  path: string;
  /** Where it came from, when the change is a rename. */
  previousPath: string | null;
  change: FileChange;
  /** True when Git refused to diff the contents. There is nothing to show. */
  binary: boolean;
  hunks: DiffHunk[];
  added: number;
  removed: number;
}

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/** `a/src/lib.rs` → `src/lib.rs`; `/dev/null` stays as it is. */
function stripPrefix(path: string): string {
  if (path === "/dev/null") return path;
  return path.replace(/^[ab]\//, "");
}

/**
 * A `diff --git a/x b/y` line.
 *
 * Paths containing spaces make this ambiguous in general; Git quotes those, and
 * the `---`/`+++` lines that follow are authoritative, so this is only a first
 * guess that later lines correct.
 */
function pathsFromHeader(line: string): { old: string; new: string } | null {
  const rest = line.slice("diff --git ".length);
  const half = Math.floor(rest.length / 2);
  const left = rest.slice(0, half).trim();
  const right = rest.slice(half).trim();
  if (!left || !right) return null;
  return { old: stripPrefix(left), new: stripPrefix(right) };
}

/** Parse a whole unified diff into its files. */
export function parseUnifiedDiff(diff: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldLine = 0;
  let newLine = 0;

  const finishFile = () => {
    if (file) files.push(file);
    file = null;
    hunk = null;
  };

  for (const line of diff.split("\n")) {
    if (line.startsWith("diff --git ")) {
      finishFile();
      const paths = pathsFromHeader(line);
      file = {
        path: paths?.new ?? "",
        previousPath: null,
        change: "modified",
        binary: false,
        hunks: [],
        added: 0,
        removed: 0,
      };
      continue;
    }

    if (!file) continue;

    if (line.startsWith("new file mode")) {
      file.change = "added";
      continue;
    }
    if (line.startsWith("deleted file mode")) {
      file.change = "removed";
      continue;
    }
    if (line.startsWith("rename from ")) {
      file.previousPath = line.slice("rename from ".length);
      file.change = "renamed";
      continue;
    }
    if (line.startsWith("rename to ")) {
      file.path = line.slice("rename to ".length);
      file.change = "renamed";
      continue;
    }
    if (line.startsWith("Binary files ") || line.startsWith("GIT binary patch")) {
      file.binary = true;
      continue;
    }
    if (line.startsWith("--- ")) {
      const path = stripPrefix(line.slice(4).trim());
      if (path === "/dev/null") file.change = "added";
      continue;
    }
    if (line.startsWith("+++ ")) {
      const path = stripPrefix(line.slice(4).trim());
      if (path === "/dev/null") {
        file.change = "removed";
      } else {
        // Authoritative over the `diff --git` guess.
        file.path = path;
      }
      continue;
    }

    const match = HUNK.exec(line);
    if (match) {
      oldLine = Number(match[1]);
      newLine = Number(match[3]);
      hunk = {
        header: line,
        oldStart: oldLine,
        oldLines: match[2] === undefined ? 1 : Number(match[2]),
        newStart: newLine,
        newLines: match[4] === undefined ? 1 : Number(match[4]),
        lines: [],
      };
      file.hunks.push(hunk);
      continue;
    }

    if (!hunk) continue;

    // "\ No newline at end of file" annotates the line before it and is not
    // itself a line of either side.
    if (line.startsWith("\\")) continue;

    if (line.startsWith("+")) {
      hunk.lines.push({ kind: "added", text: line.slice(1), oldLine: null, newLine });
      file.added += 1;
      newLine += 1;
    } else if (line.startsWith("-")) {
      hunk.lines.push({ kind: "removed", text: line.slice(1), oldLine, newLine: null });
      file.removed += 1;
      oldLine += 1;
    } else if (line.startsWith(" ") || line === "") {
      // A context line is a space followed by the text. A truly empty string is
      // the trailing split artefact at the end of the diff, not a line.
      if (line === "") continue;
      hunk.lines.push({ kind: "context", text: line.slice(1), oldLine, newLine });
      oldLine += 1;
      newLine += 1;
    }
  }

  finishFile();
  return files;
}

/** Files Git has never seen, as entries a review can list beside the diff. */
export function untrackedFiles(paths: readonly string[]): DiffFile[] {
  return paths.map((path) => ({
    path,
    previousPath: null,
    change: "added" as const,
    binary: false,
    hunks: [],
    added: 0,
    removed: 0,
  }));
}

/** The language id Monaco knows this file by, or null when it has none. */
export function languageOf(path: string): string | null {
  const extension = path.split(".").at(-1)?.toLowerCase();
  if (!extension || extension === path.toLowerCase()) return null;
  const languages: Record<string, string> = {
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    jsx: "javascript",
    mjs: "javascript",
    cjs: "javascript",
    rs: "rust",
    py: "python",
    go: "go",
    java: "java",
    rb: "ruby",
    php: "php",
    cs: "csharp",
    cpp: "cpp",
    cc: "cpp",
    c: "c",
    h: "cpp",
    hpp: "cpp",
    swift: "swift",
    kt: "kotlin",
    scala: "scala",
    sh: "shell",
    bash: "shell",
    sql: "sql",
    json: "json",
    yaml: "yaml",
    yml: "yaml",
    toml: "ini",
    md: "markdown",
    html: "html",
    css: "css",
    scss: "scss",
  };
  return languages[extension] ?? null;
}
