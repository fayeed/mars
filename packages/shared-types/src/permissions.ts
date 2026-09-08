/**
 * The permission broker as the UI sees it.
 *
 * A pending request is rendered in the task conversation, not in a modal: the
 * agent is stopped, and the question is part of the transcript. A grant is a
 * durable object listed in Settings, where it can be revoked.
 */

export type CommandClass = "read_only" | "mutating" | "destructive" | "network";

export type PermissionDecisionChoice =
  | "allow_once"
  | "allow_task"
  | "allow_pattern"
  | "deny";

export type SideEffect =
  | { kind: "shell"; command: string; cwd: string }
  | { kind: "fileWrite"; path: string; declaredScope: string[] }
  | { kind: "network"; host: string; detail: string }
  | { kind: "gitPush"; remote: string; branch: string; cwd: string };

export interface PendingPermissionRequest {
  id: string;
  taskId: string;
  runId: string | null;
  effect: SideEffect;
  classification: CommandClass;
  summary: string;
  /** Why this was classified the way it was, in words a person can check. */
  reason: string;
  /** What "allow for this pattern" would grant, shown before it is chosen. */
  suggestedPattern: string;
  requestedAt: string;
}

export type PermissionScope =
  | { kind: "repository"; repositoryId: string }
  | { kind: "worktree"; taskId: string; root: string }
  | { kind: "command"; program: string; cwd: string }
  | { kind: "network_host"; host: string }
  | { kind: "remote"; remote: string; branch: string };

export interface PermissionGrant {
  id: string;
  taskId: string | null;
  capability: string;
  scope: PermissionScope;
  pattern: string | null;
  decision: string;
  usesRemaining: number | null;
  grantedBy: string;
  grantedAt: string;
  expiresAt: string | null;
  revokedAt: string | null;
  revokedBy: string | null;
  /** Live now: not revoked, not expired, uses left. */
  active: boolean;
}

/** The ceiling an agent was constructed with. No decision can widen it. */
export interface CapabilityCeiling {
  readFiles: boolean;
  writeFiles: boolean;
  commands: "none" | "read_only" | "mutating" | "destructive";
  network: boolean;
  gitPush: boolean;
}

/** What the broker did with an answer. */
export type PermissionOutcomeView =
  | { outcome: "allowed"; grantId: string; reason: string }
  | {
      outcome: "denied";
      reason: string;
      /** True when the ceiling refused it, which no person can override. */
      byConstruction: boolean;
    }
  | { outcome: "pending"; requestId: string };
