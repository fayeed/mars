export type ProvenanceKind =
  | "human"
  | "extracted_code"
  | "extracted_git"
  | "agent_observed"
  | "agent_inferred"
  | "imported"
  | "ambiguous";

export type KnowledgeKind =
  | "summary"
  | "decision"
  | "constraint"
  | "attempt"
  | "risk"
  | "question"
  | "test"
  | "commit";

export type KnowledgeStatus = "candidate" | "approved" | "rejected" | "stale" | "superseded";

export type EntityKind =
  | "project"
  | "repository"
  | "task"
  | "context"
  | "skill"
  | "agent_session"
  | "file"
  | "symbol"
  | "knowledge"
  | "branch"
  | "worktree";

export interface EntityRef {
  kind: EntityKind;
  id: string;
}

export interface KnowledgeRecord {
  id: string;
  projectId: string;
  repositoryId: string | null;
  kind: KnowledgeKind;
  title: string;
  body: string;
  status: KnowledgeStatus;
  confidence: number;
  provenanceId: string;
  provenanceKind: ProvenanceKind;
  resolutionProvenanceId: string | null;
  supersedesId: string | null;
  pinned: boolean;
  createdAt: string;
  updatedAt: string;
}

/** A record with where it came from: the task, the agent, the files. */
export interface KnowledgeItem {
  record: KnowledgeRecord;
  taskId: string | null;
  taskTitle: string | null;
  /** The agent that recorded it, when an agent did. */
  agent: string | null;
  runId: string | null;
  /**
   * `agent`: recorded by an agent while it worked. `extraction`: read out of
   * a finished session. `person`: written or corrected by someone.
   */
  via: "agent" | "extraction" | "person";
  alternatives: string[];
  /** Repository-relative paths it says it affects. */
  files: string[];
}

export type KnowledgeAction = "approve" | "reject" | "correct" | "mark_stale" | "pin" | "unpin";

export type WhyStatus = "answered" | "answered_with_conflicts" | "insufficient_evidence";

export interface WhyFact {
  knowledgeId: string;
  kind: KnowledgeKind;
  title: string;
  body: string;
  confidence: number;
  provenance: string;
}

export interface WhySource {
  label: string;
  location: string;
}

export interface WhyPathStep {
  kind: string;
  label: string;
  location: string;
}

export interface WhyPath {
  relation: string;
  steps: WhyPathStep[];
}

export interface WhyAnswer {
  status: WhyStatus;
  subject: EntityRef;
  facts: WhyFact[];
  inferences: string[];
  warnings: string[];
  sources: WhySource[];
  paths: WhyPath[];
}
