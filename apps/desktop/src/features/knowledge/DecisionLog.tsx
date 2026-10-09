import type { KnowledgeAction, KnowledgeItem, KnowledgeKind } from "@mars/shared-types";
import { CheckIcon, PencilIcon, PinIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock } from "@/shell/primitives";

export interface DecisionLogProps {
  items: KnowledgeItem[];
  state: ActivityState;
  error?: string | null;
  /** Whether rejected, stale and superseded records are listed too. */
  includeResolved: boolean;
  onIncludeResolved: (value: boolean) => void;
  busyId?: string | null;
  onResolve: (
    item: KnowledgeItem,
    action: KnowledgeAction,
    edit?: { title: string; body: string },
  ) => void;
  onOpenTask?: (taskId: string) => void;
  onOpenFile?: (path: string) => void;
}

type Show = "decisions" | "choices" | "everything";

/**
 * The kinds a person settles: choices an agent made or a limit, risk or open
 * question it found. Summaries, tests and commits record what happened and
 * are listed under "everything".
 */
export const REVIEWED_KINDS: readonly KnowledgeKind[] = ["decision", "constraint", "risk", "question"];

const SHOWN: Record<Show, readonly KnowledgeKind[] | null> = {
  decisions: ["decision"],
  choices: REVIEWED_KINDS,
  everything: null,
};

const ALTERNATIVES_HEADING = "\n\nConsidered and not chosen:";

/** The body without the alternatives, which are listed on their own. */
function bodyText(item: KnowledgeItem) {
  const at = item.record.body.indexOf(ALTERNATIVES_HEADING);
  return item.alternatives.length > 0 && at >= 0 ? item.record.body.slice(0, at) : item.record.body;
}

function whenText(timestamp: string) {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? timestamp : date.toLocaleString();
}

function source(item: KnowledgeItem) {
  if (item.via === "person") return "written by you";
  const agent = item.agent ?? "an agent";
  return item.via === "agent" ? `${agent}, while working` : `read from ${agent}'s session`;
}

const buttonClass =
  "flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint";
const primaryClass =
  "flex items-center gap-1 border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:bg-transparent disabled:text-faint";

