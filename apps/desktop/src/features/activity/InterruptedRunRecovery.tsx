import type { InterruptedRun, RecoveryAction } from "@mars/shared-types";
import { AlertTriangleIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface InterruptedRunRecoveryProps {
  run: InterruptedRun;
  /** What the log says the run finished before it stopped. */
  completed: { label: string; detail: string }[];
  onChoose: (action: RecoveryAction) => void;
  busy?: boolean;
  error?: string | null;
}

function Group({ children, count, title }: { children: React.ReactNode; count?: number; title: string }) {
  return (
    <section className="flex flex-col gap-1.5">
      <div className="flex items-baseline gap-2">
        <MetaLabel>{title}</MetaLabel>
        {count === undefined ? null : <Mono className="text-faint">{count}</Mono>}
      </div>
      {children}
    </section>
  );
}

/**
 * Mars stopped in the middle of a run, and this is what the log says was true
 * when it did.
 *
 * The four groups are deliberately separate, because they carry different
 * certainty: what completed is known, what was in flight was started and never
 * reported, what is on disk was checked just now, and what is unverified is
 * stated as plainly as the log can state it. Nothing resumes on its own — there
 * is no default action here, and each way out says what it costs.
 */
export function InterruptedRunRecovery({
  busy = false,
  completed,
  error,
  onChoose,
  run,
}: InterruptedRunRecoveryProps) {
  return (
    <article
      aria-label="Interrupted run"
      data-slot="interrupted-run"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · interrupted run</MetaLabel>
        <StateLabel state="stale" detail={`stopped while ${run.statusWhenStopped}`} />
        <Mono className="ml-auto text-faint">{run.runId.slice(0, 12)}</Mono>
      </header>

      <p className="text-[12px] leading-relaxed text-muted-foreground">
        Mars stopped while <span className="text-foreground">{run.agentId}</span> was working as{" "}
        {run.role}. Nothing has been resumed, and nothing will be until you choose.
      </p>

      <Group title="Completed" count={completed.length}>
        {completed.length === 0 ? (
          <p className="text-[11px] text-faint">Nothing had finished.</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {completed.map((item) => (
              <li key={`${item.label}:${item.detail}`} className="flex items-baseline gap-2">
                <Mono className="min-w-0 flex-1 truncate text-muted-foreground">{item.label}</Mono>
                <Mono className="shrink-0 text-faint">{item.detail}</Mono>
              </li>
            ))}
          </ul>
        )}
      </Group>

      <Group title="In flight when it stopped" count={run.inFlight.length}>
        {run.inFlight.length === 0 ? (
          <p className="text-[11px] text-faint">Nothing was mid-operation.</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {run.inFlight.map((item) => (
              <li key={item.sequence} className="flex items-baseline gap-2">
                <span className="w-24 shrink-0 text-[11px] text-faint">{item.kind}</span>
                <Mono className="min-w-0 flex-1 truncate text-foreground">{item.summary}</Mono>
                <Mono className="shrink-0 text-faint">seq {item.sequence}</Mono>
              </li>
            ))}
          </ul>
        )}
      </Group>

      <Group title="Written to disk" count={run.filesWritten.length}>
        {run.filesWritten.length === 0 ? (
          <p className="text-[11px] text-faint">No file writes were recorded.</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {run.filesWritten.map((file) => (
              <li key={file.path} className="flex items-baseline gap-2">
                <Mono className="min-w-0 flex-1 truncate text-foreground">{file.path}</Mono>
                <Mono className="shrink-0 text-faint">{file.change}</Mono>
                <Mono className="w-24 shrink-0 text-right text-faint">
                  {file.presentOnDisk === null
                    ? "not checked"
                    : file.presentOnDisk
                      ? "on disk"
                      : "missing"}
                </Mono>
              </li>
            ))}
          </ul>
        )}
      </Group>

      <Group title="Unverified">
        <ul className="flex flex-col gap-0.5">
          {run.unverified.map((statement) => (
            <li key={statement} className="flex items-baseline gap-1.5">
              <AlertTriangleIcon className="size-3 shrink-0 self-center text-primary" />
              <span className="text-[11px] leading-relaxed text-muted-foreground">{statement}</span>
            </li>
          ))}
        </ul>
      </Group>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <Group title="What happens next is your call">
        <ul className="flex flex-col gap-1">
          {run.options.map((option) => (
            <li key={option.action}>
              <button
                type="button"
                disabled={busy}
                onClick={() => onChoose(option.action)}
                className={cn(
                  "flex w-full flex-col gap-0.5 border border-border px-2 py-1.5 text-left outline-none",
                  "hover:border-border-strong focus-visible:border-border-strong",
                  "disabled:cursor-not-allowed disabled:text-faint",
                )}
              >
                <span
                  className={cn(
                    "text-[12px]",
                    option.action === "rerun" ? "text-destructive" : "text-foreground",
                  )}
                >
                  {option.label}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {option.consequence}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </Group>
    </article>
  );
}
