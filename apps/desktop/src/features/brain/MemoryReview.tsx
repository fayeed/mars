import type { AdapterKind, CommandFailure, KnowledgeRecord } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  BrainCircuitIcon,
  CheckIcon,
  LoaderCircleIcon,
  PinIcon,
  RefreshCwIcon,
  SaveIcon,
  Trash2Icon,
} from "lucide-react";
import { useState } from "react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";

interface MemoryReviewProps {
  adapter: AdapterKind;
  taskId: string;
}

export function MemoryReview({ adapter, taskId }: MemoryReviewProps) {
  const [open, setOpen] = useState(false);
  const [records, setRecords] = useState<KnowledgeRecord[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<KnowledgeRecord | null>(null);

  async function openReview() {
    setOpen(true);
    await refresh();
  }

  async function refresh() {
    setBusy(true);
    setError(null);
    try {
      setRecords(await invoke<KnowledgeRecord[]>("list_task_memory", { taskId }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function extract() {
    setBusy(true);
    setError(null);
    try {
      await invoke<KnowledgeRecord[]>("extract_task_memory", {
        taskId,
        adapter,
        modelPermission: true,
      });
      await refresh();
    } catch (caught) {
      setError(messageFor(caught));
      setBusy(false);
    }
  }

  async function resolve(
    record: KnowledgeRecord,
    action: "approve" | "reject" | "correct" | "mark_stale" | "pin" | "unpin",
  ) {
    setBusy(true);
    setError(null);
    try {
      await invoke<KnowledgeRecord>("resolve_memory", {
        request: {
          taskId,
          recordId: record.id,
          action,
          title: action === "correct" ? record.title : null,
          body: action === "correct" ? record.body : null,
        },
      });
      setEditing(null);
      await refresh();
    } catch (caught) {
      setError(messageFor(caught));
      setBusy(false);
    }
  }

  return (
    <>
      <Button className="justify-start" onClick={() => void openReview()} type="button" variant="outline">
        <BrainCircuitIcon /> Review project memory
      </Button>
      <Dialog onOpenChange={setOpen} open={open}>
        <DialogContent className="max-h-[88vh] sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Review post-task memory</DialogTitle>
            <DialogDescription>
              Extraction sends visible session and diff evidence to {adapter}. Nothing becomes trusted project memory until you approve it.
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-wrap gap-2">
            <Button disabled={busy} onClick={() => void extract()} size="sm" type="button">
              {busy ? <LoaderCircleIcon className="animate-spin" /> : <BrainCircuitIcon />}
              Extract with {adapter}
            </Button>
            <Button disabled={busy} onClick={() => void refresh()} size="sm" type="button" variant="outline">
              <RefreshCwIcon /> Refresh
            </Button>
          </div>
          {error ? <Alert variant="destructive"><AlertDescription>{error}</AlertDescription></Alert> : null}
          <ScrollArea className="max-h-[60vh] pr-3">
            <div className="grid gap-3">
              {!busy && records.length === 0 ? (
                <p className="rounded-lg border border-dashed p-6 text-center text-xs text-muted-foreground">
                  No candidates yet. Extraction requires a completed agent session and a visible model run.
                </p>
              ) : null}
              {records.map((record) => (
                <article className="grid gap-2 rounded-lg border p-3" key={record.id}>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge>{record.kind}</Badge>
                    <Badge variant="outline">{record.status}</Badge>
                    {record.pinned ? <Badge variant="secondary"><PinIcon /> pinned</Badge> : null}
                    <span className="ml-auto text-[10px] text-muted-foreground">
                      {record.provenanceKind} · {Math.round(record.confidence * 100)}%
                    </span>
                  </div>
                  {editing?.id === record.id ? (
                    <>
                      <Input
                        aria-label="Corrected title"
                        onChange={(event) => setEditing({ ...editing, title: event.target.value })}
                        value={editing.title}
                      />
                      <Textarea
                        aria-label="Corrected rationale"
                        onChange={(event) => setEditing({ ...editing, body: event.target.value })}
                        value={editing.body}
                      />
                    </>
                  ) : (
                    <>
                      <h3 className="text-sm font-medium">{record.title}</h3>
                      <p className="text-xs leading-relaxed text-muted-foreground">{record.body}</p>
                    </>
                  )}
                  <div className="flex flex-wrap gap-2">
                    {record.status === "candidate" ? (
                      <>
                        <Button disabled={busy} onClick={() => void resolve(record, "approve")} size="xs" type="button">
                          <CheckIcon /> Approve
                        </Button>
                        <Button disabled={busy} onClick={() => void resolve(record, "reject")} size="xs" type="button" variant="outline">
                          <Trash2Icon /> Reject
                        </Button>
                      </>
                    ) : null}
                    {record.status === "approved" && editing?.id !== record.id ? (
                      <Button disabled={busy} onClick={() => setEditing(record)} size="xs" type="button" variant="outline">
                        Edit / supersede
                      </Button>
                    ) : null}
                    {editing?.id === record.id ? (
                      <Button disabled={busy} onClick={() => void resolve(editing, "correct")} size="xs" type="button">
                        <SaveIcon /> Save superseding correction
                      </Button>
                    ) : null}
                    {record.status === "approved" ? (
                      <Button disabled={busy} onClick={() => void resolve(record, "mark_stale")} size="xs" type="button" variant="outline">
                        Mark stale
                      </Button>
                    ) : null}
                    {record.status === "approved" || record.status === "stale" ? (
                      <Button disabled={busy} onClick={() => void resolve(record, record.pinned ? "unpin" : "pin")} size="xs" type="button" variant="ghost">
                        <PinIcon /> {record.pinned ? "Unpin" : "Pin"}
                      </Button>
                    ) : null}
                  </div>
                </article>
              ))}
            </div>
          </ScrollArea>
        </DialogContent>
      </Dialog>
    </>
  );
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}
