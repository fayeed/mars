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

/** What a run cost, as the provider reported it at the end. */
export interface RunUsage {
  inputTokens?: number;
  outputTokens?: number;
  cachedInputTokens?: number;
  costUsd?: number;
  turns?: number;
}

export type NormalizedAgentEvent =
  | { type: "checkpoint"; sessionId: string }
  | { type: "text"; text: string }
  | { type: "tool"; name: string; input: unknown; callId?: string }
  | { type: "command"; command: string; status: string; callId?: string }
  | { type: "file_change"; path: string; change: string; callId?: string }
  | { type: "test"; command: string; status: string; callId?: string }
  | {
      type: "tool_result";
      callId: string;
      output: string;
      isError: boolean;
      exitCode?: number;
    }
  | { type: "permission"; tool: string; description: string }
  | { type: "warning"; message: string }
  | { type: "unknown"; providerType: string; raw: unknown }
  | { type: "end"; success: boolean; message: string | null; usage?: RunUsage };

/**
 * One agent event as the app emits it. `taskId` and `runId` name where it
 * belongs, so a screen showing one task never reacts to another task's run.
 */
export interface AgentTimelinePayload {
  sessionId: string;
  taskId?: string;
  runId?: string;
  event: NormalizedAgentEvent;
}

/** Where an agent CLI's executable was found. */
export type ExecutableSource = "configured" | "path" | "installer" | "editor_extension";

export interface AgentExecutable {
  adapter: AdapterKind;
  path: string | null;
  source: ExecutableSource | null;
  /** The path chosen in Settings, if any. */
  chosen: string | null;
  /** Where Mars looked when it found nothing. */
  searched: string[];
}

/** A run that was launched, as `start_run` and `continue_run` answer. */
export interface LaunchedRun {
  runId: string;
  sessionId: string;
  adapter: AdapterKind;
  version: string;
  billing: AgentBilling;
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
