import type { CheckRun, TaskProjection } from "@mars/shared-types";
import { PlayIcon, RefreshCwIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { formatDuration } from "@/features/activity/ActivityLog";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateDot } from "@/shell/primitives";

/** A diagnostic, wherever it came from. */
export interface Problem {
  path: string;
  line: number | null;
  severity: "error" | "warning";
  message: string;
  /** What reported it — a command, a test, the index. */
  source: string;
}

export interface ProblemsPanelProps {
  problems: Problem[];
  state: ActivityState;
  onOpen: (path: string, line: number | null) => void;
}

/**
 * Diagnostics, grouped by the file they are about.
 *
 * Mars runs no language server of its own, so everything here was reported by
 * something that actually ran — a command, a test, the indexer. The source is
 * on every row because a warning from a linter and a panic from a test run are
 * not the same kind of claim.
 */
export function ProblemsPanel({ onOpen, problems, state }: ProblemsPanelProps) {
  if (state === "loading") return <StateBlock state="loading" title="Collecting problems" />;
  if (problems.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No problems"
        description="Nothing that has run in this worktree reported one. Mars does not analyse the code on its own, so this is a record of what was run, not a clean bill of health."
      />
    );
  }

  const byFile = new Map<string, Problem[]>();
  for (const problem of problems) {
    const list = byFile.get(problem.path) ?? [];
    list.push(problem);
    byFile.set(problem.path, list);
  }

  return (
    <ul aria-label="Problems" className="flex flex-col">
      {[...byFile.entries()].map(([path, items]) => (
        <li key={path}>
          <div className="flex items-baseline gap-2 border-b border-border bg-surface px-3 py-1">
            <Mono className="min-w-0 flex-1 truncate text-foreground">{path}</Mono>
            <Mono className="shrink-0 text-faint">{items.length}</Mono>
          </div>
          <ul>
            {items.map((problem, index) => (
              <li key={`${problem.message}:${index}`}>
                <button
                  type="button"
                  onClick={() => onOpen(problem.path, problem.line)}
                  className="flex w-full items-baseline gap-2 border-b border-border px-3 py-1 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
                >
                  <StateDot
                    className="self-center"
                    state={problem.severity === "error" ? "failed" : "blocked"}
                  />
                  <Mono className="w-10 shrink-0 text-right text-faint">
                    {problem.line ?? ""}
                  </Mono>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-foreground">
                    {problem.message}
                  </span>
                  <Mono className="shrink-0 text-faint">{problem.source}</Mono>
                </button>
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ul>
  );
}

export interface TestsPanelProps {
  projection: TaskProjection | null;
  state: ActivityState;
  onRerun: (command: string) => void;
  onOpenOutput: (digest: string) => void;
}

/** Test runs the record holds, newest last, with what each one actually did. */
export function TestsPanel({ onOpenOutput, onRerun, projection, state }: TestsPanelProps) {
  const tests = projection?.tests ?? [];
  if (state === "loading") return <StateBlock state="loading" title="Reading test runs" />;
  if (tests.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No test runs recorded"
        description="Nothing has run tests in this task yet. An agent that says the tests pass without a run here is making a claim the record does not support."
      />
    );
  }

  return (
    <ul aria-label="Tests" className="flex flex-col">
      {tests.map((test) => {
        const passed = test.status === "passed" || test.status === "completed";
        return (
          <li
            key={test.sequence}
            className="flex items-baseline gap-2 border-b border-border px-3 py-1"
          >
            <StateDot className="self-center" state={passed ? "done" : "failed"} />
            <Mono className={cn("min-w-0 flex-1 truncate", passed ? "text-foreground" : "text-destructive")}>
              {test.command}
            </Mono>
            <Mono className="shrink-0 text-faint">{test.status}</Mono>
            <Mono className="w-12 shrink-0 text-right text-faint">
              {formatDuration(test.durationMs) ?? ""}
            </Mono>
            {test.resultDigest ? (
              <button
                type="button"
                onClick={() => onOpenOutput(test.resultDigest as string)}
                className="shrink-0 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
              >
                Output
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => onRerun(test.command)}
              className="flex shrink-0 items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              <PlayIcon className="size-3" />
              Re-run
            </button>
          </li>
        );
      })}
    </ul>
  );
}

export interface ChecksPanelProps {
  checks: CheckRun[];
  state: ActivityState;
  onRefresh: () => void;
  onOpenUrl: (url: string) => void;
}

/** How a forge check reads in the shell's vocabulary. */
function checkState(check: CheckRun): ActivityState {
  if (check.status !== "completed") return "streaming";
  if (check.conclusion === "success") return "done";
  if (check.conclusion === null) return "idle";
  return "failed";
}

/** CI as the forge reports it, with a way to the log for each run. */
export function ChecksPanel({ checks, onOpenUrl, onRefresh, state }: ChecksPanelProps) {
  if (state === "loading") return <StateBlock state="loading" title="Reading checks" />;
  if (checks.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No checks"
        description="Checks appear once a pull request exists and the forge has scheduled them."
        action={
          <button
            type="button"
            onClick={onRefresh}
            className="flex items-center gap-1 border border-border px-2 py-1 text-[11px] hover:border-border-strong"
          >
            <RefreshCwIcon className="size-3" />
            Refresh
          </button>
        }
      />
    );
  }

  return (
    <div className="flex flex-col">
      <div className="flex items-baseline gap-2 border-b border-border px-3 py-1">
        <MetaLabel>Checks</MetaLabel>
        <Mono className="text-faint">{checks.length}</Mono>
        <button
          type="button"
          onClick={onRefresh}
          className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
        >
          <RefreshCwIcon className="size-3" />
          Refresh
        </button>
      </div>
      <ul aria-label="Check runs" className="flex flex-col">
        {checks.map((check) => (
          <li
            key={check.providerId}
            className="flex items-baseline gap-2 border-b border-border px-3 py-1"
          >
            <StateDot className="self-center" state={checkState(check)} />
            <span className="min-w-0 flex-1 truncate text-[11px] text-foreground">{check.name}</span>
            <Mono className="shrink-0 text-faint">{check.conclusion ?? check.status}</Mono>
            <button
              type="button"
              onClick={() => onOpenUrl(check.url)}
              className="shrink-0 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              Logs
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Problems, read from what actually ran.
 *
 * Errors recorded during the run and failed commands become rows; nothing is
 * invented from static analysis Mars does not do.
 */
export function problemsFromProjection(projection: TaskProjection | null): Problem[] {
  if (!projection) return [];
  const problems: Problem[] = [];

  for (const command of projection.commands) {
    if (command.status === "failed" || (command.exitCode !== null && command.exitCode !== 0)) {
      problems.push({
        path: command.command,
        line: null,
        severity: "error",
        message: `exited ${command.exitCode ?? "non-zero"}`,
        source: "command",
      });
    }
  }
  for (const message of projection.errors) {
    problems.push({ path: "run", line: null, severity: "error", message, source: "agent" });
  }
  return problems;
}
