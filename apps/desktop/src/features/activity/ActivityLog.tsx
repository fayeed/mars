import type { ActivityEvent, ActivityKind, ActivityPayload } from "@mars/shared-types";
import { ChevronDownIcon, ChevronRightIcon } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";
import { FailureEvidenceView } from "@/features/activity/FailureEvidence";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateDot, StateLabel } from "@/shell/primitives";
import { useVirtualRows } from "@/shell/useVirtualRows";

/** Reports a row's real height so the spacers above and below stay honest. */
type MeasureRow = (index: number, element: HTMLElement | null) => void;

export interface ActivityLogProps {
  events: ActivityEvent[];
  /** Resolves a spilled payload. Called once per row, when it is expanded. */
  loadPayload?: (digest: string) => Promise<ActivityPayload>;
  onOpenFile?: (path: string, line: number | null) => void;
  /** Rows for these kinds start open. Failures do, so evidence is not buried. */
  expandedKinds?: ActivityKind[];
  /**
   * Replace the default row for an event. Returning `null` keeps the default.
   *
   * This is how a screen puts its own card — a plan, a permission request, a
   * summary — at the point in the record where it belongs, rather than in a
   * pile underneath it.
   */
  renderEvent?: (event: ActivityEvent) => ReactNode | null;
  /**
   * The next step for a failed row. One per failure — the log offers the
   * obvious move, not a menu of strategies.
   */
  failureAction?: (event: ActivityEvent) => { label: string; onAct: () => void } | null;
  /**
   * The element the log scrolls inside.
   *
   * Given rather than owned: the transcript scrolls in the conversation column,
   * which is also where the scroll anchor compensates for layout changes. A
   * scroller of its own would put two on screen. Without it the log renders
   * every row, which is correct and simply slower.
   */
  scroller?: HTMLElement | null;
}

/** A duration a person can read, at the precision the number deserves. */
export function formatDuration(ms: number | null): string | null {
  if (ms === null) return null;
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  return `${minutes}m ${Math.round((ms % 60_000) / 1000)}s`;
}

function field(payload: unknown, name: string): string | null {
  if (typeof payload !== "object" || payload === null) return null;
  const value = (payload as Record<string, unknown>)[name];
  return typeof value === "string" ? value : null;
}

function numberField(payload: unknown, name: string): number | null {
  if (typeof payload !== "object" || payload === null) return null;
  const value = (payload as Record<string, unknown>)[name];
  return typeof value === "number" ? value : null;
}

/**
 * How an event reads in the shell's state vocabulary.
 *
 * A command that has not reported a status is `streaming`, not `idle`: it is
 * something the agent started and has not finished, and the row should say so
 * while it is still true.
 */
export function eventActivityState(event: ActivityEvent): ActivityState {
  const status = field(event.payload, "status");
  switch (event.kind) {
    case "error":
      return "failed";
    case "permission_request":
      return "blocked";
    case "done":
      return event.payload && (event.payload as { success?: boolean }).success === false
        ? "failed"
        : "done";
    case "command":
    case "test_run":
      if (status === "failed") return "failed";
      if (status === "completed" || status === "passed") return "done";
      return status === null ? "done" : "streaming";
    case "file_edit":
      return "done";
    default:
      return "idle";
  }
}

/** The right-hand result column: what happened, in as few characters as it takes. */
function resultOf(event: ActivityEvent): string | null {
  const status = field(event.payload, "status");
  const exit = numberField(event.payload, "exitCode");
  if (event.kind === "command") {
    if (exit !== null) return `exit ${exit}`;
    return status;
  }
  if (event.kind === "test_run") return status;
  if (event.kind === "file_edit") {
    const added = numberField(event.payload, "added");
    const removed = numberField(event.payload, "removed");
    if (added !== null || removed !== null) return `+${added ?? 0} −${removed ?? 0}`;
    return field(event.payload, "change");
  }
  if (event.kind === "permission_decision") return field(event.payload, "decision");
  return null;
}

