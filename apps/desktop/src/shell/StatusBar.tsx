import type { ReactNode } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { Mono, StateDot } from "@/shell/primitives";

export interface StatusBarProps {
  repository?: string;
  worktree?: string;
  branch?: string;
  index?: { state: ActivityState; detail?: string };
  agents?: { state: ActivityState; count: number };
  permissions?: { state: ActivityState; pending: number };
  connection?: ReactNode;
}

function Field({
  children,
  label,
  state,
}: {
  children: ReactNode;
  label: string;
  state?: ActivityState;
}) {
  return (
    <span className="flex items-center gap-1.5">
      {state ? <StateDot state={state} /> : null}
      <span className="text-faint">{label}</span>
      <Mono className="text-muted-foreground">{children}</Mono>
    </span>
  );
}

/**
 * One line, always true: where you are on disk, whether the index can be
 * trusted, what is running, and what is waiting on your answer.
 */
export function StatusBar({
  agents,
  branch,
  connection,
  index,
  permissions,
  repository,
  worktree,
}: StatusBarProps) {
  return (
    <footer
      aria-label="Status"
      className="flex h-[var(--shell-status-height)] shrink-0 items-center gap-4 border-t border-border bg-surface px-2 text-[11px]"
    >
      <Field label="Repo">{repository ?? "—"}</Field>
      {worktree ? <Field label="Worktree">{worktree}</Field> : null}
      <Field label="Branch">{branch ?? "—"}</Field>
      {index ? (
        <Field label="Index" state={index.state}>
          {index.detail ?? "—"}
        </Field>
      ) : null}
      {agents ? (
        <Field label="Agents" state={agents.state}>
          {agents.count}
        </Field>
      ) : null}
      {permissions ? (
        <Field label="Permissions" state={permissions.state}>
          {permissions.pending === 0 ? "none pending" : `${permissions.pending} pending`}
        </Field>
      ) : null}
      <div className="ml-auto flex items-center gap-2">{connection}</div>
    </footer>
  );
}
