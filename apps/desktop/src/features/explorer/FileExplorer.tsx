import type { CommandFailure, FileEntry, FilePage } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { AlertCircleIcon, FileCode2Icon, FolderTreeIcon, RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

interface FileExplorerProps {
  activePath: string | null;
  repositoryId: string;
  repositoryName: string;
  onOpen: (path: string) => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

async function listAllFiles(repositoryId: string) {
  const loaded: FileEntry[] = [];
  let cursor: string | null = null;
  do {
    const page: FilePage = await invoke<FilePage>("list_files", {
      repositoryId,
      cursor,
      limit: 500,
    });
    loaded.push(...page.entries);
    cursor = page.nextCursor;
  } while (cursor !== null);
  return loaded;
}

export function FileExplorer({
  activePath,
  repositoryId,
  repositoryName,
  onOpen,
}: FileExplorerProps) {
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setFiles(await listAllFiles(repositoryId));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setLoading(false);
    }
  }, [repositoryId]);

  useEffect(() => {
    let active = true;
    void listAllFiles(repositoryId)
      .then((loaded) => {
        if (active) setFiles(loaded);
      })
      .catch((caught: unknown) => {
        if (active) setError(messageFor(caught));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [repositoryId]);

  return (
    <aside className="flex h-full min-h-0 flex-col bg-card" aria-label="Repository files">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <FolderTreeIcon className="size-4 text-primary" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-xs font-medium">{repositoryName}</p>
          <p className="text-[10px] uppercase tracking-wider text-muted-foreground">
            {loading ? "Scanning…" : `${files.length} files`}
          </p>
        </div>
        <Button aria-label="Refresh files" onClick={() => void load()} size="icon-sm" type="button" variant="ghost">
          <RefreshCwIcon data-icon="inline-start" />
        </Button>
      </header>

      {error ? (
        <Alert className="m-2 w-auto" variant="destructive">
          <AlertCircleIcon />
          <AlertTitle>Explorer unavailable</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}

      <ScrollArea className="min-h-0 flex-1">
        <div className="p-2">
          {files.map((file) => {
            const depth = Math.max(0, file.relativePath.split("/").length - 1);
            const name = file.relativePath.split("/").at(-1);
            return (
              <Button
                className="w-full justify-start overflow-hidden font-normal"
                key={file.relativePath}
                onClick={() => onOpen(file.relativePath)}
                size="sm"
                style={{ paddingLeft: `${8 + Math.min(depth, 6) * 12}px` }}
                title={file.relativePath}
                type="button"
                variant={activePath === file.relativePath ? "secondary" : "ghost"}
              >
                <FileCode2Icon className="shrink-0 text-muted-foreground" data-icon="inline-start" />
                <span className="truncate">{name}</span>
              </Button>
            );
          })}
          {!loading && files.length === 0 && !error ? (
            <p className="px-2 py-8 text-center text-xs text-muted-foreground">No visible text files yet.</p>
          ) : null}
        </div>
      </ScrollArea>
    </aside>
  );
}