/** The mono line itself: the command, the path, the thing that was done. */
function subjectOf(event: ActivityEvent): string {
  return (
    field(event.payload, "command") ??
    field(event.payload, "path") ??
    field(event.payload, "name") ??
    event.summary
  );
}

const KIND_LABEL: Partial<Record<ActivityKind, string>> = {
  command: "Command",
  file_edit: "File",
  test_run: "Test",
  tool_call: "Tool",
  thought: "Thought",
  permission_request: "Permission",
  permission_decision: "Permission",
  error: "Error",
  done: "Run",
  run_started: "Run",
  recovery_noted: "Recovery",
  worktree_planned: "Worktree",
  worktree_created: "Worktree",
  worktree_removed: "Worktree",
  worktree_restored: "Worktree",
};

/**
 * One machine-authored line: collapsed to a single row, expandable to its
 * evidence, never hidden. The row carries what it did, how it ended and how
 * long it took, so the log can be read without opening anything.
 */
function MachineRow({
  event,
  failureAction,
  index,
  loadPayload,
  measure,
  onOpenFile,
  startOpen,
}: {
  event: ActivityEvent;
  failureAction?: (event: ActivityEvent) => { label: string; onAct: () => void } | null;
  index: number;
  loadPayload?: (digest: string) => Promise<ActivityPayload>;
  measure: MeasureRow;
  onOpenFile?: (path: string, line: number | null) => void;
  startOpen: boolean;
}) {
  const [open, setOpen] = useState(startOpen);
  const [payload, setPayload] = useState<ActivityPayload | null>(null);
  const [payloadState, setPayloadState] = useState<ActivityState>("idle");

  const state = eventActivityState(event);
  const duration = formatDuration(event.durationMs);
  const result = resultOf(event);
  const inlineText = field(event.payload, "output") ?? field(event.payload, "detail");
  // A row is worth expanding when it has anything behind it: a spilled payload,
  // inline output, or a payload object with more than the row already shows.
  const expandable = Boolean(event.payloadDigest || inlineText);

  const expand = useCallback(() => {
    const next = !open;
    setOpen(next);
    if (!next || payload || !event.payloadDigest || !loadPayload) return;
    setPayloadState("loading");
    loadPayload(event.payloadDigest)
      .then((resolved) => {
        setPayload(resolved);
        setPayloadState("idle");
      })
      .catch(() => setPayloadState("failed"));
  }, [event.payloadDigest, loadPayload, open, payload]);

  const body = payload?.text ?? inlineText ?? "";
  const failed = state === "failed";

  return (
    // Re-measured on every render, not only on mount: expanding a row to its
    // evidence changes its height, and every spacer below it would be wrong
    // until the next scroll.
    <li className="border-b border-border last:border-b-0" ref={(element) => measure(index, element)}>
      <button
        type="button"
        aria-expanded={expandable ? open : undefined}
        disabled={!expandable}
        onClick={expand}
        className={cn(
          "flex w-full items-baseline gap-1.5 px-3 py-1 text-left outline-none",
          expandable && "hover:bg-surface-raised focus-visible:bg-surface-raised",
          !expandable && "cursor-default",
        )}
      >
        <span className="flex w-3 shrink-0 justify-center self-center" aria-hidden="true">
          {expandable ? (
            open ? (
              <ChevronDownIcon className="size-3 text-faint" />
            ) : (
              <ChevronRightIcon className="size-3 text-faint" />
            )
          ) : null}
        </span>
        <StateDot className="self-center" state={state} />
        <span className="w-16 shrink-0 text-[10px] uppercase tracking-[0.12em] text-faint">
          {KIND_LABEL[event.kind] ?? event.kind}
        </span>
        <Mono
          className={cn("min-w-0 flex-1 truncate", failed ? "text-destructive" : "text-foreground")}
        >
          {subjectOf(event)}
        </Mono>
        {result ? (
          <Mono className={cn("shrink-0", failed ? "text-destructive" : "text-muted-foreground")}>
            {result}
          </Mono>
        ) : null}
        {duration ? <Mono className="w-12 shrink-0 text-right text-faint">{duration}</Mono> : null}
      </button>

      {open && expandable ? (
        <div className="border-t border-border bg-sunken px-3 py-2">
          {payloadState === "loading" ? (
            <StateLabel state="loading" detail="resolving output" />
          ) : payloadState === "failed" ? (
            <StateLabel state="failed" detail="the recorded output could not be read" />
          ) : failed ? (
            <FailureEvidenceView
              onOpenFile={onOpenFile}
              primaryAction={failureAction?.(event) ?? undefined}
              text={body}
            />
          ) : (
            <pre className="max-h-72 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
              {body}
            </pre>
          )}
        </div>
      ) : null}
    </li>
  );
}

