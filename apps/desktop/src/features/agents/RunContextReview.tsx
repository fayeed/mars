import type { CapabilityCeiling, RunContextPack, RunPackEntry } from "@mars/shared-types";
import {
  AlertTriangleIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  MinusIcon,
  PlayIcon,
  PlusIcon,
} from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface RunContextReviewProps {
  pack: RunContextPack | null;
  state: ActivityState;
  error?: string | null;
  /** Entity refs this person removed, as opposed to ones the budget dropped. */
  removed: string[];
  onToggleEntry: (entityRef: string, included: boolean) => void;
  onStart: (digest: string) => void;
  busy?: boolean;
}

/**
 * The order sections are read in, and what each one is for. Grouping by kind
 * rather than by section is deliberate: "which memories are being injected" is
 * a question people ask, and it is not answerable from a flat list.
 */
const KIND_LABEL: Record<string, string> = {
  memory: "Memories",
  file: "Files",
  symbol: "Symbols",
  skill: "Skills",
  context: "Attachments",
  task: "Task",
};

const KIND_ORDER = ["memory", "task", "skill", "file", "symbol", "context"];

/**
 * A byte figure, and the approximate token figure people think in.
 *
 * The budget the core enforces is in bytes, so bytes lead and the token count
 * is marked as the estimate it is. Four bytes per token is the usual rule of
 * thumb for source text; it is not a measurement and does not pretend to be.
 */
function bytes(value: number) {
  return value < 1024 ? `${value} B` : `${(value / 1024).toFixed(1)} KiB`;
}

function approxTokens(value: number) {
  return `≈${Math.round(value / 4).toLocaleString()} tok`;
}

function ceilingRows(ceiling: CapabilityCeiling) {
  return [
    { label: "Read files", value: ceiling.readFiles ? "yes" : "no", danger: false },
    { label: "Write files", value: ceiling.writeFiles ? "yes" : "no", danger: ceiling.writeFiles },
    {
      label: "Commands",
      value: ceiling.commands,
      danger: ceiling.commands === "destructive",
    },
    { label: "Network", value: ceiling.network ? "yes" : "no", danger: ceiling.network },
    { label: "Push", value: ceiling.gitPush ? "yes" : "no", danger: ceiling.gitPush },
  ];
}

function Entry({
  entry,
  onRemove,
}: {
  entry: RunPackEntry;
  onRemove: () => void;
}) {
  return (
    <li className="flex flex-col gap-1 border-b border-border px-2 py-1.5 last:border-b-0">
      <div className="flex items-baseline gap-2">
        <span className="min-w-0 flex-1 truncate text-[12px] text-foreground">{entry.title}</span>
        <Mono className="shrink-0 text-faint">{bytes(entry.byteSize)}</Mono>
        <Mono className="shrink-0 text-faint">{approxTokens(entry.byteSize)}</Mono>
        <button
          type="button"
          aria-label={`Remove ${entry.title}`}
          onClick={onRemove}
          className="shrink-0 text-faint outline-none hover:text-destructive focus-visible:text-destructive"
        >
          <MinusIcon className="size-3" />
        </button>
      </div>
      <p className="text-[11px] leading-relaxed text-muted-foreground">{entry.reason}</p>
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <Mono className="text-faint">{entry.provenance}</Mono>
        <Mono className="text-faint">{entry.freshness}</Mono>
        <Mono className="min-w-0 truncate text-faint">{entry.sourceLocation}</Mono>
      </div>
    </li>
  );
}

/**
 * Exactly what the agent will see, and exactly what it may do, before it runs.
 *
 * This is the screen that makes supervision real, so nothing here is
 * summarised: every included item carries where it came from, how fresh it is,
 * why it was chosen and what it costs; every memory carries the conversation
 * turn it was approved from; and the prompt that will actually be sent is
 * readable verbatim. The digest is over that prompt, so what was reviewed and
 * what gets sent can be compared byte for byte.
 */
