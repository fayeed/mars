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

/** Who pays for a run, as the agent's own CLI reports it. */
export type AgentBilling =
  /** The person's own subscription. `plan` is the tier when the CLI names one. */
  | { type: "plan"; plan: string | null }
  /** An API key, named by where it came from, never by its value. */
  | { type: "apiKey"; source: string }
  | { type: "cloudProvider"; provider: string }
  | { type: "signedOut" }
  /** The CLI did not answer in a form Mars reads. Not a refusal to launch. */
  | { type: "unknown"; detail: string };

export interface AgentAccount {
  kind: AdapterKind;
  /** What a run started now would bill. */
  billing: AgentBilling;
  /** What the CLI is signed in with once `overrides` are set aside. */
  signIn: AgentBilling;
  /** Variables in Mars's environment that outrank `signIn`, highest first. */
  overrides: string[];
  /** Whether the person chose `signIn` over `overrides` for this agent. */
  useSignIn: boolean;
  /** What a person runs in a terminal to sign in, e.g. `claude auth login`. */
  signInCommand: string | null;
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
