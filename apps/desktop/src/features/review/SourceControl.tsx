import type { WorktreePlan, WorktreeProposal } from "@mars/shared-types";
import { useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateDot } from "@/shell/primitives";

import { GitHubPanel } from "../forge/GitHubPanel";

import { GitPanel } from "./GitPanel";
import { IntegratePanel } from "./IntegratePanel";

export interface SourceControlProps {
  repositoryId: string;
  repositoryName: string;
  /** The diff review for the selected task: its change since its base. */
  diff: ReactNode;
  tasks: WorktreePlan[];
  selectedTaskId: string | null;
  onSelectTask: (taskId: string) => void;
  worktreeState: (state: WorktreePlan["state"]) => ActivityState;
  /** Called after anything changes a working directory or its history. */
  onChanged?: () => void;
  onOpenFile?: (path: string, line: number | null) => void;
  /** The branch a task planned from an issue starts from. */
  baseBranch: string;
  /** Which part is open first; the screen remounts this to open another. */
  initialScope?: SourceScope;
  onTaskPlanned?: (proposal: WorktreeProposal) => void;
}

export type SourceScope = "repository" | "task" | "github";

type TaskView = "review" | "git" | "merge";

/**
 * Source control as Mars means it: not one working copy, but several.
 *
 * The repository you opened is one; every task's worktree is another, on its
 * own branch. Each can be read and changed the way a terminal would — status,
 * staging, commits, history, branches, sync — and a task can be reviewed
 * against where it started and merged home when it is done.
 */
export function SourceControl({
  baseBranch,
  diff,
  initialScope,
  onChanged,
  onOpenFile,
  onSelectTask,
  onTaskPlanned,
  repositoryId,
  repositoryName,
  selectedTaskId,
  tasks,
  worktreeState,
}: SourceControlProps) {
  const [scope, setScope] = useState<SourceScope>(
    initialScope ?? (selectedTaskId ? "task" : "repository"),
  );
  const [view, setView] = useState<TaskView>("review");
  const task = scope === "task" ? tasks.find((candidate) => candidate.taskId === selectedTaskId) : undefined;

  return (
    <div className="flex h-full min-h-0">
      <aside
        aria-label="Branches and worktrees"
        className="flex w-56 shrink-0 flex-col border-r border-border bg-surface"
      >
        <button
          type="button"
          aria-current={scope === "repository" ? "true" : undefined}
          onClick={() => setScope("repository")}
          className={cn(
            "relative flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none",
            "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
            "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
            scope === "repository" && "bg-surface-raised before:bg-primary",
          )}
        >
          <MetaLabel>Repository</MetaLabel>
          <span className="truncate text-[12px] text-foreground">{repositoryName}</span>
        </button>
        <button
          type="button"
          aria-current={scope === "github" ? "true" : undefined}
          onClick={() => setScope("github")}
          className={cn(
            "relative flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none",
            "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
            "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
            scope === "github" && "bg-surface-raised before:bg-primary",
          )}
        >
          <MetaLabel>GitHub</MetaLabel>
          <span className="truncate text-[12px] text-foreground">Issues and pull requests</span>
        </button>
        <div className="flex items-baseline gap-2 border-b border-border px-2 py-1.5">
          <MetaLabel>Worktrees</MetaLabel>
          <Mono className="text-faint">{tasks.length}</Mono>
        </div>
        {tasks.length === 0 ? (
          <p className="px-2 py-2 text-[11px] text-muted-foreground">
            No task has a worktree yet. Each one gets its own branch and its own checkout.
          </p>
        ) : (
          <ul className="min-h-0 flex-1 overflow-y-auto">
            {tasks.map((candidate) => (
              <li key={candidate.taskId}>
                <button
                  type="button"
                  aria-current={
                    scope === "task" && candidate.taskId === selectedTaskId ? "true" : undefined
                  }
                  onClick={() => {
                    setScope("task");
                    onSelectTask(candidate.taskId);
                  }}
                  className={cn(
                    "relative flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none",
                    "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
                    "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
                    scope === "task" &&
                      candidate.taskId === selectedTaskId &&
                      "bg-surface-raised before:bg-primary",
                  )}
                >
                  <span className="flex items-center gap-1.5">
                    <StateDot state={worktreeState(candidate.state)} />
                    <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                      {candidate.title}
                    </span>
                  </span>
                  <Mono className="truncate text-faint">{candidate.branchName}</Mono>
                  <span className="flex items-baseline gap-2">
                    <Mono className="text-faint">from {candidate.baseRef}</Mono>
                    <Mono className="ml-auto text-faint">{candidate.state}</Mono>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {scope === "github" ? (
          <GitHubPanel
            baseBranch={baseBranch}
            onTaskPlanned={onTaskPlanned}
            repositoryId={repositoryId}
          />
        ) : scope === "repository" ? (
          <GitPanel
            key={`repository:${repositoryId}`}
            onChanged={onChanged}
            onOpenFile={onOpenFile}
            target={{ kind: "repository", repositoryId }}
          />
        ) : !task ? (
          <StateBlock state="idle" title="Pick a worktree" />
        ) : (
          <>
            <div
              className="flex items-center gap-1 border-b border-border px-2 py-1"
              role="tablist"
              aria-label="Worktree views"
            >
              {(
                [
                  ["review", "Review"],
                  ["git", "Git"],
                  ["merge", "Merge"],
                ] as const
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={view === id}
                  onClick={() => setView(id)}
                  className={cn(
                    "border px-2 py-0.5 text-[11px] outline-none",
                    view === id
                      ? "border-primary text-foreground"
                      : "border-border text-muted-foreground hover:border-border-strong",
                  )}
                >
                  {label}
                </button>
              ))}
              <Mono className="ml-auto truncate text-faint">{task.branchName}</Mono>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              {view === "review" ? (
                diff
              ) : view === "git" ? (
                task.state === "ready" ? (
                  <GitPanel
                    key={`task:${task.taskId}`}
                    onChanged={onChanged}
                    onOpenFile={onOpenFile}
                    target={{ kind: "task", taskId: task.taskId }}
                  />
                ) : (
                  <StateBlock
                    state={worktreeState(task.state)}
                    title="This task's worktree is not there"
                    description={`It is ${task.state}.`}
                  />
                )
              ) : (
                <IntegratePanel
                  key={task.taskId}
                  onChanged={onChanged}
                  taskId={task.taskId}
                  taskTitle={task.title}
                  worktreeReady={task.state === "ready"}
                />
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
