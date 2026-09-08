import type {
  ApprovalOutcome,
  CommandFailure,
  ConflictChoice,
  ConflictResolution,
  DetectedConflict,
  Memory,
  MemoryProposal,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangleIcon,
  CheckIcon,
  LoaderCircleIcon,
  QuoteIcon,
  RefreshCwIcon,
  XIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";

interface MemoryApprovalProps {
  projectId: string;
}

const CHOICE_LABELS: Record<ConflictChoice, string> = {
  keep_both: "Keep both",
  supersede_other: "Replace the old one",
  revoke_other: "Withdraw the old one",
  keep_existing: "Keep the old one instead",
};

export function MemoryApproval({ projectId }: MemoryApprovalProps) {
  const [pending, setPending] = useState<MemoryProposal[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [conflicts, setConflicts] = useState<Record<string, DetectedConflict[]>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      setPending(
        await invoke<MemoryProposal[]>("memory_pending", { filters: { projectId } }),
      );
      setMemories(await invoke<Memory[]>("memory_list", { filters: { projectId } }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }, [projectId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function approve(proposal: MemoryProposal, resolutions?: ConflictResolution[]) {
    setBusy(true);
    setError(null);
    try {
      const edited = edits[proposal.id];
      const outcome = await invoke<ApprovalOutcome>("memory_approve", {
        request: {
          proposalId: proposal.id,
          // Only send an edit when it is actually a change, so the audit trail
          // distinguishes "accepted" from "edited, then accepted". Who is
          // approving is filled in by the backend.
          editedText: edited && edited !== proposal.text ? edited : null,
          resolutions: resolutions ?? [],
        },
      });
      if (outcome.outcome === "conflictsRequireChoice") {
        // Nothing was written. The human decides before anything is kept.
        setConflicts((current) => ({ ...current, [proposal.id]: outcome.conflicts }));
        return;
      }
      setConflicts((current) => {
        const next = { ...current };
        delete next[proposal.id];
        return next;
      });
      await refresh();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function reject(proposal: MemoryProposal) {
    setBusy(true);
    setError(null);
    try {
      await invoke<MemoryProposal>("memory_reject", {
        request: {
          proposalId: proposal.id,
          reason: "rejected in review",
        },
      });
      await refresh();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  function resolve(proposal: MemoryProposal, other: string, choice: ConflictChoice) {
    const detected = conflicts[proposal.id] ?? [];
    // Every flagged pair needs an answer, so the ones not being changed here
    // are sent as "keep both" alongside the one that was chosen.
    const resolutions: ConflictResolution[] = detected.map((conflict) => ({
      otherMemoryId: conflict.other.id,
      choice: conflict.other.id === other ? choice : "keep_both",
    }));
    void approve(proposal, resolutions);
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-card" aria-label="Memory">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-medium">Memory</p>
          <p className="text-[10px] uppercase tracking-wider text-muted-foreground">
            {pending.length} awaiting approval · {memories.length} active
          </p>
        </div>
        <Button
          aria-label="Refresh"
          disabled={busy}
          onClick={() => void refresh()}
          size="icon-sm"
          type="button"
          variant="ghost"
        >
          {busy ? <LoaderCircleIcon className="animate-spin" /> : <RefreshCwIcon />}
        </Button>
      </header>

      {error ? <p className="p-3 text-xs text-destructive">{error}</p> : null}

      <ScrollArea className="min-h-0 flex-1">
        <div className="grid gap-4 p-3">
          {pending.length > 0 ? (
            <div className="grid gap-2" aria-label="Awaiting approval">
              <h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                Proposed · nothing here is in use
              </h3>
              {pending.map((proposal) => (
                <article className="grid gap-2 rounded-lg border bg-background p-2" key={proposal.id}>
                  <div className="flex flex-wrap items-center gap-1">
                    <Badge variant="outline">{proposal.kind}</Badge>
                    <Badge variant="outline">{proposal.scope}</Badge>
                    <span className="text-[10px] text-muted-foreground">
                      proposed by {proposal.proposedBy}
                    </span>
                  </div>
                  <Textarea
                    aria-label={`Memory text for ${proposal.id}`}
                    className="min-h-16 text-xs"
                    onChange={(event) =>
                      setEdits((current) => ({ ...current, [proposal.id]: event.target.value }))
                    }
                    value={edits[proposal.id] ?? proposal.text}
                  />
                  {/* The excerpt that motivated this. Without it there is
                      nothing to check the claim against. */}
                  <blockquote className="border-l-2 pl-2 text-[10px] text-muted-foreground">
                    <QuoteIcon className="mr-1 inline size-3" />
                    {proposal.origin.quotedText}
                    <span className="ml-1">
                      {proposal.origin.verified
                        ? "· checked against the transcript"
                        : "· not checked against a transcript"}
                    </span>
                  </blockquote>

                  {(conflicts[proposal.id] ?? []).length > 0 ? (
                    <div className="grid gap-2 rounded-md border border-amber-500/40 bg-amber-500/5 p-2" aria-label="Conflicts">
                      <p className="flex gap-1 text-[10px] text-amber-700 dark:text-amber-300">
                        <AlertTriangleIcon className="mt-0.5 size-3 shrink-0" />
                        This has to be decided before anything is remembered.
                      </p>
                      {(conflicts[proposal.id] ?? []).map((conflict) => (
                        <div className="grid gap-1" key={conflict.other.id}>
                          <p className="text-[10px] text-muted-foreground">
                            {conflict.explanation}
                          </p>
                          <p className="text-xs">{conflict.other.text}</p>
                          <div className="flex flex-wrap gap-1">
                            {(
                              [
                                "keep_both",
                                "supersede_other",
                                "revoke_other",
                                "keep_existing",
                              ] as ConflictChoice[]
                            ).map((choice) => (
                              <Button
                                disabled={busy}
                                key={choice}
                                onClick={() => resolve(proposal, conflict.other.id, choice)}
                                size="xs"
                                type="button"
                                variant="outline"
                              >
                                {CHOICE_LABELS[choice]}
                              </Button>
                            ))}
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="flex gap-2">
                      <Button
                        disabled={busy}
                        onClick={() => void approve(proposal)}
                        size="xs"
                        type="button"
                      >
                        <CheckIcon />
                        {edits[proposal.id] && edits[proposal.id] !== proposal.text
                          ? "Edit and accept"
                          : "Accept"}
                      </Button>
                      <Button
                        disabled={busy}
                        onClick={() => void reject(proposal)}
                        size="xs"
                        type="button"
                        variant="ghost"
                      >
                        <XIcon />
                        Reject
                      </Button>
                    </div>
                  )}
                </article>
              ))}
            </div>
          ) : null}

          <div className="grid gap-2" aria-label="Active memories">
            <h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              Remembered · approved by a person
            </h3>
            {memories.map((memory) => (
              <article className="grid gap-1 rounded-lg border bg-background p-2" key={memory.id}>
                <p className="text-xs">{memory.text}</p>
                <div className="flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
                  <Badge variant="secondary">{memory.kind}</Badge>
                  <Badge variant="outline">{memory.scope}</Badge>
                  <span>approved by {memory.approvedBy}</span>
                  <span>·</span>
                  {/* The usage trail is what turns a memory from an assertion
                      into something with a record of influence. */}
                  <span>
                    {memory.usedInTasks === 0
                      ? "not used yet"
                      : `used in ${memory.usedInTasks} task${memory.usedInTasks === 1 ? "" : "s"}`}
                  </span>
                </div>
              </article>
            ))}
            {memories.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                Nothing has been approved yet.
              </p>
            ) : null}
          </div>
        </div>
      </ScrollArea>
    </section>
  );
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}
