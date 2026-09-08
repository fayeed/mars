import type { ActivityState } from "@/shell/activity-state";

/**
 * What a claim on screen was read from.
 *
 * Everything Mars shows about source — a Project Brain citation, a search hit,
 * a packed context entry, a review finding's hunk — was derived from one exact
 * version of one file. That version is a blob sha, and it is the only thing
 * that makes the claim checkable later. A path and a line number are not
 * enough: both survive an edit that moves the line somewhere else, so a chip
 * reading `parser.ts:81` keeps looking correct long after line 81 stopped
 * being what the answer was about.
 *
 * Anchoring on the sha is what lets a surface say so.
 */
export interface BlobAnchor {
  /** Repository-relative path, as the index stores it. */
  path: string;
  /**
   * The blob sha the claim was derived from.
   *
   * `WORKING_TREE_SHA` when the core read the span live instead of from the
   * index, and there is therefore no recorded version to compare against.
   */
  sha: string;
}

/**
 * The sha the core writes for a span it read out of the working tree rather
 * than the index — graph-walk citations, for instance. It was true when it was
 * read and carries no version, so it can never be re-checked.
 */
export const WORKING_TREE_SHA = "working-tree";

/**
 * What is known about whether a claim still matches the file it came from.
 *
 * `unverified` is a first-class answer and not a synonym for fresh: it means
 * nobody has checked, which is exactly the case while a lookup is in flight or
 * when the claim never carried a sha. Collapsing it into `fresh` would let a
 * surface imply a guarantee it does not have.
 */
export type AnchorVerdict = "fresh" | "stale" | "missing" | "unverified";

/**
 * Current working-tree shas by path. A `null` value means the lookup ran and
 * the file is gone; a missing key means it has not run.
 */
export type WorkingTreeShas = ReadonlyMap<string, string | null>;

export function shortSha(sha: string, length = 7): string {
  if (!sha || sha === WORKING_TREE_SHA) return sha;
  return sha.length <= length ? sha : sha.slice(0, length);
}

export function anchorVerdict(anchor: BlobAnchor, shas: WorkingTreeShas): AnchorVerdict {
  if (!anchor.sha || anchor.sha === WORKING_TREE_SHA) return "unverified";
  if (!shas.has(anchor.path)) return "unverified";
  const current = shas.get(anchor.path) ?? null;
  if (current === null) return "missing";
  return current === anchor.sha ? "fresh" : "stale";
}

/** Worst first: one changed file makes a whole answer worth re-checking. */
const VERDICT_ORDER: readonly AnchorVerdict[] = ["missing", "stale", "unverified", "fresh"];

/** The one verdict a group of anchors should report upward. */
export function summarizeVerdicts(verdicts: readonly AnchorVerdict[]): AnchorVerdict {
  return VERDICT_ORDER.find((verdict) => verdicts.includes(verdict)) ?? "fresh";
}

export function summarizeAnchors(
  anchors: readonly BlobAnchor[],
  shas: WorkingTreeShas,
): AnchorVerdict {
  if (anchors.length === 0) return "fresh";
  return summarizeVerdicts(anchors.map((anchor) => anchorVerdict(anchor, shas)));
}

/**
 * How a verdict reads in the shell's state vocabulary, or `null` when there is
 * nothing to report. A surface must not spend a state mark saying "still true".
 */
export function verdictState(verdict: AnchorVerdict): ActivityState | null {
  return verdict === "stale" || verdict === "missing" ? "stale" : null;
}

/** What the label says, in words rather than an enum name. */
export function verdictLabel(verdict: AnchorVerdict): string | null {
  switch (verdict) {
    case "stale":
      return "changed since this was read";
    case "missing":
      return "the file is gone";
    case "unverified":
      return "not re-checked";
    default:
      return null;
  }
}

/** Every distinct path a set of anchors points at, for one batched lookup. */
export function anchorPaths(anchors: readonly BlobAnchor[]): string[] {
  const paths: string[] = [];
  for (const anchor of anchors) {
    if (!anchor.path || paths.includes(anchor.path)) continue;
    paths.push(anchor.path);
  }
  return paths;
}
