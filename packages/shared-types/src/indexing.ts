import type { IndexSummary } from "./brain";

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

export type Retrieval = "lexical" | "semantic" | "hybrid";

export interface SearchHit {
  chunkId: string;
  relativePath: string;
  symbolPath: string;
  header: string;
  body: string;
  language: string;
  startLine: number;
  endLine: number;
  blobSha: string;
  score: number;
  retrieval: Retrieval;
  lexicalRank: number | null;
  semanticRank: number | null;
}

export type FreshnessState = "fresh" | "stale" | "missing" | "unknown";

export interface Freshness {
  state: FreshnessState;
  relativePath: string;
  indexedBlobSha: string | null;
  currentBlobSha: string | null;
}

export interface SearchResults {
  query: string;
  hits: SearchHit[];
  freshness: Freshness[];
  /** True when any hit rests on a file that changed since it was indexed. */
  containsStale: boolean;
}

/**
 * One hit in the command palette, whatever kind of thing it points at.
 *
 * Nothing is ranked across kinds: there is no honest way to compare a file name
 * to an approved memory, so results are grouped rather than interleaved.
 */
export interface PaletteHit {
  kind: "task" | "message" | "file" | "symbol" | "memory";
  label: string;
  detail: string;
  /** A task id, a path, or a memory id, depending on `kind`. */
  target: string;
  line: number | null;
  taskId: string | null;
  sequence: number | null;
}

/**
 * Progress from an index pass, streamed on the `index-event` channel.
 *
 * A 20,000-file repository is minutes of work. Without these the app simply
 * goes quiet for the duration, which is indistinguishable from having hung.
 */
export type IndexEvent =
  | { event: "started"; runId: string; mode: string; totalFiles: number }
  | {
      event: "fileIndexed";
      runId: string;
      relativePath: string;
      position: number;
      totalFiles: number;
    }
  | { event: "phase"; runId: string; phase: string }
  // `IndexSummary` is declared in ./brain, which re-exports through the index.
  | { event: "completed"; runId: string; summary: IndexSummary }
  | { event: "failed"; runId: string; error: string }
  | { event: "cancelled"; runId: string };
