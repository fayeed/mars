import type { ContextPack } from "./context-pack";

export interface HandoffEvent {
  sequence: number;
  eventType: "text" | "command" | "file_change" | "test" | "end";
  summary: string;
  sourceLocation: string;
}

export interface ReviewHandoff {
  taskId: string;
  implementationSessionId: string;
  implementationAdapter: "claude" | "codex";
  reviewerAdapter: "claude" | "codex";
  contextPack: ContextPack;
  implementationEvents: HandoffEvent[];
  diff: string;
  diffSha256: string;
  maxBytes: number;
  totalBytes: number;
}

export type ReviewVerdict = "pass" | "changes_requested";
export type ReviewStatus = "candidate" | "accepted" | "dismissed";
export type FindingSeverity = "info" | "warning" | "error" | "blocker";
export type FindingStatus = "open" | "sent" | "resolved" | "dismissed";

export interface ReviewFinding {
  id: string;
  reviewId: string;
  severity: FindingSeverity;
  title: string;
  body: string;
  relativePath: string;
  line: number;
  evidence: string[];
  status: FindingStatus;
  fingerprint: string;
}

export interface ReviewRecord {
  id: string;
  taskId: string;
  implementationSessionId: string;
  reviewerAdapter: "claude" | "codex";
  status: ReviewStatus;
  verdict: ReviewVerdict;
  summary: string;
  handoffDigest: string;
  findings: ReviewFinding[];
  createdAt: string;
  updatedAt: string;
}
