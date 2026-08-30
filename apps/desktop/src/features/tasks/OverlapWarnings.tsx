import type { CommandFailure, OverlapWarning } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangleIcon, InfoIcon, LoaderCircleIcon, RefreshCwIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

export function OverlapWarnings({ taskId }: { taskId: string }) {
  const [warnings, setWarnings] = useState<OverlapWarning[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    setBusy(true);
    setError(null);
    try {
      setWarnings(await invoke<OverlapWarning[]>("task_overlap_warnings", { taskId }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    let active = true;
    void invoke<OverlapWarning[]>("task_overlap_warnings", { taskId })
      .then((items) => { if (active) setWarnings(items); })
      .catch((caught) => { if (active) setError(messageFor(caught)); });
    return () => { active = false; };
  }, [taskId]);

  return (
    <section className="grid gap-2 rounded-lg border p-3" aria-label="Parallel work overlap">
      <div className="flex items-center gap-2">
        <AlertTriangleIcon className="size-4" />
        <h3 className="text-sm font-medium">Parallel work overlap</h3>
        <Badge className="ml-auto" variant={warnings.length ? "outline" : "secondary"}>
          {warnings.length ? `${warnings.length} warning${warnings.length === 1 ? "" : "s"}` : "clear"}
        </Badge>
        <Button aria-label="Refresh overlap warnings" disabled={busy} onClick={() => void refresh()} size="icon-sm" type="button" variant="ghost">
          {busy ? <LoaderCircleIcon className="animate-spin" /> : <RefreshCwIcon />}
        </Button>
      </div>
      {warnings.map((warning) => (
        <article className="grid gap-1.5 rounded-md border bg-muted/15 p-2 text-xs" key={`${warning.otherTaskId}:${warning.kind}:${warning.evidence.join(":")}`}>
          <div className="flex items-center gap-2">
            {warning.severity === "info" ? <InfoIcon className="size-3.5" /> : <AlertTriangleIcon className={warning.severity === "error" ? "size-3.5 text-destructive" : "size-3.5 text-amber-600"} />}
            <Badge variant="outline">{warning.kind.replaceAll("_", " ")}</Badge>
            <span className="font-medium">{warning.otherTaskTitle}</span>
          </div>
          <p>{warning.message}</p>
          <code className="break-all text-[10px] text-muted-foreground">{warning.otherWorktreePath}</code>
          <p className="text-[10px] text-muted-foreground">{warning.evidence.join(" · ")} · refreshed {new Date(warning.refreshedAt).toLocaleTimeString()}</p>
        </article>
      ))}
      {!warnings.length && !error ? <p className="text-xs text-muted-foreground">No file, rename, symbol, or one-hop dependency overlap detected.</p> : null}
      {error ? <Alert variant="destructive"><AlertTitle>Overlap scan failed</AlertTitle><AlertDescription>{error}</AlertDescription></Alert> : null}
    </section>
  );
}
