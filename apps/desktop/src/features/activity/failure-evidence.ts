/**
 * Reading evidence out of program output.
 *
 * Test runners are not a format Mars controls, so nothing here assumes one.
 * Every extractor recognises a shape it is sure about and gives up otherwise —
 * and the raw text is always shown alongside whatever was recognised, so a
 * failure to parse costs a convenience, never a fact.
 */

export interface SourceReference {
  /** Repository-relative where the text gave one; otherwise as written. */
  path: string;
  line: number;
  column: number | null;
  /** The exact text this was recognised from, so it can be shown verbatim. */
  raw: string;
}

export interface ComparedValues {
  expected: string;
  actual: string;
  /** What named the two sides, so the labels are the runner's, not ours. */
  expectedLabel: string;
  actualLabel: string;
}

export interface FailureEvidence {
  /** The assertion line, when one stands out. */
  assertion: string | null;
  compared: ComparedValues | null;
  references: SourceReference[];
}

/**
 * A path followed by `:line` or `:line:column`.
 *
 * Deliberately narrow: the path must look like a source path (a segment with a
 * dot in it) so timestamps like `12:30:01` and durations do not become links.
 */
const REFERENCE = /((?:[A-Za-z]:)?[\w./\\-]*\w+\.\w+):(\d+)(?::(\d+))?/g;

/** Runner-agnostic markers for a two-sided comparison. */
const COMPARISONS: { expected: RegExp; actual: RegExp; labels: [string, string] }[] = [
  // Rust's assert_eq!
  {
    expected: /^\s*left:\s*(.+)$/m,
    actual: /^\s*right:\s*(.+)$/m,
    labels: ["left", "right"],
  },
  // Most JavaScript and Python runners
  {
    expected: /^\s*[-+]?\s*expected:?\s*(.+)$/im,
    actual: /^\s*[-+]?\s*(?:actual|received|got):?\s*(.+)$/im,
    labels: ["expected", "actual"],
  },
];

// Rust puts the failing expression between the two words; most other
// runners name the error type outright.
const ASSERTION = /^.*(?:assertion[^\n]*?failed|AssertionError|expected\s.+?\sto\s).*$/im;

/** Every `file:line` the text mentions, in order, without duplicates. */
export function sourceReferences(text: string, limit = 12): SourceReference[] {
  const seen = new Set<string>();
  const found: SourceReference[] = [];
  for (const match of text.matchAll(REFERENCE)) {
    const [raw, path, line, column] = match;
    const key = `${path}:${line}:${column ?? ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    found.push({
      // Windows output uses backslashes; the rest of Mars speaks in forward ones.
      path: path.replace(/\\/g, "/"),
      line: Number(line),
      column: column === undefined ? null : Number(column),
      raw,
    });
    if (found.length >= limit) break;
  }
  return found;
}

/** The two sides of a failed comparison, when the runner named them. */
export function comparedValues(text: string): ComparedValues | null {
  for (const shape of COMPARISONS) {
    const expected = shape.expected.exec(text);
    const actual = shape.actual.exec(text);
    if (!expected || !actual) continue;
    return {
      expected: expected[1].trim(),
      actual: actual[1].trim(),
      expectedLabel: shape.labels[0],
      actualLabel: shape.labels[1],
    };
  }
  return null;
}

/** What can be said about a failure with confidence, and nothing beyond it. */
export function failureEvidence(text: string): FailureEvidence {
  const assertion = ASSERTION.exec(text);
  return {
    assertion: assertion ? assertion[0].trim() : null,
    compared: comparedValues(text),
    references: sourceReferences(text),
  };
}
