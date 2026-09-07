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
