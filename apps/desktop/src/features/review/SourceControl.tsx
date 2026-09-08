import type { WorktreePlan } from "@mars/shared-types";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateDot } from "@/shell/primitives";

export interface SourceControlProps {
  /** The diff review for the selected task. */
  diff: ReactNode;
  tasks: WorktreePlan[];
  selectedTaskId: string | null;
  onSelectTask: (taskId: string) => void;
  worktreeState: (state: WorktreePlan["state"]) => ActivityState;
}

/**
 * Source control as Mars means it: not one working copy, but several.
 *
 * A task owns a branch and a worktree, so "what has changed" is always a
 * question about which one. The list names them all with the state each is
 * actually in — a worktree that failed to provision is not the same as one
 * nobody has started — and the change itself fills the rest of the space.
 *
 * There is no staging area here. Mars does not stage on a person's behalf, and
 * a control that implied it did would be claiming an action nothing performs.
 */
export function SourceControl({
  diff,
  onSelectTask,
  selectedTaskId,
  tasks,
  worktreeState,
}: SourceControlProps) {
  return (
    <div className="flex h-full min-h-0">
      <aside
        aria-label="Branches and worktrees"
        className="flex w-56 shrink-0 flex-col border-r border-border bg-surface"
      >
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
            {tasks.map((task) => (
              <li key={task.taskId}>
                <button
                  type="button"
                  aria-current={task.taskId === selectedTaskId ? "true" : undefined}
                  onClick={() => onSelectTask(task.taskId)}
                  className={cn(
                    "relative flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none",
                    "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
                    "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
                    task.taskId === selectedTaskId && "bg-surface-raised before:bg-primary",
                  )}
                >
                  <span className="flex items-center gap-1.5">
                    <StateDot state={worktreeState(task.state)} />
                    <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                      {task.title}
                    </span>
                  </span>
                  <Mono className="truncate text-faint">{task.branchName}</Mono>
                  <span className="flex items-baseline gap-2">
                    <Mono className="text-faint">from {task.baseRef}</Mono>
                    <Mono className="ml-auto text-faint">{task.state}</Mono>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </aside>

      <div className="min-h-0 min-w-0 flex-1">{diff}</div>
    </div>
  );
}
