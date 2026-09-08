/**
 * Human-approved, traceable memory.
 *
 * Two rules shape these types. Every memory was approved by a person, which is
 * why `approvedBy` and `approvedAt` are not optional. Every memory can be
 * traced to the conversation turn that motivated it, which is why `origin`
 * carries the exact excerpt rather than a summary of it.
 *
 * A `MemoryProposal` is not a memory. It is what an agent queued, and it can
 * never reach an agent's context.
 */

export type MemoryScope = "repo" | "task" | "global";

export type MemoryKind = "convention" | "decision" | "preference" | "fact" | "caution";

export type MemoryStatus = "active" | "superseded" | "revoked";

export type ProposalStatus = "pending" | "approved" | "rejected";

export interface MemoryOrigin {
  conversationId: string;
  messageId: string;
  /** The exact excerpt that motivated the proposal. */
  quotedText: string;
  /**
   * Whether the excerpt was checked against that turn. False means the trace
   * is recorded but unconfirmed, and the UI should say so rather than imply
   * more certainty than exists.
   */
  verified: boolean;
}

export interface MemoryProposal {
  id: string;
  projectId: string;
  repositoryId: string | null;
  taskId: string | null;
  scope: MemoryScope;
  kind: MemoryKind;
  text: string;
  confidence: number;
  origin: MemoryOrigin;
  proposedBy: string;
  proposedAt: string;
  status: ProposalStatus;
  resolvedBy: string | null;
  resolvedAt: string | null;
  rejectionReason: string | null;
  memoryId: string | null;
}

export interface Memory {
  id: string;
  projectId: string;
  repositoryId: string | null;
  taskId: string | null;
  scope: MemoryScope;
  kind: MemoryKind;
  text: string;
  confidence: number;
  origin: MemoryOrigin;
  proposalId: string;
  approvedBy: string;
  approvedAt: string;
  supersedes: string | null;
  status: MemoryStatus;
  revokedBy: string | null;
  revokedAt: string | null;
  revocationReason: string | null;
  /** How many times this memory has been injected into an agent's context. */
  useCount: number;
  /** The figure behind "used in 4 tasks". */
  usedInTasks: number;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryUse {
  id: string;
  memoryId: string;
  taskId: string | null;
  sessionId: string | null;
  surface: string;
  relevance: number;
  usedAt: string;
}

export interface MemoryUsage {
  memoryId: string;
  useCount: number;
  usedInTasks: number;
  uses: MemoryUse[];
}

export type MemoryEventKind =
  | "proposed"
  | "approved"
  | "edited_and_approved"
  | "rejected"
  | "revised"
  | "superseded"
  | "revoked"
  | "conflict_resolved";

export interface MemoryEvent {
  id: string;
  proposalId: string | null;
  memoryId: string | null;
  kind: MemoryEventKind;
  actor: string;
  conversationId: string;
  messageId: string;
  detail: string;
  occurredAt: string;
}

/**
 * Why two memories were shown together. Neither value claims a contradiction
 * was detected; they narrow the field to the pairs a person should look at.
 */
export type ConflictSignal = "overlap" | "opposing_polarity";

export interface DetectedConflict {
  other: Memory;
  similarity: number;
  signal: ConflictSignal;
  explanation: string;
}

export type ConflictChoice =
  | "keep_both"
  | "supersede_other"
  | "revoke_other"
  | "keep_existing";

export interface ConflictResolution {
  otherMemoryId: string;
  choice: ConflictChoice;
}

/** The result of asking to approve a proposal. */
export type ApprovalOutcome =
  | { outcome: "approved"; memory: Memory; message: MemoryEvent }
  | { outcome: "conflictsRequireChoice"; conflicts: DetectedConflict[] }
  | { outcome: "keptExisting"; proposal: MemoryProposal; kept: string[] };

export interface NewProposal {
  projectId: string;
  repositoryId?: string | null;
  taskId?: string | null;
  scope: MemoryScope;
  kind: MemoryKind;
  text: string;
  confidence: number;
  conversationId: string;
  messageId: string;
  quotedText: string;
  proposedBy: string;
}

export interface ProposalFilters {
  projectId: string;
  repositoryId?: string | null;
  taskId?: string | null;
  statuses?: ProposalStatus[];
  limit?: number;
}

export interface ApproveRequest {
  proposalId: string;
  /**
   * Assigned by the backend from the local user. Anything sent here is
   * discarded: who approved a memory is not the client's to assert.
   */
  approvedBy?: string;
  /** Present when the human edited before accepting. */
  editedText?: string | null;
  resolutions?: ConflictResolution[];
}

export interface RejectRequest {
  proposalId: string;
  /** Assigned by the backend; see `ApproveRequest.approvedBy`. */
  rejectedBy?: string;
  reason: string;
}

export interface ReviseRequest {
  memoryId: string;
  text: string;
  /** Assigned by the backend; see `ApproveRequest.approvedBy`. */
  revisedBy?: string;
  kind?: MemoryKind | null;
}

export interface RevokeRequest {
  memoryId: string;
  /** Assigned by the backend; see `ApproveRequest.approvedBy`. */
  revokedBy?: string;
  reason: string;
}

export interface MemoryFilters {
  projectId: string;
  repositoryId?: string | null;
  taskId?: string | null;
  scopes?: MemoryScope[];
  kinds?: MemoryKind[];
  /** Empty means active only. */
  statuses?: MemoryStatus[];
  contains?: string | null;
  limit?: number;
}

export interface RetrievalContext {
  projectId: string;
  repositoryId?: string | null;
  taskId?: string | null;
  sessionId?: string | null;
  query: string;
  surface: string;
  limit?: number;
  /** False previews the selection without recording a use. */
  recordUses?: boolean;
}

export interface RetrievedMemory {
  memory: Memory;
  relevance: number;
  reason: string;
}
