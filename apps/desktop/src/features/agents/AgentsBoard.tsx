import type { RunOverview } from "@mars/shared-types";
import { SquareIcon } from "lucide-react";
import { useMemo } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateDot, StateLabel } from "@/shell/primitives";

export interface AgentsBoardProps {
  runs: RunOverview[];
  state: ActivityState;
  /** Milliseconds since the epoch; injected so tests are not tied to the clock. */
  now: number;
  selectedTaskId?: string | null;
  onOpenTask: (taskId: string) => void;
  onStop: (runId: string) => void;
  stoppingRunId?: string | null;
}

/** How one run reads in the shell's state vocabulary. */
export function runOverviewState(run: RunOverview): ActivityState {
  if (run.live) return run.pendingPermissions > 0 ? "blocked" : "streaming";
  switch (run.status) {
    case "starting":
    case "running":
    case "blocked":
    case "interrupting":
      // The record says it was going, but nothing is reading it any more.
      return "stale";
    case "completed":
      return "done";
    case "failed":
      return "failed";
    case "interrupted":
      return "stale";
    default:
      return "idle";
  }
}

function statusText(run: RunOverview): string {
  if (run.live) return run.pendingPermissions > 0 ? "waiting on you" : "working";
  switch (run.status) {
    case "completed":
      return "finished";
    case "cancelled":
      return "stopped";
    case "interrupted":
      return "interrupted";
    case "failed":
      return run.error ? `failed: ${run.error}` : "failed";
    default:
      return "no longer running";
  }
}

