export type WorktreeState =
  | "planned"
  | "provisioning"
  | "ready"
  | "removing"
  | "removed"
  | "orphaned"
  | "failed";

export interface WorktreePlan {
  taskId: string;
  repositoryId: string;
  title: string;
  intent: string;
  branchName: string;
  baseRef: string;
  worktreePath: string;
  state: WorktreeState;
}

export type OverlapKind =
  | "exact_file"
  | "rename"
  | "changed_symbol"
  | "dependency"
  | "unknown_index";

export interface OverlapWarning {
  kind: OverlapKind;
  severity: "error" | "warning" | "info";
  message: string;
  evidence: string[];
  otherTaskId: string;
  otherTaskTitle: string;
  otherWorktreePath: string;
  refreshedAt: string;
}

export type AdapterKind = "claude" | "codex";

export interface AdapterProbe {
  kind: AdapterKind;
  executable: string;
  version: string;
  supportsResume: boolean;
}

export interface AgentPermissions {
  writeFiles: boolean;
  runCommands: boolean;
  network: boolean;
}

export interface AgentLaunchOutcome {
  sessionId: string;
  adapter: AdapterKind;
  version: string;
}

export type NormalizedAgentEvent =
  | { type: "checkpoint"; sessionId: string }
  | { type: "text"; text: string }
  | { type: "tool"; name: string; input: unknown }
  | { type: "command"; command: string; status: string }
  | { type: "file_change"; path: string; change: string }
  | { type: "test"; command: string; status: string }
  | { type: "permission"; tool: string; description: string }
  | { type: "unknown"; providerType: string; raw: unknown }
  | { type: "end"; success: boolean; message: string | null };

export interface AgentTimelinePayload {
  sessionId: string;
  event: NormalizedAgentEvent;
}

export type AgentSessionStatus =
  | "running"
  | "cancelling"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export interface AgentSessionRecord {
  id: string;
  status: AgentSessionStatus;
  pid: number | null;
  exitCode: number | null;
  error: string | null;
}
