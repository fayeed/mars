import type { BaseDrift, DriftChoice, OverlapWarning } from "@mars/shared-types";
import { AlertTriangleIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface BaseDriftMessageProps {
  drift: BaseDrift;
  /** What the moved commits touch that this task also touches. */
  overlaps?: OverlapWarning[];
  onResolve: (choice: DriftChoice) => void;
  busy?: boolean;
  error?: string | null;
}

/**
 * The base moved under a task that was already planned against it.
 *
 * This is stated rather than repaired: Mars pins the commit a plan was built
 * on, so it can tell the difference between "the ref points somewhere else now"
 * and "the plan changed", and only a person decides which of those matters. All
 * three ways out are offered together with what each one costs, because the
 * cheapest-looking option here is also the one that can throw work away.
 */
export function BaseDriftMessage({
  busy = false,
  drift,
  error,
  onResolve,
  overlaps = [],
}: BaseDriftMessageProps) {
  return (
    <article
      aria-label="Base moved"
      data-slot="base-drift"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · the base moved</MetaLabel>
        <StateLabel state="stale" detail={`${drift.commitsBehind} commit(s) behind`} />
      </header>

      <div className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <span className="w-24 shrink-0 text-[11px] text-faint">Base</span>
          <Mono className="text-foreground">{drift.baseRef}</Mono>
        </div>
        <div className="flex items-baseline gap-2">
          <span className="w-24 shrink-0 text-[11px] text-faint">Planned on</span>
          <Mono className="text-muted-foreground">{drift.plannedOid.slice(0, 12)}</Mono>
        </div>
        <div className="flex items-baseline gap-2">
          <span className="w-24 shrink-0 text-[11px] text-faint">Now at</span>
          <Mono className="text-primary">{drift.currentOid.slice(0, 12)}</Mono>
        </div>
      </div>

      <section className="flex flex-col gap-1">
        <MetaLabel>What it touches</MetaLabel>
        {overlaps.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            Nothing this task has touched overlaps with what moved, as far as the index can tell.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {overlaps.map((overlap) => (
              <li
                key={`${overlap.kind}:${overlap.otherTaskId}:${overlap.message}`}
                className={cn(
                  "flex flex-col gap-0.5 border-l-2 py-1 pl-2",
                  overlap.severity === "error" ? "border-destructive" : "border-primary",
                )}
              >
                <div className="flex flex-wrap items-baseline gap-2">
                  <Mono
                    className={
                      overlap.severity === "error" ? "text-destructive" : "text-primary"
                    }
                  >
                    {overlap.kind}
                  </Mono>
                  <span className="min-w-0 flex-1 text-[11px] text-foreground">
                    {overlap.message}
                  </span>
                </div>
                {overlap.evidence.length > 0 ? (
                  <Mono className="text-faint">{overlap.evidence.join(", ")}</Mono>
                ) : null}
                <Mono className="text-faint">
                  also in {overlap.otherTaskTitle} · refreshed {overlap.refreshedAt}
                </Mono>
              </li>
            ))}
          </ul>
        )}
      </section>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <section className="flex flex-col gap-1.5">
        <MetaLabel>How to resolve it</MetaLabel>
        <ul className="flex flex-col gap-1">
          {drift.options.map((option) => (
            <li key={option.choice}>
              <button
                type="button"
                disabled={busy}
                onClick={() => onResolve(option.choice)}
                className={cn(
                  "flex w-full flex-col gap-0.5 border border-border px-2 py-1.5 text-left outline-none",
                  "hover:border-border-strong focus-visible:border-border-strong",
                  "disabled:cursor-not-allowed disabled:text-faint",
                )}
              >
                <span
                  className={cn(
                    "flex items-center gap-1.5 text-[12px]",
                    option.choice === "recreate" ? "text-destructive" : "text-foreground",
                  )}
                >
                  {option.choice === "recreate" ? (
                    <AlertTriangleIcon className="size-3 shrink-0" />
                  ) : null}
                  {option.label}
                </span>
                <span className="text-[11px] leading-relaxed text-muted-foreground">
                  {option.consequence}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <p className="text-[11px] text-faint">
          Nothing happens until you choose. The plan still pins the commit it was built on.
        </p>
      </section>
    </article>
  );
}