function DecisionRow({
  busy,
  item,
  onOpenFile,
  onOpenTask,
  onResolve,
}: {
  busy: boolean;
  item: KnowledgeItem;
  onOpenFile?: (path: string) => void;
  onOpenTask?: (taskId: string) => void;
  onResolve: DecisionLogProps["onResolve"];
}) {
  const { record } = item;
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(record.title);
  const [body, setBody] = useState(record.body);
  const waiting = record.status === "candidate";
  const changed = title.trim() !== record.title || body.trim() !== record.body;

  return (
    <li
      className="flex flex-col gap-1.5 border-b border-border px-3 py-2"
      data-status={record.status}
      aria-label={record.title}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className="text-faint">{record.kind}</Mono>
        {record.status === "approved" ? null : (
          <Mono className={waiting ? "text-primary" : "text-faint"}>
            {waiting ? "needs your review" : record.status}
          </Mono>
        )}
        {record.pinned ? (
          <Mono className="flex items-center gap-0.5 text-faint">
            <PinIcon className="size-3" /> pinned
          </Mono>
        ) : null}
        <Mono className="ml-auto text-faint">{Math.round(record.confidence * 100)}% sure</Mono>
      </div>

      {editing ? (
        <div className="flex flex-col gap-1">
          <input
            aria-label="Title"
            className="border border-border bg-transparent px-1.5 py-0.5 text-[12px] text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => setTitle(event.target.value)}
            value={title}
          />
          <textarea
            aria-label="What was decided, and why"
            className="min-h-24 resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] leading-relaxed text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => setBody(event.target.value)}
            value={body}
          />
          <p className="text-[11px] text-faint">
            {waiting
              ? "Approving it reworded keeps the agent's version in the history, superseded by yours."
              : "A correction supersedes this record rather than overwriting it."}
          </p>
          <div className="flex items-center gap-1">
            <button
              type="button"
              className={primaryClass}
              disabled={busy || !changed || !title.trim() || !body.trim()}
              onClick={() => {
                onResolve(item, "correct", { title: title.trim(), body: body.trim() });
                setEditing(false);
              }}
            >
              <CheckIcon className="size-3" />
              {waiting ? "Approve as reworded" : "Save correction"}
            </button>
            <button
              type="button"
              className={buttonClass}
              onClick={() => {
                setTitle(record.title);
                setBody(record.body);
                setEditing(false);
              }}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          <h3 className="text-[13px] font-medium text-foreground">{record.title}</h3>
          <p className="whitespace-pre-wrap text-[12px] leading-relaxed text-foreground">
            {bodyText(item)}
          </p>
          {item.alternatives.length > 0 ? (
            <div className="flex flex-col gap-0.5">
              <MetaLabel>Considered and not chosen</MetaLabel>
              <ul className="flex flex-col gap-0.5 pl-3">
                {item.alternatives.map((alternative) => (
                  <li key={alternative} className="list-disc text-[12px] text-muted-foreground">
                    {alternative}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </>
      )}

      {item.files.length > 0 ? (
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <MetaLabel>Affects</MetaLabel>
          {item.files.map((path) => (
            <button
              key={path}
              type="button"
              className="outline-none hover:underline focus-visible:underline"
              onClick={() => onOpenFile?.(path)}
            >
              <Mono className="text-foreground">{path}</Mono>
            </button>
          ))}
        </div>
      ) : null}

      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className="text-faint">{source(item)}</Mono>
        {item.taskId ? (
          <button
            type="button"
            className="min-w-0 truncate text-left outline-none hover:underline focus-visible:underline"
            onClick={() => item.taskId && onOpenTask?.(item.taskId)}
          >
            <Mono className="text-muted-foreground">in {item.taskTitle ?? "a task"}</Mono>
          </button>
        ) : null}
        <Mono className="ml-auto text-faint">{whenText(record.createdAt)}</Mono>
      </div>

      {editing ? null : (
        <div className="flex flex-wrap items-center gap-1">
          {waiting ? (
            <>
              <button
                type="button"
                className={primaryClass}
                disabled={busy}
                onClick={() => onResolve(item, "approve")}
              >
                <CheckIcon className="size-3" />
                Approve
              </button>
              <button
                type="button"
                className={buttonClass}
                disabled={busy}
                onClick={() => setEditing(true)}
              >
                <PencilIcon className="size-3" />
                Reword…
              </button>
              <button
                type="button"
                className={cn(buttonClass, "text-muted-foreground")}
                disabled={busy}
                onClick={() => onResolve(item, "reject")}
              >
                <XIcon className="size-3" />
                Reject
              </button>
            </>
          ) : null}
          {record.status === "approved" ? (
            <>
              <button
                type="button"
                className={buttonClass}
                disabled={busy}
                onClick={() => setEditing(true)}
              >
                <PencilIcon className="size-3" />
                Correct…
              </button>
              <button
                type="button"
                className={cn(buttonClass, "text-muted-foreground")}
                disabled={busy}
                onClick={() => onResolve(item, "mark_stale")}
              >
                No longer true
              </button>
            </>
          ) : null}
          {record.status === "approved" || record.status === "stale" ? (
            <button
              type="button"
              className={cn(buttonClass, "text-muted-foreground")}
              disabled={busy}
              onClick={() => onResolve(item, record.pinned ? "unpin" : "pin")}
            >
              <PinIcon className="size-3" />
              {record.pinned ? "Unpin" : "Pin"}
            </button>
          ) : null}
        </div>
      )}
    </li>
  );
}

/**
 * What agents decided, across every task, and what you said about it.
 *
 * An agent records a decision when it makes one (and extraction finds more
 * in a finished session). None of it is fact until a person approves it, so
 * what is waiting is at the top; approved decisions are what agents are later
 * told, and a correction supersedes rather than overwrites. Each record says
 * which agent made it, in which task, what else it weighed, and what it
 * touches — the things that make it checkable.
 */
export function DecisionLog({
  busyId,
  error,
  includeResolved,
  items,
  onIncludeResolved,
  onOpenFile,
  onOpenTask,
  onResolve,
  state,
}: DecisionLogProps) {
  const [show, setShow] = useState<Show>("choices");
  const [contains, setContains] = useState("");

  const shown = useMemo(() => {
    const kinds = SHOWN[show];
    const words = contains.toLowerCase().split(/\s+/).filter(Boolean);
    return items.filter(
      (item) =>
        (kinds === null || kinds.includes(item.record.kind)) &&
        words.every(
          (word) =>
            item.record.title.toLowerCase().includes(word) ||
            item.record.body.toLowerCase().includes(word) ||
            item.files.some((path) => path.toLowerCase().includes(word)),
        ),
    );
  }, [contains, items, show]);
  const waiting = shown.filter((item) => item.record.status === "candidate");
  const settled = shown.filter((item) => item.record.status !== "candidate");

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="decision-log">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-1.5">
        <select
          aria-label="Show"
          className="border border-border bg-transparent px-1 text-[11px]"
          value={show}
          onChange={(event) => setShow(event.target.value as Show)}
        >
          <option value="choices">Decisions, constraints, risks, questions</option>
          <option value="decisions">Decisions only</option>
          <option value="everything">Everything recorded</option>
        </select>
        <input
          aria-label="Filter decisions"
          className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none placeholder:text-faint focus-visible:border-border-strong"
          onChange={(event) => setContains(event.target.value)}
          placeholder="Filter by words or a file path"
          value={contains}
        />
        <label className="flex items-center gap-1 text-[11px] text-muted-foreground">
          <input
            type="checkbox"
            checked={includeResolved}
            onChange={(event) => onIncludeResolved(event.target.checked)}
          />
          Include rejected and superseded
        </label>
      </div>
      {error ? (
        <p role="alert" className="border-b border-border px-3 py-1 text-[11px] text-destructive">
          {error}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto">
        {state === "loading" && items.length === 0 ? (
          <StateBlock state="loading" title="Reading what agents decided" />
        ) : state === "failed" && items.length === 0 ? (
          <StateBlock state="failed" title="The decisions could not be read" />
        ) : shown.length === 0 ? (
          <StateBlock
            state="idle"
            title={items.length === 0 ? "No decisions recorded yet" : "Nothing matches"}
            description={
              items.length === 0
                ? "Agents record a decision when they make one — a library chosen, a trade-off taken. Each lands here for you to approve before any agent is told it as fact."
                : undefined
            }
          />
        ) : (
          <>
            <section aria-label="Waiting on you">
              <div className="flex items-baseline gap-2 border-b border-border px-3 py-1.5">
                <MetaLabel>Waiting on you</MetaLabel>
                <Mono className="text-faint">{waiting.length}</Mono>
              </div>
              {waiting.length === 0 ? (
                <p className="border-b border-border px-3 py-2 text-[11px] text-muted-foreground">
                  Nothing to review.
                </p>
              ) : (
                <ul>
                  {waiting.map((item) => (
                    <DecisionRow
                      busy={busyId === item.record.id}
                      item={item}
                      key={item.record.id}
                      onOpenFile={onOpenFile}
                      onOpenTask={onOpenTask}
                      onResolve={onResolve}
                    />
                  ))}
                </ul>
              )}
            </section>
            {settled.length > 0 ? (
              <section aria-label="Settled">
                <div className="flex items-baseline gap-2 border-b border-border px-3 py-1.5">
                  <MetaLabel>Settled</MetaLabel>
                  <Mono className="text-faint">{settled.length}</Mono>
                </div>
                <ul>
                  {settled.map((item) => (
                    <DecisionRow
                      busy={busyId === item.record.id}
                      item={item}
                      key={item.record.id}
                      onOpenFile={onOpenFile}
                      onOpenTask={onOpenTask}
                      onResolve={onResolve}
                    />
                  ))}
                </ul>
              </section>
            ) : null}
          </>
        )}
      </div>
    </div>
  );
}
