import type { IndexStatus, StartingPoint } from "@mars/shared-types";
import { ArrowUpRightIcon, RefreshCwIcon } from "lucide-react";

import type { ActivityState } from "@/shell/activity-state";
import type { ShellTask } from "@/shell/model";
import { MetaLabel, Mono, StateBlock, StateDot, StateLabel } from "@/shell/primitives";

export interface RepositoryHomeProps {
  repositoryName: string;
  index: IndexStatus | null;
  indexState: ActivityState;
  onIndex: () => void;
  startingPoints: StartingPoint[];
  startingPointsState: ActivityState;
  /** Puts the suggestion's prompt in the composer rather than sending it. */
  onUseStartingPoint: (prompt: string) => void;
  tasks: ShellTask[];
  onSelectTask: (taskId: string) => void;
}

/**
 * What the index is, in the shell's own vocabulary.
 *
 * An index that has read nothing is idle rather than done, and an index the
 * working tree has moved out from under is stale — not failed. The difference
 * matters: stale answers are still answers, and saying so is the point.
 */
export function indexActivityState(status: IndexStatus | null): ActivityState {
  if (!status) return "loading";
  if (status.runningRuns > 0) return "loading";
  if (status.counts.files === 0) return "idle";
  return status.headMatchesIndex ? "done" : "stale";
}

function indexDetail(status: IndexStatus | null): string {
  if (!status) return "";
  if (status.counts.files === 0) return "nothing indexed";
  const { files, symbols } = status.counts;
  return `${files.toLocaleString()} files · ${symbols.toLocaleString()} symbols`;
}

function Group({ children, title }: { children: React.ReactNode; title: string }) {
  return (
    <section className="flex flex-col gap-2 border-b border-border px-3 py-3">
      <MetaLabel>{title}</MetaLabel>
      {children}
    </section>
  );
}

/**
 * The landing surface: a conversation with nothing in it yet.
 *
 * There is no "select a file to begin" here and no dashboard standing between a
 * person and the composer. What is on screen is what would help them type the
 * first message — whether the index can be trusted, what they were doing last,
 * and a few grounded places to start — and all of it is above a composer that
 * is already live.
 */
export function RepositoryHome({
  index,
  indexState,
  onIndex,
  onSelectTask,
  onUseStartingPoint,
  repositoryName,
  startingPoints,
  startingPointsState,
  tasks,
}: RepositoryHomeProps) {
  const state = indexState === "loading" ? "loading" : indexActivityState(index);
  const recent = tasks.slice(0, 5);

  return (
    <div className="flex flex-col">
      <div className="flex flex-col gap-1.5 px-3 py-4">
        <MetaLabel>{repositoryName}</MetaLabel>
        <h1 className="font-heading text-lg tracking-tight text-foreground">
          Ask for a change, or pick up where you left off.
        </h1>
        <p className="max-w-prose text-[12px] leading-relaxed text-muted-foreground">
          Describe what you want in the composer below. Mars will propose a branch and a worktree,
          show you the context and permissions it would give the agent, and wait for you to approve
          both before anything runs.
        </p>
      </div>

      <Group title="Index">
        <div className="flex flex-wrap items-center gap-2">
          <StateLabel state={state} detail={indexDetail(index)} />
          {index?.branch ? <Mono className="text-faint">{index.branch}</Mono> : null}
          <button
            type="button"
            onClick={onIndex}
            disabled={state === "loading"}
            className="ml-auto flex items-center gap-1.5 border border-border px-2 py-1 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <RefreshCwIcon className="size-3" />
            {index && index.counts.files > 0 ? "Re-index" : "Index now"}
          </button>
        </div>
        {state === "stale" ? (
          <p className="text-[11px] text-muted-foreground">
            The working tree moved to a different commit after the last run. Answers will be drawn
            from what was read then, and every one of them will say so.
          </p>
        ) : null}
        {state === "idle" ? (
          <p className="text-[11px] text-muted-foreground">
            Nothing has been read yet. The conversation works either way, but questions about this
            code have nothing to draw on until an index exists.
          </p>
        ) : null}
      </Group>

      <Group title="Starting points">
        {startingPointsState === "loading" ? (
          <StateBlock className="h-auto py-3" state="loading" title="Looking for somewhere to start" />
        ) : startingPoints.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            Nothing to suggest yet. That is not a problem — describe what you want below.
          </p>
        ) : (
          <ul className="flex flex-col gap-px">
            {startingPoints.map((point) => (
              <li key={point.id}>
                <button
                  type="button"
                  onClick={() => onUseStartingPoint(point.prompt)}
                  className="group flex w-full flex-col gap-1 border border-transparent px-2 py-1.5 text-left outline-none hover:border-border hover:bg-surface-raised focus-visible:border-border focus-visible:bg-surface-raised"
                >
                  <span className="flex items-center gap-1.5">
                    <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                      {point.title}
                    </span>
                    <ArrowUpRightIcon className="size-3 shrink-0 text-faint opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
                  </span>
                  <span className="text-[11px] leading-relaxed text-muted-foreground">
                    {point.reason}
                  </span>
                  <Mono className="text-faint">{point.source}</Mono>
                </button>
              </li>
            ))}
          </ul>
        )}
      </Group>

      <Group title="Recent tasks">
        {recent.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No tasks yet. The first message you send will propose one.
          </p>
        ) : (
          <ul className="flex flex-col gap-px">
            {recent.map((task) => (
              <li key={task.id}>
                <button
                  type="button"
                  onClick={() => onSelectTask(task.id)}
                  className="flex w-full items-center gap-2 border border-transparent px-2 py-1.5 text-left outline-none hover:border-border hover:bg-surface-raised focus-visible:border-border focus-visible:bg-surface-raised"
                >
                  <StateDot state={task.state} />
                  <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
                    {task.title}
                  </span>
                  {task.branch ? <Mono className="shrink-0 text-faint">{task.branch}</Mono> : null}
                </button>
              </li>
            ))}
          </ul>
        )}
      </Group>
    </div>
  );
}