/** A human or agent turn: prose, in the reading font, never collapsed. */
function MessageTurn({
  event,
  index,
  measure,
}: {
  event: ActivityEvent;
  index: number;
  measure: MeasureRow;
}) {
  return (
    <li
      className="flex flex-col gap-1 border-b border-border px-3 py-2.5"
      ref={(element) => measure(index, element)}
    >
      <MetaLabel>{event.actor === "human" ? "You" : event.actorLabel}</MetaLabel>
      <p className="text-[13px] leading-relaxed whitespace-pre-wrap text-foreground">
        {field(event.payload, "text") ?? event.summary}
      </p>
    </li>
  );
}

/**
 * The activity record, rendered.
 *
 * Human and agent prose sits in the reading font; everything a machine did sits
 * in mono with its result and duration on the same line. Nothing is dropped —
 * a kind this component has no special rendering for still gets a row.
 */
export function ActivityLog({
  events,
  expandedKinds = ["error"],
  failureAction,
  loadPayload,
  onOpenFile,
  renderEvent,
  scroller = null,
}: ActivityLogProps) {
  const { measure, window } = useVirtualRows({
    count: events.length,
    // A collapsed row is one line; the estimate is only ever used for rows that
    // have never been on screen, and is replaced by a measurement the moment
    // one is.
    estimatedRowHeight: 26,
    scroller,
  });

  const start = window?.start ?? 0;
  const visible = window ? events.slice(window.start, window.end) : events;

  return (
    <ul aria-label="Activity" className="flex flex-col">
      {/* Spacers stand in for the rows that are not rendered, so the scrollbar
          describes the whole record rather than the part of it on screen. */}
      {window && window.paddingTop > 0 ? (
        <li aria-hidden="true" style={{ height: window.paddingTop }} />
      ) : null}

      {visible.map((event, offset) => {
        const index = start + offset;
        const custom = renderEvent?.(event);
        if (custom) {
          return (
            <li
              className="border-b border-border last:border-b-0"
              key={event.id}
              ref={(element) => measure(index, element)}
            >
              {custom}
            </li>
          );
        }
        return event.kind === "message" ? (
          <MessageTurn event={event} key={event.id} measure={measure} index={index} />
        ) : (
          <MachineRow
            event={event}
            failureAction={failureAction}
            index={index}
            key={event.id}
            loadPayload={loadPayload}
            measure={measure}
            onOpenFile={onOpenFile}
            startOpen={expandedKinds.includes(event.kind) || eventActivityState(event) === "failed"}
          />
        );
      })}

      {window && window.paddingBottom > 0 ? (
        <li aria-hidden="true" style={{ height: window.paddingBottom }} />
      ) : null}
    </ul>
  );
}

/** A slot for screens that interleave their own cards into the record. */
export function ActivityGroup({ children }: { children: ReactNode }) {
  return <div className="border-b border-border">{children}</div>;
}
