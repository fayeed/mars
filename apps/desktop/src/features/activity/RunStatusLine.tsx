import type { ActivityEvent, ReplayedRun } from "@mars/shared-types";
import { SquareIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { eventActivityState, formatDuration } from "@/features/activity/ActivityLog";
import type { ActivityState } from "@/shell/activity-state";
import { Mono, StateDot } from "@/shell/primitives";

export interface RunStatusLineProps {
  run: ReplayedRun | null;
  /** The most recent event, whatever kind it is. */
  latest: ActivityEvent | null;
  /** Present while the run is stopped waiting on a person. */
  blockedSummary?: string | null;
  onInterrupt: () => void;
  interrupting?: boolean;
  /** Overridden in tests so elapsed time is not wall-clock dependent. */
  now?: number;
}

/** How the log's view of a run reads in the shell's state vocabulary. */
export function runActivityState(run: ReplayedRun | null): ActivityState {
  if (!run) return "idle";
  switch (run.state) {
    case "running":
      return "streaming";
    case "blocked":
      return "blocked";
    case "failed":
      return "failed";
    case "completed":
      return "done";
    default:
      return "idle";
  }
}

/** What the agent is doing right now, in the fewest words that are still true. */
function describe(run: ReplayedRun | null, latest: ActivityEvent | null, blocked?: string | null) {
  if (!run) return "No run in progress";
  if (run.state === "blocked") {
    return blocked ?? "Waiting for your answer to a permission request";
  }
  if (run.state === "failed") return run.failure ?? "The run failed";
  if (run.state === "completed") return "Finished";
  if (!latest) return "Starting";
  const state = eventActivityState(latest);
  // An event that is still open describes the present; a finished one is the
  // last thing that happened, and the line says which of those it is.
  return state === "streaming" ? latest.summary : `Last: ${latest.summary}`;
}

/**
 * The persistent "what is happening right now" line.
 *
 * It sits above the composer rather than in the transcript, because the
 * transcript scrolls and this must not: at any moment during a run, the answer
 * to "what is it doing and how do I stop it" is in the same place on screen.
 */
export function RunStatusLine({
  blockedSummary,
  interrupting = false,
  latest,
  now,
  onInterrupt,
  run,
}: RunStatusLineProps) {
  const [tick, setTick] = useState(() => now ?? Date.now());

  const live = run?.state === "running";
  useEffect(() => {
    if (!live || now !== undefined) return;
    const timer = setInterval(() => setTick(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live, now]);

  if (!run || run.state === "completed") return null;

  const state = runActivityState(run);
  const started = Date.parse(run.startedAt);
  const elapsed = Number.isNaN(started) ? null : formatDuration((now ?? tick) - started);

  return (
    <div
      aria-label="Run status"
      aria-live="polite"
      data-slot="run-status"
      className="flex shrink-0 items-center gap-2 border-t border-border bg-surface-raised px-2.5 py-1.5"
    >
      <StateDot state={state} />
      <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">
        {describe(run, latest, blockedSummary)}
      </span>
      {elapsed ? <Mono className="shrink-0 text-faint">{elapsed}</Mono> : null}
      <Mono className="shrink-0 text-faint">seq {run.lastSequence}</Mono>
      {run.state === "running" || run.state === "blocked" ? (
        <button
          type="button"
          disabled={interrupting}
          onClick={onInterrupt}
          className="flex shrink-0 items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-destructive hover:text-destructive focus-visible:border-destructive disabled:cursor-not-allowed disabled:text-faint"
        >
          <SquareIcon className="size-3" />
          {interrupting ? "Interrupting…" : "Interrupt"}
        </button>
      ) : null}
    </div>
  );
}
