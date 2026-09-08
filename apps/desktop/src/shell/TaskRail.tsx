import { PanelLeftIcon, PlusIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { summarizeStates } from "@/shell/activity-state";
import { bindingHint } from "@/shell/keymap";
import type { ShellTask } from "@/shell/model";
import {
  CollapsedStrip,
  Mono,
  PanelHeader,
  RailButton,
  StateBlock,
  StateDot,
} from "@/shell/primitives";
import { Region } from "@/shell/Region";
import { useShell } from "@/shell/shell-store";

export interface TaskRailProps {
  tasks: ShellTask[];
  selectedTaskId?: string | null;
  onSelectTask?: (taskId: string) => void;
  onCreateTask?: () => void;
  /** Slot for a screen-supplied action, such as a create-task dialog trigger. */
  headerAction?: ReactNode;
  /** State of the rail itself — loading the list, or failing to. */
  state?: ActivityState;
  onRetry?: () => void;
}

export function TaskRail({
  headerAction,
  onCreateTask,
  onRetry,
  onSelectTask,
  selectedTaskId,
  state = "idle",
  tasks,
}: TaskRailProps) {
  const { dispatch, layout } = useShell();
  const rollup = summarizeStates(tasks.map((task) => task.state));
  const active = tasks.filter((task) => task.state === "streaming" || task.state === "loading");

  if (layout.taskRailCollapsed) {
    return (
      <CollapsedStrip
        label={`Tasks ${tasks.length}`}
        onExpand={() => dispatch({ type: "setTaskRail", collapsed: false })}
        side="left"
        state={tasks.length > 0 ? rollup : undefined}
      >
        <div className="flex flex-col items-center gap-1 pb-1" aria-hidden="true">
          {active.slice(0, 4).map((task) => (
            <StateDot key={task.id} state={task.state} />
          ))}
        </div>
      </CollapsedStrip>
    );
  }

  return (
    <Region
      as="aside"
      label="Tasks"
      region="task-rail"
      className="w-[var(--shell-task-rail-width)] shrink-0 border-r border-border bg-surface"
    >
      <PanelHeader
        title="Tasks"
        state={tasks.length > 0 ? rollup : undefined}
        actions={
          <>
            {headerAction}
            {onCreateTask ? (
              <RailButton marker="none" size="strip" title="New task" onClick={onCreateTask}>
                <PlusIcon />
                <span className="sr-only">New task</span>
              </RailButton>
            ) : null}
            <RailButton
              marker="none"
              size="strip"
              aria-expanded
              title={`Collapse tasks · ${bindingHint("toggle.taskRail")}`}
              onClick={() => dispatch({ type: "setTaskRail", collapsed: true })}
            >
              <PanelLeftIcon />
              <span className="sr-only">Collapse tasks</span>
            </RailButton>
          </>
        }
      />

      {state === "loading" || state === "failed" ? (
        <StateBlock
          state={state}
          title={state === "loading" ? "Loading tasks" : "Tasks could not be listed"}
          description={
            state === "failed" ? "The core did not answer. Nothing was changed." : undefined
          }
          action={
            state === "failed" && onRetry ? (
              <button
                type="button"
                className="border border-border px-2 py-1 text-[11px] hover:border-border-strong"
                onClick={onRetry}
              >
                Retry
              </button>
            ) : null
          }
        />
      ) : tasks.length === 0 ? (
        <StateBlock
          state="idle"
          title="No tasks yet"
          description="A task owns one branch and one worktree. Start one to give an agent somewhere to work."
        />
      ) : (
        <ul className="min-h-0 flex-1 overflow-y-auto" role="list">
          {tasks.map((task) => {
            const current = task.id === selectedTaskId;
            return (
              <li key={task.id}>
                <button
                  type="button"
                  aria-current={current ? "true" : undefined}
                  onClick={() => onSelectTask?.(task.id)}
                  className={cn(
                    "relative flex w-full flex-col gap-1 border-b border-border px-2.5 py-2 text-left outline-none",
                    "before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-transparent",
                    "hover:bg-surface-raised focus-visible:shadow-[inset_0_0_0_1px_var(--ring)]",
                    current && "bg-surface-raised before:bg-primary",
                  )}
                >
                  <span className="flex items-center gap-1.5">
                    <StateDot state={task.state} />
                    <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                      {task.title}
                    </span>
                  </span>
                  <span className="flex items-center gap-2 pl-3">
                    {task.branch ? (
                      <Mono className="truncate text-faint">{task.branch}</Mono>
                    ) : null}
                    {task.detail ? (
                      <Mono className="ml-auto shrink-0 text-faint">{task.detail}</Mono>
                    ) : null}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </Region>
  );
}
