import type { ReactNode } from "react";

import type { ActivityState } from "@/shell/activity-state";
import type { InspectorTab, UtilityTab } from "@/shell/shell-store";

/** One row in the task rail. */
export interface ShellTask {
  id: string;
  title: string;
  state: ActivityState;
  /** Machine-authored trailing detail: branch, elapsed time, file count. */
  branch?: string;
  detail?: string;
  agent?: string;
}

/** What an inspector or utility tab reports before it is opened. */
export interface TabStatus {
  state: ActivityState;
  count?: number;
}

export type InspectorStatuses = Partial<Record<InspectorTab, TabStatus>>;
export type UtilityStatuses = Partial<Record<UtilityTab, TabStatus>>;

export type InspectorContent = Partial<Record<InspectorTab, ReactNode>>;
export type UtilityContent = Partial<Record<UtilityTab, ReactNode>>;

/** An agent the composer can address. */
export interface ComposerAgent {
  id: string;
  name: string;
  /** Human-readable scope of what this agent may already do without asking. */
  permissionScope: string;
  /** Capabilities that will still stop and ask. */
  asks?: string[];
}

/** An `@` mention target: a file or a symbol the index already knows. */
export interface MentionTarget {
  id: string;
  label: string;
  /** Repository-relative path, or `path:line` for a symbol. */
  detail: string;
  kind: "file" | "symbol";
}

/** A `/` skill the composer can invoke. */
export interface ComposerSkill {
  id: string;
  name: string;
  description: string;
  /**
   * The skill's own instruction text, verbatim. Picking a skill hands an agent
   * instructions written by somebody else, so the composer can show exactly
   * what those instructions say before it is picked — not a summary of them.
   */
  instructions?: string;
}

/**
 * Where the agent will be allowed to work, chosen before the message is sent.
 *
 * The globs are the scope, not a description of it: they are what the worktree
 * plan will declare and what the permission broker will measure a write
 * against, so they are shown verbatim.
 */
export interface ComposerScope {
  id: string;
  label: string;
  globs: string[];
  detail?: string;
}

export interface ComposerAttachment {
  id: string;
  label: string;
  detail?: string;
}

export interface ComposerSubmission {
  text: string;
  agentId: string;
  attachmentIds: string[];
  /** Absent when the screen offered no scopes to choose between. */
  scopeId?: string;
}
