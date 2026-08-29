import type {
  CommandFailure,
  ContextArtifactContent,
  ContextItem,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangleIcon,
  FileIcon,
  ImageIcon,
  PaperclipIcon,
  SearchIcon,
  ShieldAlertIcon,
} from "lucide-react";
import {
  type ClipboardEvent,
  type ChangeEvent,
  type DragEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { QuickCapture } from "@/features/context/QuickCapture";

interface ContextTrayProps {
  repositoryId: string;
  taskId: string;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function ContextTray({ repositoryId, taskId }: ContextTrayProps) {
  const [items, setItems] = useState<ContextItem[]>([]);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  const search = useCallback(async (nextQuery: string) => {
    try {
      setItems(
        await invoke<ContextItem[]>("search_context", {
          repositoryId,
          taskId,
          query: nextQuery,
        }),
      );
      setError(null);
    } catch (caught) {
      setError(messageFor(caught));
    }
  }, [repositoryId, taskId]);

  useEffect(() => {
    let active = true;
    invoke<ContextItem[]>("search_context", { repositoryId, taskId, query: "" })
      .then((captured) => {
        if (active) setItems(captured);
      })
      .catch((caught: unknown) => {
        if (active) setError(messageFor(caught));
      });
    return () => {
      active = false;
    };
  }, [repositoryId, taskId]);

  function add(item: ContextItem) {
    if (item.quarantineReason) {
      setError(item.quarantineReason);
      return;
    }
    setItems((current) => [item, ...current.filter((entry) => entry.id !== item.id)]);
  }

  async function captureFiles(files: FileList | File[]) {
    setBusy(true);
    setError(null);
    try {
      for (const file of Array.from(files)) {
        const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
        add(
          await invoke<ContextItem>("capture_context_bytes", {
            repositoryId,
            taskId,
            title: null,
            originalName: file.name || "pasted-artifact",
            bytes,
            mimeType: file.type || "application/octet-stream",
          }),
        );
      }
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  function pasted(event: ClipboardEvent<HTMLElement>) {
    if (event.clipboardData.files.length === 0) return;
    event.preventDefault();
    void captureFiles(event.clipboardData.files);
  }

  function dropped(event: DragEvent<HTMLElement>) {
    event.preventDefault();
    if (event.dataTransfer.files.length > 0) void captureFiles(event.dataTransfer.files);
  }

  function selected(event: ChangeEvent<HTMLInputElement>) {
    if (event.target.files?.length) void captureFiles(event.target.files);
    event.target.value = "";
  }

  async function markConstraint(itemId: string) {
    setError(null);
    try {
      add(await invoke<ContextItem>("classify_context", { itemId, classification: "constraint" }));
    } catch (caught) {
      setError(messageFor(caught));
    }
  }

  return (
    <section
      aria-label="Context tray"
      className="grid gap-3 rounded-lg border border-dashed bg-muted/10 p-3"
      onDragOver={(event) => event.preventDefault()}
      onDrop={dropped}
      onPaste={pasted}
    >
      <header className="flex items-center justify-between gap-2">
        <div>
          <h3 className="text-xs font-medium">Context tray</h3>
          <p className="text-[10px] text-muted-foreground">Paste or drop images/files here.</p>
        </div>
        <div className="flex gap-1.5">
          <QuickCapture onCaptured={add} repositoryId={repositoryId} taskId={taskId} />
          <Button disabled={busy} onClick={() => fileInput.current?.click()} size="sm" type="button" variant="outline">
            <PaperclipIcon /> Attach file
          </Button>
          <input aria-label="Choose context files" className="sr-only" multiple onChange={selected} ref={fileInput} type="file" />
        </div>
      </header>
      <div className="relative">
        <SearchIcon className="absolute left-2.5 top-2.5 size-3.5 text-muted-foreground" />
        <Input
          aria-label="Search task context"
          className="pl-8"
          onChange={(event) => {
            setQuery(event.target.value);
            void search(event.target.value);
          }}
          placeholder="Search attached context"
          value={query}
        />
      </div>
      <div className="grid gap-2">
        {items.map((item) => (
          <ContextCard item={item} key={item.id} onMarkConstraint={markConstraint} />
        ))}
        {items.length === 0 ? (
          <p className="py-3 text-center text-xs text-muted-foreground">No context attached yet.</p>
        ) : null}
      </div>
      {error ? (
        <p className="flex items-center gap-1.5 text-xs text-destructive" role="alert">
          <ShieldAlertIcon className="size-3.5" /> {error}
        </p>
      ) : null}
    </section>
  );
}

function ContextCard({ item, onMarkConstraint }: { item: ContextItem; onMarkConstraint: (itemId: string) => Promise<void> }) {
  return (
    <article className="grid gap-2 rounded-md border bg-card p-2.5">
      <div className="flex items-start gap-2">
        {item.kind === "image" ? <ImageIcon className="mt-0.5 size-3.5 text-primary" /> : <FileIcon className="mt-0.5 size-3.5 text-primary" />}
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">{item.title}</p>
          <p className="truncate text-[10px] text-muted-foreground">{item.mimeType}</p>
        </div>
        {item.classification ? <Badge variant="outline">{item.classification}</Badge> : null}
      </div>
      {item.bodyText ? <p className="line-clamp-3 whitespace-pre-wrap text-xs text-muted-foreground">{item.bodyText}</p> : null}
      {item.kind === "image" ? <ArtifactImage item={item} /> : null}
      {item.quarantineReason ? (
        <p className="flex items-center gap-1 text-[10px] text-destructive"><AlertTriangleIcon className="size-3" /> {item.quarantineReason}</p>
      ) : (
        <Button className="justify-self-start" disabled={item.classification === "constraint"} onClick={() => void onMarkConstraint(item.id)} size="sm" type="button" variant="ghost">
          Use as constraint
        </Button>
      )}
    </article>
  );
}

function ArtifactImage({ item }: { item: ContextItem }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    let objectUrl: string | null = null;
    invoke<ContextArtifactContent>("read_context_artifact", { itemId: item.id }).then((content) => {
      if (!active) return;
      const buffer = Uint8Array.from(content.bytes).buffer;
      objectUrl = URL.createObjectURL(new Blob([buffer], { type: content.mimeType }));
      setUrl(objectUrl);
    });
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [item.id]);
  return url ? <img alt={item.title} className="max-h-40 rounded-md border object-contain" src={url} /> : null;
}