export function formatDuration(milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

function usageOf(run: RunOverview) {
  const usage = run.usage ?? null;
  const tokens = (usage?.inputTokens ?? 0) + (usage?.outputTokens ?? 0);
  return { tokens, cost: usage?.costUsd ?? null };
}

export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}k`;
  return String(tokens);
}

interface Lane {
  taskId: string;
  title: string;
  branch: string;
  runs: RunOverview[];
  latest: number;
  /** 0 when an agent here is waiting on you, 1 when one is working, else 2. */
  rank: number;
}

/**
 * Runs grouped into one lane per task, each lane's runs in the order they
 * started.
 */
export function lanesOf(runs: RunOverview[]): Lane[] {
  const byTask = new Map<string, Lane>();
  for (const run of runs) {
    const started = Date.parse(run.startedAt);
    const lane = byTask.get(run.taskId) ?? {
      taskId: run.taskId,
      title: run.taskTitle,
      branch: run.branchName,
      runs: [],
      latest: 0,
      rank: 2,
    };
    lane.runs.push(run);
    lane.latest = Math.max(lane.latest, started);
    lane.rank = Math.min(lane.rank, run.live ? (run.pendingPermissions > 0 ? 0 : 1) : 2);
    byTask.set(run.taskId, lane);
  }
  for (const lane of byTask.values()) {
    lane.runs.sort((left, right) => Date.parse(left.startedAt) - Date.parse(right.startedAt));
  }
  // Lanes with an agent waiting on you first, then lanes with one working,
  // then the rest by how recently anything started in them.
  return [...byTask.values()].sort(
    (left, right) => left.rank - right.rank || right.latest - left.latest,
  );
}

const LANE_HEIGHT = 28;

/**
 * Every agent in the project at once.
 *
 * The timeline is the picture: one lane per task, one bar per run, laid on a
 * shared clock so it is plain which agents worked in parallel, which one is
 * still going, and which is stopped waiting on you. A follow-up is joined to
 * the run it continued; a reviewer's bar is hollow because it cannot change
 * anything. The list below says the same things in words, with what each run
 * cost.
 */
export function AgentsBoard({
  now,
  onOpenTask,
  onStop,
  runs,
  selectedTaskId,
  state,
  stoppingRunId,
}: AgentsBoardProps) {
  const lanes = useMemo(() => lanesOf(runs), [runs]);
  const window = useMemo(() => {
    if (runs.length === 0) return { start: now - 60_000, end: now };
    const starts = runs.map((run) => Date.parse(run.startedAt));
    const ends = runs.map((run) =>
      run.live || !run.endedAt ? now : Date.parse(run.endedAt),
    );
    const start = Math.min(...starts);
    const end = Math.max(now, ...ends);
    return { start, end: end === start ? start + 60_000 : end };
  }, [now, runs]);

  const working = runs.filter((run) => run.live && run.pendingPermissions === 0).length;
  const waiting = runs.filter((run) => run.live && run.pendingPermissions > 0).length;
  const totals = runs.reduce(
    (sum, run) => {
      const { cost, tokens } = usageOf(run);
      return { tokens: sum.tokens + tokens, cost: sum.cost + (cost ?? 0) };
    },
    { tokens: 0, cost: 0 },
  );

  if (runs.length === 0) {
    return (
      <StateBlock
        state={state === "loading" ? "loading" : state === "failed" ? "failed" : "idle"}
        title={state === "failed" ? "Runs could not be read" : "No agent has run here yet"}
        description="Start one from a task's conversation. Every run in this repository — working, waiting or finished — appears here on one timeline."
      />
    );
  }

  const span = window.end - window.start;
  const x = (time: number) => ((time - window.start) / span) * 100;

  return (
    <div className="flex flex-col" data-slot="agents-board">
      <div
        aria-label="Summary"
        className="flex flex-wrap items-center gap-4 border-b border-border px-3 py-2"
        role="group"
      >
        <StateLabel state={working > 0 ? "streaming" : "idle"} detail={`${working} working`} />
        <StateLabel state={waiting > 0 ? "blocked" : "idle"} detail={`${waiting} waiting on you`} />
        <Mono className="text-muted-foreground">{runs.length} run(s)</Mono>
        {totals.tokens > 0 ? (
          <Mono className="text-muted-foreground">{formatTokens(totals.tokens)} tokens</Mono>
        ) : null}
        {totals.cost > 0 ? (
          <Mono className="text-muted-foreground">${totals.cost.toFixed(2)} reported</Mono>
        ) : null}
      </div>

      <section aria-label="Timeline" className="border-b border-border px-3 py-2">
        <div className="mb-1 flex items-baseline justify-between">
          <MetaLabel>Timeline</MetaLabel>
          <Mono className="text-faint">
            {formatDuration(span)} · solid writes · hollow reviews · ▸ follow-up
          </Mono>
        </div>
        <ul className="flex flex-col">
          {lanes.map((lane) => (
            <li
              key={lane.taskId}
              className={cn(
                "grid grid-cols-[minmax(8rem,14rem)_1fr] items-center gap-2 border-t border-border first:border-t-0",
                lane.taskId === selectedTaskId && "bg-surface-raised",
              )}
              style={{ height: LANE_HEIGHT }}
            >
              <button
                type="button"
                className="min-w-0 truncate text-left text-[12px] text-foreground outline-none hover:underline focus-visible:underline"
                onClick={() => onOpenTask(lane.taskId)}
                title={`${lane.title} · ${lane.branch}`}
              >
                {lane.title}
              </button>
              <svg
                aria-label={`${lane.title}: ${lane.runs.length} run(s)`}
                className="h-4 w-full overflow-visible"
                preserveAspectRatio="none"
                role="img"
                viewBox="0 0 100 16"
              >
                <line
                  stroke="var(--border)"
                  strokeWidth={0.4}
                  vectorEffect="non-scaling-stroke"
                  x1={0}
                  x2={100}
                  y1={8}
                  y2={8}
                />
                {lane.runs.map((run, index) => {
                  const start = x(Date.parse(run.startedAt));
                  const endTime = run.live || !run.endedAt ? now : Date.parse(run.endedAt);
                  const width = Math.max(0.6, x(endTime) - start);
                  const runState = runOverviewState(run);
                  const color =
                    runState === "failed"
                      ? "var(--destructive)"
                      : runState === "streaming" || runState === "blocked"
                        ? "var(--primary)"
                        : "var(--muted-foreground)";
                  const reviewer = run.role === "reviewer";
                  const previous = lane.runs[index - 1];
                  const continues =
                    previous !== undefined && run.resumesRunId === previous.id;
                  return (
                    <g data-run={run.id} data-state={runState} key={run.id}>
                      {continues && previous ? (
                        <line
                          stroke={color}
                          strokeDasharray="1 1"
                          strokeWidth={1}
                          vectorEffect="non-scaling-stroke"
                          x1={x(previous.endedAt ? Date.parse(previous.endedAt) : now)}
                          x2={start}
                          y1={8}
                          y2={8}
                        />
                      ) : null}
                      <rect
                        className={runState === "streaming" ? "shell-pulse" : undefined}
                        fill={reviewer ? "transparent" : color}
                        height={runState === "blocked" ? 10 : 8}
                        rx={0.5}
                        stroke={color}
                        strokeDasharray={runState === "blocked" ? "2 1" : undefined}
                        strokeWidth={1}
                        vectorEffect="non-scaling-stroke"
                        width={width}
                        x={start}
                        y={runState === "blocked" ? 3 : 4}
                      >
                        <title>{`${run.adapterKind} ${run.role} · ${statusText(run)}`}</title>
                      </rect>
                    </g>
                  );
                })}
              </svg>
            </li>
          ))}
        </ul>
      </section>

      <section aria-label="Runs">
        <ul className="flex flex-col">
          {[...runs]
            .sort((left, right) => {
              // Live first, the ones waiting on you before the ones working.
              const rank = (run: RunOverview) =>
                run.live ? (run.pendingPermissions > 0 ? 0 : 1) : 2;
              return (
                rank(left) - rank(right) ||
                Date.parse(right.startedAt) - Date.parse(left.startedAt)
              );
            })
            .map((run) => {
              const runState = runOverviewState(run);
              const end = run.live || !run.endedAt ? now : Date.parse(run.endedAt);
              const { cost, tokens } = usageOf(run);
              return (
                <li
                  className="flex flex-col gap-1 border-b border-border px-3 py-2"
                  data-run={run.id}
                  key={run.id}
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <StateDot state={runState} />
                    <span className="text-[12px] text-foreground">
                      {run.adapterKind} · {run.role}
                    </span>
                    <button
                      type="button"
                      className="min-w-0 truncate text-[12px] text-muted-foreground outline-none hover:text-foreground hover:underline focus-visible:underline"
                      onClick={() => onOpenTask(run.taskId)}
                    >
                      {run.taskTitle}
                    </button>
                    <span
                      className={cn(
                        "text-[11px]",
                        runState === "blocked" ? "text-primary" : "text-faint",
                      )}
                    >
                      {statusText(run)}
                    </span>
                    {run.live ? (
                      <button
                        type="button"
                        className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:text-faint"
                        disabled={stoppingRunId === run.id}
                        onClick={() => onStop(run.id)}
                      >
                        <SquareIcon className="size-3" />
                        {stoppingRunId === run.id ? "Stopping" : "Stop"}
                      </button>
                    ) : null}
                  </div>
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5">
                    <Mono className="text-faint">{run.branchName || "no branch"}</Mono>
                    <Mono className="text-faint">{formatDuration(end - Date.parse(run.startedAt))}</Mono>
                    {run.filesTouched > 0 ? (
                      <Mono className="text-faint">{run.filesTouched} file(s)</Mono>
                    ) : null}
                    {tokens > 0 ? (
                      <Mono className="text-faint">{formatTokens(tokens)} tokens</Mono>
                    ) : null}
                    {cost !== null ? <Mono className="text-faint">${cost.toFixed(2)}</Mono> : null}
                    {run.resumesRunId ? <Mono className="text-faint">follow-up</Mono> : null}
                  </div>
                </li>
              );
            })}
        </ul>
      </section>
    </div>
  );
}