export function RunContextReview({
  busy = false,
  error,
  onStart,
  onToggleEntry,
  pack,
  removed,
  state,
}: RunContextReviewProps) {
  const [showPrompt, setShowPrompt] = useState(false);

  const grouped = useMemo(() => {
    if (!pack) return [];
    const buckets = new Map<string, RunPackEntry[]>();
    for (const entry of pack.entries) {
      const list = buckets.get(entry.kind) ?? [];
      list.push(entry);
      buckets.set(entry.kind, list);
    }
    return [...buckets.entries()].sort(
      ([a], [b]) =>
        (KIND_ORDER.indexOf(a) + 1 || KIND_ORDER.length + 1) -
        (KIND_ORDER.indexOf(b) + 1 || KIND_ORDER.length + 1),
    );
  }, [pack]);

  if (state === "loading" || !pack) {
    return (
      <StateBlock
        state={state === "failed" ? "failed" : "loading"}
        title={state === "failed" ? "The context pack could not be assembled" : "Assembling context"}
        description={
          state === "failed"
            ? error ?? "Nothing was started. The run cannot begin without a pack you have seen."
            : "Retrieving what this task needs, then measuring it against the budget."
        }
      />
    );
  }

  const mutatingTools = pack.tools.filter((tool) => !tool.readOnly);

  return (
    <article
      data-slot="run-context-review"
      aria-label="Context pack and capability review"
      className="flex flex-col"
    >
      <header className="flex flex-col gap-1.5 border-b border-border px-3 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <MetaLabel className="text-foreground">Mars · context and capabilities</MetaLabel>
          <StateLabel state="blocked" detail="waiting for you" />
          <Mono className="ml-auto text-faint">{pack.digest.slice(0, 12)}</Mono>
        </div>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          This is everything <span className="text-foreground">{pack.agentId}</span> will see as{" "}
          <span className="text-foreground">{pack.permissions.role}</span>, and everything it may do.
          Remove anything it does not need. Nothing runs until you start it.
        </p>
        <div className="flex flex-wrap items-baseline gap-2">
          <Mono className="text-foreground">{bytes(pack.totalBytes)}</Mono>
          <Mono className="text-faint">{approxTokens(pack.totalBytes)}</Mono>
          <span className="text-[11px] text-faint">
            budgeted in bytes; the token figure is an estimate
          </span>
        </div>
      </header>

      <section className="flex flex-col gap-1.5 border-b border-border px-3 py-2">
        <MetaLabel>Budget by section</MetaLabel>
        <ul className="flex flex-col gap-1">
          {pack.sectionUsage.map((usage) => {
            const ratio = usage.maxBytes === 0 ? 0 : usage.usedBytes / usage.maxBytes;
            return (
              <li key={usage.section} className="flex items-center gap-2">
                <span className="w-20 shrink-0 text-[11px] text-muted-foreground">
                  {usage.section}
                </span>
                <span className="h-1 min-w-0 flex-1 bg-surface-raised" aria-hidden="true">
                  <span
                    className={cn("block h-full", ratio > 0.9 ? "bg-primary" : "bg-muted-foreground")}
                    style={{ width: `${Math.min(100, Math.round(ratio * 100))}%` }}
                  />
                </span>
                <Mono className="shrink-0 text-faint">
                  {bytes(usage.usedBytes)} / {bytes(usage.maxBytes)}
                </Mono>
              </li>
            );
          })}
        </ul>
      </section>

      <section className="flex flex-col">
        {grouped.map(([kind, entries]) => (
          <div key={kind} className="border-b border-border">
            <div className="flex items-baseline gap-2 px-3 pt-2">
              <MetaLabel>{KIND_LABEL[kind] ?? kind}</MetaLabel>
              <Mono className="text-faint">{entries.length}</Mono>
              {kind === "memory" ? (
                <span className="text-[11px] text-faint">
                  each traceable to the turn it was approved from
                </span>
              ) : null}
            </div>
            <ul className="px-1 py-1">
              {entries.map((entry) => (
                <Entry
                  entry={entry}
                  key={entry.entityRef}
                  onRemove={() => onToggleEntry(entry.entityRef, false)}
                />
              ))}
            </ul>
          </div>
        ))}
      </section>

      {pack.exclusions.length > 0 ? (
        <section className="flex flex-col gap-1 border-b border-border px-3 py-2">
          <MetaLabel>Not included</MetaLabel>
          <ul className="flex flex-col gap-1">
            {pack.exclusions.map((exclusion) => {
              const byPerson = removed.includes(exclusion.entityRef);
              return (
                <li
                  className="flex items-baseline gap-2"
                  key={`${exclusion.entityRef}:${exclusion.reason}`}
                >
                  <Mono className="min-w-0 flex-1 truncate text-faint">{exclusion.entityRef}</Mono>
                  <span className="shrink-0 text-[11px] text-muted-foreground">
                    {exclusion.reason}
                  </span>
                  {byPerson ? (
                    <button
                      type="button"
                      onClick={() => onToggleEntry(exclusion.entityRef, true)}
                      className="flex shrink-0 items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
                    >
                      <PlusIcon className="size-3" />
                      Add back
                    </button>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}

      <section className="flex flex-col gap-2 border-b border-border px-3 py-2">
        <MetaLabel>What it may do</MetaLabel>

        <div className="flex flex-col gap-1">
          <span className="text-[11px] text-faint">Tools</span>
          <ul className="flex flex-wrap gap-1">
            {pack.tools.map((tool) => (
              <li
                key={tool.name}
                title={tool.description}
                className={cn(
                  "flex items-center gap-1 border px-1.5 py-0.5",
                  tool.readOnly ? "border-border" : "border-destructive/50",
                )}
              >
                <Mono className={tool.readOnly ? "text-muted-foreground" : "text-destructive"}>
                  {tool.name}
                </Mono>
                {tool.readOnly ? null : <span className="text-[10px] text-destructive">mutating</span>}
              </li>
            ))}
          </ul>
          {mutatingTools.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              Every tool is read-only. This agent cannot change anything.
            </p>
          ) : null}
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-[11px] text-faint">Ceiling</span>
          <ul className="flex flex-wrap gap-x-3 gap-y-1">
            {ceilingRows(pack.permissions.ceiling).map((row) => (
              <li key={row.label} className="flex items-baseline gap-1.5">
                <span className="text-[11px] text-muted-foreground">{row.label}</span>
                <Mono className={row.danger ? "text-destructive" : "text-faint"}>{row.value}</Mono>
                {row.danger ? (
                  <AlertTriangleIcon className="size-3 shrink-0 text-destructive" />
                ) : null}
              </li>
            ))}
          </ul>
          <p className="text-[11px] text-muted-foreground">
            The ceiling comes from the role. No decision made during the run can widen it.
          </p>
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-[11px] text-faint">Declared scope</span>
          {pack.permissions.declaredScope.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              None declared in the approved plan — every write will stop and ask.
            </p>
          ) : (
            <ul className="flex flex-wrap gap-1">
              {pack.permissions.declaredScope.map((glob) => (
                <li key={glob} className="border border-border px-1.5 py-0.5">
                  <Mono className="text-muted-foreground">{glob}</Mono>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="flex flex-col gap-1">
          <span className="text-[11px] text-faint">Standing grants</span>
          {pack.permissions.standingGrants.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              None. Every effect this agent attempts will stop and ask.
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {pack.permissions.standingGrants.map((grant) => (
                <li
                  key={`${grant.capability}:${grant.pattern ?? ""}`}
                  className="flex flex-wrap items-baseline gap-2"
                >
                  <Mono className="text-foreground">{grant.capability}</Mono>
                  {grant.pattern ? <Mono className="text-faint">{grant.pattern}</Mono> : null}
                  <span className="text-[11px] text-muted-foreground">
                    {grant.decision} · granted by {grant.grantedBy}
                    {grant.expiresAt ? ` · expires ${grant.expiresAt}` : ""}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {pack.permissions.everythingElseAsks ? (
            <p className="text-[11px] text-muted-foreground">
              Everything not listed above stops the agent and asks you, in this conversation.
            </p>
          ) : null}
        </div>
      </section>

      <section className="flex flex-col border-b border-border">
        <button
          type="button"
          aria-expanded={showPrompt}
          onClick={() => setShowPrompt((current) => !current)}
          className="flex items-center gap-1.5 px-3 py-2 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised"
        >
          {showPrompt ? (
            <ChevronDownIcon className="size-3.5 text-faint" />
          ) : (
            <ChevronRightIcon className="size-3.5 text-faint" />
          )}
          <MetaLabel>The prompt, verbatim</MetaLabel>
          <Mono className="ml-auto text-faint">{bytes(pack.prompt.length)}</Mono>
        </button>
        {showPrompt ? (
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap border-t border-border px-3 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
            {pack.prompt}
          </pre>
        ) : null}
      </section>

      {error ? (
        <p className="mx-3 mt-2 border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <footer className="flex flex-wrap items-center gap-2 px-3 py-2">
        <span className="min-w-0 flex-1 text-[11px] text-faint">
          The digest is over the prompt above. The run records it, and launching revalidates it — if
          this changes before it starts, you are asked again.
        </span>
        <button
          type="button"
          disabled={busy}
          onClick={() => onStart(pack.digest)}
          className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <PlayIcon className="size-3.5" />
          {busy ? "Starting…" : "Start run"}
        </button>
      </footer>
    </article>
  );
}
