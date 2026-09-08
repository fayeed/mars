import type {
  Memory,
  MemoryKind,
  MemoryProposal,
  MemoryScope,
  MemoryStatus,
} from "@mars/shared-types";
import { ArrowUpRightIcon, CheckIcon, PencilIcon, XIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface MemoryBrowserProps {
  memories: Memory[];
  proposals: MemoryProposal[];
  state: ActivityState;
  error?: string | null;
  filters: MemoryFilterState;
  onFilters: (next: MemoryFilterState) => void;
  /** Opens the conversation turn a memory or proposal came from. */
  onJumpToOrigin: (conversationId: string, messageId: string) => void;
  onRevise: (memory: Memory, text: string) => void;
  onRevoke: (memory: Memory, reason: string) => void;
  onApprove: (proposal: MemoryProposal, editedText: string | null) => void;
  onReject: (proposal: MemoryProposal, reason: string) => void;
  busyId?: string | null;
}

export interface MemoryFilterState {
  scopes: MemoryScope[];
  kinds: MemoryKind[];
  statuses: MemoryStatus[];
  contains: string;
}

const SCOPES: MemoryScope[] = ["repo", "task", "global"];
const KINDS: MemoryKind[] = ["convention", "decision", "preference", "fact", "caution"];
const STATUSES: MemoryStatus[] = ["active", "superseded", "revoked"];

function Toggle<T extends string>({
  onToggle,
  selected,
  values,
}: {
  onToggle: (value: T) => void;
  selected: T[];
  values: T[];
}) {
  return (
    <div className="flex flex-wrap gap-1">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          aria-pressed={selected.includes(value)}
          onClick={() => onToggle(value)}
          className={cn(
            "border px-1.5 py-0.5 text-[11px] outline-none",
            selected.includes(value)
              ? "border-primary text-foreground"
              : "border-border text-muted-foreground hover:border-border-strong",
          )}
        >
          {value}
        </button>
      ))}
    </div>
  );
}

/** Where a memory came from, and whether that trace was ever checked. */
function Origin({
  onJump,
  origin,
}: {
  onJump: () => void;
  origin: Memory["origin"];
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <blockquote className="border-l-2 border-border pl-2 text-[11px] leading-relaxed text-muted-foreground">
        {origin.quotedText}
      </blockquote>
      <div className="flex items-baseline gap-2">
        <button
          type="button"
          onClick={onJump}
          className="group flex items-baseline gap-1 outline-none"
        >
          <ArrowUpRightIcon className="size-3 shrink-0 self-center text-faint opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
          <Mono className="text-faint group-hover:text-foreground group-focus-visible:text-foreground">
            {origin.conversationId}:{origin.messageId}
          </Mono>
        </button>
        {origin.verified ? null : (
          // The model records whether the excerpt was checked against the turn.
          // Unconfirmed is said, not implied.
          <Mono className="text-primary">excerpt not verified against that turn</Mono>
        )}
      </div>
    </div>
  );
}

