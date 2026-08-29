import type { CommandFailure, ContextItem } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { ClipboardPlusIcon } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";

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

interface QuickCaptureProps {
  onCaptured: (item: ContextItem) => void;
  repositoryId: string;
  taskId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function QuickCapture({ onCaptured, repositoryId, taskId }: QuickCaptureProps) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.code === "Space") {
        event.preventDefault();
        setOpen(true);
      }
    }
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, []);

  async function capture(event: FormEvent) {
    event.preventDefault();
    if (!text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const item = await invoke<ContextItem>("capture_text_context", {
        repositoryId,
        taskId,
        title: title.trim() || null,
        text: text.trim(),
      });
      onCaptured(item);
      setTitle("");
      setText("");
      setOpen(false);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger render={<Button size="sm" type="button" variant="outline" />}>
        <ClipboardPlusIcon /> Capture text
      </DialogTrigger>
      <DialogContent>
        <form className="grid gap-4" onSubmit={(event) => void capture(event)}>
          <DialogHeader>
            <DialogTitle>Capture task context</DialogTitle>
            <DialogDescription>
              Preserve a note, error, excerpt, or instruction. Shortcut: Ctrl/⌘ + Shift + Space.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-1.5">
            <Label htmlFor="context-title">Title</Label>
            <Input id="context-title" onChange={(event) => setTitle(event.target.value)} value={title} />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="context-text">Text</Label>
            <Textarea id="context-text" onChange={(event) => setText(event.target.value)} rows={7} value={text} />
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          <DialogFooter>
            <Button disabled={busy || !text.trim()} type="submit">
              {busy ? "Capturing…" : "Capture and attach"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
