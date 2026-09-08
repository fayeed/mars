/**
 * The `Agent` interface as the desktop sees it: one vocabulary over every
 * backend, and capabilities that are a property of how the agent was built.
 */

import type { CapabilityCeiling } from "./permissions";
import type { AdapterKind } from "./tasks";
import type { RunRole, RunStatus } from "./activity";

export interface AgentCapabilities {
  adapter: AdapterKind;
  role: RunRole;
  /** Cannot be widened by any decision a person makes during the run. */
  ceiling: CapabilityCeiling;
  supportsResume: boolean;
  supportsInterrupt: boolean;
}

/** The nine kinds every backend is normalized onto, plus two for machinery. */
export type AgentEvent =
  | { type: "message"; text: string }
  | { type: "thought"; text: string }
  | { type: "tool_call"; name: string; input: unknown }
  | { type: "command"; command: string; status: string }
  | { type: "file_edit"; path: string; change: string }
  | { type: "test_run"; command: string; status: string }
  | {
      type: "permission_request";
      requestId: string;
      summary: string;
      classification: string;
    }
  | { type: "error"; message: string }
  | { type: "done"; success: boolean; message: string | null }
  /** A provider resume point, kept for follow-up runs. */
  | { type: "checkpoint"; sessionId: string }
  /** A frame no adapter recognised, preserved rather than dropped. */
  | { type: "unknown"; providerType: string; raw: unknown };

export interface AgentStatus {
  runId: string;
  sessionId: string | null;
  state: RunStatus;
  lastSequence: number;
  /** Set while the agent is stopped waiting on a decision. */
  blockedOnRequest: string | null;
}

export interface ToolGrant {
  name: string;
  description: string;
  readOnly: boolean;
}

export interface StandingGrant {
  capability: string;
  pattern: string | null;
  decision: string;
  grantedBy: string;
  expiresAt: string | null;
}

export interface PermissionsView {
  role: string;
  ceiling: CapabilityCeiling;
  declaredScope: string[];
  standingGrants: StandingGrant[];
  /** Always true: anything not listed stops the agent and asks. */
  everythingElseAsks: boolean;
}

export interface RunPackEntry {
  section: string;
  kind: string;
  entityRef: string;
  title: string;
  provenance: string;
  freshness: string;
  sourceLocation: string;
  reason: string;
  byteSize: number;
}

/**
 * Exactly what an agent will see, shown before the run starts. `prompt` is the
 * string the agent is launched with, and `digest` is over that string, so the
 * preview and the launch can be compared byte for byte.
 */
export interface RunContextPack {
  taskId: string;
  agentId: string;
  role: string;
  adapter: string;
  entries: RunPackEntry[];
  sectionUsage: { section: string; maxBytes: number; usedBytes: number }[];
  exclusions: { entityRef: string; reason: string }[];
  tools: ToolGrant[];
  permissions: PermissionsView;
  prompt: string;
  totalBytes: number;
  digest: string;
}

/**
 * What to assemble for a run. The role picks the capability ceiling in the
 * core; the declared scope is read from the approved worktree plan. Neither is
 * something this side gets to assert.
 */
export interface RunPackInput {
  taskId: string;
  agentId: string;
  adapter: AdapterKind;
  role: RunRole;
  /** What the person actually asked for, verbatim. */
  assignment: string;
  maxBytes: number;
  excludedEntityRefs?: string[];
}