function MemoryRow({
  busy,
  memory,
  onJumpToOrigin,
  onRevise,
  onRevoke,
}: {
  busy: boolean;
  memory: Memory;
  onJumpToOrigin: (conversationId: string, messageId: string) => void;
  onRevise: (memory: Memory, text: string) => void;
  onRevoke: (memory: Memory, reason: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(memory.text);
  const [revoking, setRevoking] = useState(false);
  const [reason, setReason] = useState("");

  return (
    <li className="flex flex-col gap-1.5 border-b border-border px-3 py-2">
      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className="shrink-0 text-faint">{memory.kind}</Mono>
        <Mono className="shrink-0 text-faint">{memory.scope}</Mono>
        {memory.status === "active" ? null : (
          <Mono className="shrink-0 text-primary">{memory.status}</Mono>
        )}
        <span className="ml-auto flex shrink-0 items-baseline gap-2">
          <Mono className="text-faint">used {memory.useCount}×</Mono>
          <Mono className="text-faint">in {memory.usedInTasks} task(s)</Mono>
        </span>
      </div>

      {editing ? (
        <div className="flex flex-col gap-1">
          <textarea
            aria-label={`Revise memory ${memory.id}`}
            className="min-h-16 w-full resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] leading-relaxed text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => setDraft(event.target.value)}
            value={draft}
          />
          <p className="text-[11px] text-faint">
            Revising supersedes this memory rather than overwriting it. The old text stays in the
            history.
          </p>
          <div className="flex items-center gap-1">
            <button
              type="button"
              disabled={busy || draft.trim() === memory.text}
              onClick={() => {
                onRevise(memory, draft.trim());
                setEditing(false);
              }}
              className="border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
            >
              Supersede
            </button>
            <button
              type="button"
              onClick={() => {
                setDraft(memory.text);
                setEditing(false);
              }}
              className="border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <p className="text-[13px] leading-relaxed text-foreground">{memory.text}</p>
      )}

      <Origin
        onJump={() => onJumpToOrigin(memory.origin.conversationId, memory.origin.messageId)}
        origin={memory.origin}
      />

      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className="text-faint">
          approved by {memory.approvedBy} · {memory.approvedAt}
        </Mono>
      </div>

      {revoking ? (
        <div className="flex items-center gap-1">
          <input
            aria-label={`Why revoke memory ${memory.id}`}
            autoFocus
            className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
            onChange={(event) => setReason(event.target.value)}
            placeholder="Why is this no longer true?"
            value={reason}
          />
          <button
            type="button"
            disabled={busy || reason.trim().length === 0}
            onClick={() => {
              onRevoke(memory, reason.trim());
              setRevoking(false);
              setReason("");
            }}
            className="border border-border px-2 py-0.5 text-[11px] text-destructive outline-none hover:border-destructive disabled:cursor-not-allowed disabled:text-faint"
          >
            Revoke
          </button>
        </div>
      ) : memory.status === "active" ? (
        <div className="flex items-center gap-1">
          <button
            type="button"
            disabled={busy}
            onClick={() => setEditing(true)}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <PencilIcon className="size-3" />
            Edit
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => setRevoking(true)}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <XIcon className="size-3" />
            Revoke
          </button>
        </div>
      ) : null}
    </li>
  );
}

function ProposalRow({
  busy,
  onApprove,
  onJumpToOrigin,
  onReject,
  proposal,
}: {
  busy: boolean;
  onApprove: (proposal: MemoryProposal, editedText: string | null) => void;
  onJumpToOrigin: (conversationId: string, messageId: string) => void;
  onReject: (proposal: MemoryProposal, reason: string) => void;
  proposal: MemoryProposal;
}) {
  const [draft, setDraft] = useState(proposal.text);
  const [reason, setReason] = useState("");

  return (
    <li className="flex flex-col gap-1.5 border-b border-border px-3 py-2">
      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className="text-faint">{proposal.kind}</Mono>
        <Mono className="text-faint">{proposal.scope}</Mono>
        <Mono className="ml-auto text-faint">proposed by {proposal.proposedBy}</Mono>
      </div>

      <textarea
        aria-label={`Proposed memory ${proposal.id}`}
        className="min-h-14 w-full resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] leading-relaxed text-foreground outline-none focus-visible:border-border-strong"
        onChange={(event) => setDraft(event.target.value)}
        value={draft}
      />

      <Origin
        onJump={() => onJumpToOrigin(proposal.origin.conversationId, proposal.origin.messageId)}
        origin={proposal.origin}
      />

      <div className="flex flex-wrap items-center gap-1">
        <button
          type="button"
          disabled={busy}
          onClick={() => onApprove(proposal, draft.trim() === proposal.text ? null : draft.trim())}
          className="flex items-center gap-1 border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <CheckIcon className="size-3" />
          {draft.trim() === proposal.text ? "Approve" : "Edit and approve"}
        </button>
        <input
          aria-label={`Why reject proposal ${proposal.id}`}
          className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 text-[11px] text-foreground outline-none focus-visible:border-border-strong"
          onChange={(event) => setReason(event.target.value)}
          placeholder="Reason, if rejecting"
          value={reason}
        />
        <button
          type="button"
          disabled={busy || reason.trim().length === 0}
          onClick={() => onReject(proposal, reason.trim())}
          className="border border-border px-2 py-0.5 text-[11px] text-muted-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          Reject
        </button>
      </div>
    </li>
  );
}

/**
 * Everything Mars has been told to remember, and everything it has asked to.
 *
 * A proposal is not a memory and is never shown as one: the queue is separate,
 * and nothing in it has reached an agent's context. For memories themselves the
 * two facts that matter most are on every row — where the text came from, with
 * a way back to the turn that said it, and how often it has actually been used,
 * which is the only evidence that keeping it was worth anything.
 */
export function MemoryBrowser({
  busyId,
  error,
  filters,
  memories,
  onApprove,
  onFilters,
  onJumpToOrigin,
  onReject,
  onRevise,
  onRevoke,
  proposals,
  state,
}: MemoryBrowserProps) {
  const pending = proposals.filter((proposal) => proposal.status === "pending");

  function toggle<T extends string>(list: T[], value: T): T[] {
    return list.includes(value) ? list.filter((one) => one !== value) : [...list, value];
  }

  return (
    <section aria-label="Memories" className="flex h-full min-h-0 flex-col">
      <div className="flex flex-col gap-2 border-b border-border px-3 py-2">
        <input
          aria-label="Filter memories by text"
          className="w-full border border-border bg-transparent px-1.5 py-1 text-[12px] text-foreground outline-none focus-visible:border-border-strong"
          onChange={(event) => onFilters({ ...filters, contains: event.target.value })}
          placeholder="Contains…"
          value={filters.contains}
        />
        <div className="flex flex-col gap-1">
          <MetaLabel>Scope</MetaLabel>
          <Toggle
            onToggle={(value) => onFilters({ ...filters, scopes: toggle(filters.scopes, value) })}
            selected={filters.scopes}
            values={SCOPES}
          />
        </div>
        <div className="flex flex-col gap-1">
          <MetaLabel>Kind</MetaLabel>
          <Toggle
            onToggle={(value) => onFilters({ ...filters, kinds: toggle(filters.kinds, value) })}
            selected={filters.kinds}
            values={KINDS}
          />
        </div>
        <div className="flex flex-col gap-1">
          <MetaLabel>Status</MetaLabel>
          <Toggle
            onToggle={(value) =>
              onFilters({ ...filters, statuses: toggle(filters.statuses, value) })
            }
            selected={filters.statuses}
            values={STATUSES}
          />
          <span className="text-[11px] text-faint">No status selected means active only.</span>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {pending.length > 0 ? (
          <>
            <div className="flex items-baseline gap-2 border-b border-border px-3 pt-2 pb-1">
              <MetaLabel>Awaiting your approval</MetaLabel>
              <Mono className="text-faint">{pending.length}</Mono>
              <span className="text-[11px] text-faint">none of this is in effect</span>
            </div>
            <ul>
              {pending.map((proposal) => (
                <ProposalRow
                  busy={busyId === proposal.id}
                  key={proposal.id}
                  onApprove={onApprove}
                  onJumpToOrigin={onJumpToOrigin}
                  onReject={onReject}
                  proposal={proposal}
                />
              ))}
            </ul>
          </>
        ) : null}

        {error ? (
          <p className="mx-3 my-2 border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
            <Mono>{error}</Mono>
          </p>
        ) : null}

        {state === "loading" ? (
          <StateBlock state="loading" title="Reading memories" />
        ) : memories.length === 0 ? (
          <StateBlock
            state="idle"
            title="No memories match"
            description="Memory is only ever created by you approving a proposal, so an empty list means nothing has been approved that matches these filters."
          />
        ) : (
          <>
            <div className="flex items-baseline gap-2 border-b border-border px-3 pt-2 pb-1">
              <MetaLabel>Memories</MetaLabel>
              <Mono className="text-faint">{memories.length}</Mono>
            </div>
            <ul>
              {memories.map((memory) => (
                <MemoryRow
                  busy={busyId === memory.id}
                  key={memory.id}
                  memory={memory}
                  onJumpToOrigin={onJumpToOrigin}
                  onRevise={onRevise}
                  onRevoke={onRevoke}
                />
              ))}
            </ul>
          </>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-border px-3 py-1">
        <StateLabel state={state === "failed" ? "failed" : "idle"} />
        <span className="text-[11px] text-faint">
          Every memory here was approved by a person and can be revoked by one.
        </span>
      </div>
    </section>
  );
}
