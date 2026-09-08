export interface IndexSummary {
  runId: string;
  mode: string;
  indexedFiles: number;
  skippedFiles: number;
  deletedFiles: number;
  renamedFiles: number;
  chunkCount: number;
  ignoredPaths: number;
  commitsIngested: number;
  headOid: string | null;
}

export interface BrainSymbol {
  id: string;
  name: string;
  kind: string;
  qualifiedName: string;
  relativePath: string;
  startLine: number;
  endLine: number;
  exported: boolean;
}

export interface EdgeEvidence {
  relativePath: string;
  startByte: number;
  endByte: number;
  startLine: number;
  endLine: number;
  parser: string;
}

export interface BrainEdge {
  id: string;
  edgeKind: string;
  source: BrainSymbol | null;
  target: BrainSymbol | null;
  targetKey: string;
  confidence: number;
  evidence: EdgeEvidence;
}

export interface BrainNeighborhood {
  root: BrainSymbol;
  symbols: BrainSymbol[];
  edges: BrainEdge[];
}

export interface Dependency {
  relativePath: string;
  specifier: string;
  confidence: number;
  evidence: EdgeEvidence;
}

/**
 * Project Brain answers.
 *
 * A claim is either a fact, which is what a cited span of source says, or an
 * inference, which is a conclusion drawn from facts. The two are separate in
 * this model and must stay separate wherever they are rendered: a reader must
 * never have to guess which one they are looking at.
 */
export type ClaimKind = "fact" | "inference";

export interface Claim {
  id: string;
  kind: ClaimKind;
  text: string;
  /** Citation ids. Never empty for a fact. */
  citations: string[];
  /** Claim ids an inference reasons over. Always empty for a fact. */
  supports: string[];
  confidence: number;
}

export interface Citation {
  id: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  blobSha: string;
  snippet: string;
  /** Why retrieval surfaced this span. */
  why: string;
}

export type GapKind =
  | "notIndexed"
  | "ambiguous"
  | "requiresRuntime"
  | "stale"
  | "budgetExhausted"
  | "noReasoning";

export interface Gap {
  kind: GapKind;
  subject: string;
  detail: string;
}

export interface Signal {
  name: string;
  value: number;
}

export interface AnswerSource {
  chunkId: string;
  path: string;
  symbolPath: string;
  lineStart: number;
  lineEnd: number;
  blobSha: string;
  score: number;
  tokens: number;
  signals: Signal[];
  stale: boolean;
}

export interface BrainAnswer {
  question: string;
  rewrittenQueries: string[];
  claims: Claim[];
  citations: Citation[];
  /** Sources packed into the synthesis context. Shown verbatim in the UI. */
  sources: number;
  /** Everything retrieval found, before budget packing dropped any of it. */
  retrieved: number;
  latencyMs: number;
  confidence: number;
  gaps: Gap[];
  sourceDetail: AnswerSource[];
  truncated: boolean;
}

export type QueryIntent = "locate" | "explain" | "relate" | "behaviour" | "general";

/** Pipeline progress, streamed on the `brain-event` channel. */
export type BrainEvent =
  | { type: "planned"; queries: string[]; intent: QueryIntent }
  | { type: "retrieved"; candidates: number; stalePaths: number }
  | { type: "packed"; sources: number; tokens: number; dropped: number }
  | { type: "synthesized"; provider: string; claims: number }
  | { type: "citation"; id: string; path: string; lineStart: number; lineEnd: number; blobSha: string; snippet: string; why: string }
  | { type: "claim"; id: string; kind: ClaimKind; text: string; citations: string[]; supports: string[]; confidence: number }
  | { type: "gap"; kind: GapKind; subject: string; detail: string }
  | { type: "completed"; sources: number; claims: number; latencyMs: number; confidence: number }
  | { type: "cancelled"; stage: string };

export interface BrainStreamPayload {
  streamId: string;
  event: BrainEvent;
}

export interface DependencyNode {
  id: string;
  label: string;
  kind: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  distance: number;
  /** False for a package outside the repository, which has no source to open. */
  resolved: boolean;
}

export interface DependencyEdge {
  kind: string;
  source: string;
  target: string;
  confidence: number;
  incoming: boolean;
}

export interface DependencySlice {
  target: string;
  targetKind: "file" | "symbol";
  nodes: DependencyNode[];
  edges: DependencyEdge[];
  citations: Citation[];
  gaps: Gap[];
  truncated: boolean;
  latencyMs: number;
}
