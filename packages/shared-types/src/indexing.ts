/**
 * The index as the UI states it, and what Mars can suggest doing next.
 *
 * Both are read-only projections. Nothing here decides anything; they exist so
 * a screen can say what is true about the index without guessing at it.
 */

export interface IndexCounts {
  files: number;
  symbols: number;
  edges: number;
  chunks: number;
  commits: number;
  ignoredPaths: number;
}

export interface IndexRunStatus {
  runId: string;
  [key: string]: unknown;
}

export interface IndexStatus {
  repositoryId: string;
  rootPath: string;
  branch: string | null;
  headOid: string | null;
  indexedHeadOid: string | null;
  /**
   * False when the working tree has moved to a different commit since the last
   * completed run, which is the usual cause of a stale index.
   */
  headMatchesIndex: boolean;
  embeddingModel: string;
  embeddingDimensions: number;
  counts: IndexCounts;
  lastRun: IndexRunStatus | null;
  runningRuns: number;
}

/**
 * Somewhere to start, and the evidence behind it. A suggestion is only worth as
 * much as the reason attached to it, so each carries the claim it rests on and
 * where that claim came from.
 */
export interface StartingPoint {
  id: string;
  title: string;
  /** Why this is being suggested, in terms a person can check. */
  reason: string;
  /** Where the claim came from, so it can be verified. */
  source: string;
  /** What picking this would put in the composer. */
  prompt: string;
}
