/**
 * The worktree plan a person approves before Mars touches Git.
 *
 * Everything here is produced without mutating anything, and the fingerprint is
 * what approval is recorded against — so the dialog and the provisioning step
 * are talking about the same plan.
 */

import type { WorktreeState } from "./tasks";

export interface SetupCommand {
  command: string;
  purpose: string;
  /** How the command classifier read it, so the risk is visible up front. */
  classification: string;
}

export type PreflightCode =
  | "base_moved"
  | "dirty_base"
  | "worktree_exists"
  | "branch_exists"
  | "low_disk"
  | "risky_setup_command";

export type PreflightSeverity = "warning" | "blocking";

export interface Resolution {
  action: string;
  /** What choosing this does. Never omitted. */
  consequence: string;
}

export interface PreflightFinding {
  code: PreflightCode;
  severity: PreflightSeverity;
  detail: string;
  resolutions: Resolution[];
}

export interface DiskReport {
  /** null when free space could not be read, rather than assumed. */
  availableBytes: number | null;
  thresholdBytes: number;
  low: boolean;
}

export interface WorktreeProposal {
  taskId: string;
  repositoryId: string;
  title: string;
  intent: string;
  baseRef: string;
  /** The commit the plan pinned, not what the ref points at now. */
  baseOid: string;
  branchName: string;
  worktreePath: string;
  expectedScope: string[];
  setupCommands: SetupCommand[];
  preflight: PreflightFinding[];
  disk: DiskReport;
  state: WorktreeState;
  /** Digest over what the plan decides. Approval is recorded against it. */
  fingerprint: string;
  approvedBy: string | null;
  approvedAt: string | null;
}

export type DriftChoice = "rebase" | "recreate" | "continue_and_diverge";

export interface DriftOption {
  choice: DriftChoice;
  label: string;
  consequence: string;
}

export interface BaseDrift {
  baseRef: string;
  plannedOid: string;
  currentOid: string;
  commitsBehind: number;
  options: DriftOption[];
}

export interface CleanupPlan {
  taskId: string;
  worktreePath: string;
  branchName: string;
  branchOid: string | null;
  /** Where the branch tip is archived, so removal can be undone. */
  archiveRef: string;
  /** Uncommitted files removal would discard. These are what make it final. */
  uncommitted: string[];
  commitsAheadOfBase: number;
  reversible: boolean;
  fingerprint: string;
}

export interface CleanupRecord {
  id: string;
  taskId: string;
  removedPath: string;
  branchName: string;
  archivedRef: string | null;
  archivedOid: string | null;
  state: string;
}

/**
 * A setup command as the client asks for it. The classification is deliberately
 * absent: how risky a command is is decided by the core's classifier, never
 * asserted by the UI.
 */
export interface SetupCommandInput {
  command: string;
  purpose: string;
}

export interface PlanTaskInput {
  repositoryId: string;
  title: string;
  intent: string;
  baseRef: string;
  expectedScope?: string[];
  setupCommands?: SetupCommandInput[];
}
