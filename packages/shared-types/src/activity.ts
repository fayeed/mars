/**
 * The activity record: the append-only, per-task log the UI renders from.
 *
 * Every surface that shows what a task did — the timeline, the diff summary,
 * the permission conversation, the crash-recovery dialog — is a projection of
 * these events. Reload replays them; nothing is cached that the log does not
 * say.
 */

export type ActivityKind =
  | "message"
  | "thought"
  | "tool_call"
  | "command"
  | "file_edit"
  | "test_run"
  | "permission_request"
  | "permission_decision"
  | "error"
  | "done"
  | "run_started"
  | "worktree_planned"
  | "worktree_created"
  | "worktree_removed"
  | "worktree_restored"
  | "recovery_noted";

export type ActivityActor = "human" | "agent" | "system";

export interface ActivityEvent {
  id: string;
  taskId: string;
  /** Monotonic within a task, assigned on write. Never reused, never reordered. */
  sequence: number;
  runId: string | null;
  kind: ActivityKind;
  actor: ActivityActor;
  actorLabel: string;
  summary: string;
  payload: unknown;
  /**
   * Set when the payload was too large to inline; resolve it through the
   * payload endpoint rather than expecting it in `payload`.
   */
  payloadDigest: string | null;
  occurredAt: string;
  durationMs: number | null;
}

export type RunRole = "implementer" | "reviewer";

export type RunStatus =
  | "starting"
  | "running"
  /** Stopped, waiting for a human to answer a permission request. */
  | "blocked"
  | "interrupting"
  | "completed"
  | "failed"
  | "cancelled"
  /** Alive when Mars stopped. Never resumed automatically. */
  | "interrupted";

export interface RunRecord {
  id: string;
  taskId: string;
  sessionId: string | null;
  agentId: string;
  adapterKind: string;
  role: RunRole;
  capabilities: unknown;
  contextPackDigest: string | null;
  status: RunStatus;
  lastSequence: number;
  error: string | null;
  startedAt: string;
  endedAt: string | null;
}

export type ReplayedRunState = "running" | "blocked" | "completed" | "failed";

export interface ReplayedRun {
  runId: string;
  actorLabel: string;
  state: ReplayedRunState;
  startedAt: string;
  endedAt: string | null;
  firstSequence: number;
  lastSequence: number;
  messageCount: number;
  blockedOnRequest: string | null;
  failure: string | null;
}

export interface PendingPermissionView {
  requestId: string;
  runId: string | null;
  summary: string;
  classification: string | null;
  requestedAt: string;
  sequence: number;
}

export interface DecidedPermissionView {
  requestId: string;
  decision: string;
  decidedBy: string;
  decidedAt: string;
  sequence: number;
}

export interface FileTouch {
  path: string;
  change: string;
  runId: string | null;
  lastSequence: number;
  edits: number;
}

export interface CommandRun {
  command: string;
  status: string;
  exitCode: number | null;
  durationMs: number | null;
  outputDigest: string | null;
  runId: string | null;
  sequence: number;
}

export interface TestOutcome {
  command: string;
  status: string;
  durationMs: number | null;
  resultDigest: string | null;
  sequence: number;
}

export interface WorktreeViewFromLog {
  branchName: string | null;
  baseRef: string | null;
  baseOid: string | null;
  path: string | null;
  state: string;
  sequence: number;
}

/** The whole of a task's state, folded from its log. */
export interface TaskProjection {
  taskId: string;
  lastSequence: number;
  eventCount: number;
  runs: ReplayedRun[];
  pendingPermissions: PendingPermissionView[];
  decidedPermissions: DecidedPermissionView[];
  filesTouched: FileTouch[];
  commands: CommandRun[];
  tests: TestOutcome[];
  errors: string[];
  worktree: WorktreeViewFromLog | null;
}

export type RecoveryAction = "resume" | "discard" | "rerun";

export interface RecoveryOption {
  action: RecoveryAction;
  label: string;
  /** What taking this option does. Always shown next to the choice. */
  consequence: string;
}

export interface InFlightItem {
  kind: string;
  summary: string;
  sequence: number;
  startedAt: string;
}

export interface WrittenFile {
  path: string;
  change: string;
  edits: number;
  lastSequence: number;
  /** null when the worktree path is unknown, so presence was not checked. */
  presentOnDisk: boolean | null;
}

export interface InterruptedRun {
  runId: string;
  taskId: string;
  agentId: string;
  adapterKind: string;
  role: string;
  statusWhenStopped: string;
  startedAt: string;
  lastEventAt: string | null;
  lastSequence: number;
  inFlight: InFlightItem[];
  filesWritten: WrittenFile[];
  /** Statements the log cannot confirm. Shown verbatim. */
  unverified: string[];
  options: RecoveryOption[];
}

/**
 * Command output, diffs and test results, resolved from the digest an event
 * carries. Kept out of the event row so the log stays pageable.
 */
export interface ActivityPayload {
  digest: string;
  byteSize: number;
  /** Lossy UTF-8. Every payload the record spills is program output. */
  text: string;
}
