import type { CommandFailure, WorktreePlan } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { PlusIcon } from "lucide-react";
import { type FormEvent, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

interface CreateTaskDialogProps {
  onCreated: (task: WorktreePlan) => void;
  repositoryId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function CreateTaskDialog({ onCreated, repositoryId }: CreateTaskDialogProps) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [intent, setIntent] = useState("");
  const [baseRef, setBaseRef] = useState("HEAD");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!title.trim() || !intent.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const task = await invoke<WorktreePlan>("create_task", {
        repositoryId,
        title: title.trim(),
        intent: intent.trim(),
        baseRef: baseRef.trim() || "HEAD",
      });
      onCreated(task);
      setTitle("");
      setIntent("");
      setBaseRef("HEAD");
      setOpen(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger render={<Button size="sm" type="button" />}>
        <PlusIcon data-icon="inline-start" /> New task
      </DialogTrigger>
      <DialogContent>
        <form className="flex flex-col gap-4" onSubmit={(event) => void create(event)}>
          <DialogHeader>
            <DialogTitle>Create an isolated task</DialogTitle>
            <DialogDescription>
              Mars plans a branch and worktree first. No Git mutation happens until you approve the plan.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor="task-title">Title</Label>
            <Input id="task-title" onChange={(event) => setTitle(event.target.value)} value={title} />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="task-intent">Intent</Label>
            <Textarea id="task-intent" onChange={(event) => setIntent(event.target.value)} rows={4} value={intent} />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="task-base">Base ref</Label>
            <Input id="task-base" onChange={(event) => setBaseRef(event.target.value)} value={baseRef} />
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          <DialogFooter>
            <Button disabled={busy || !title.trim() || !intent.trim()} type="submit">
              {busy ? "Planning…" : "Plan task"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
