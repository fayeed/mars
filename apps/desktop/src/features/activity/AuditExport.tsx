import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { DownloadIcon, FileTextIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { failureMessage } from "@/shell/failure";
import { useModalFocus } from "@/shell/modal";
import { FailureBlock, KeyHint, MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface AuditExportProps {
  open: boolean;
  onClose: () => void;
  taskId: string | null;
  /** Names the file before the reader is asked where to put it. */
  taskTitle?: string;
}

interface Written {
  path: string;
  byteSize: number;
}

/** A filename that says which task and when, without needing to be opened. */
export function auditFileName(taskTitle: string | undefined, taskId: string, when: Date): string {
  const slug = (taskTitle ?? taskId)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  const day = when.toISOString().slice(0, 10);
  return `${day}-${slug || "task"}-activity.md`;
}

/**
 * A task's record, as a document to read and to keep.
 *
 * Shown before it is saved, on purpose. This is the artefact somebody hands to
 * a reviewer, an auditor or their own future self, and the moment to find out
 * what is in it is before it leaves the machine — not after. Everything on
 * screen is rendered from the activity log alone, so what this says and what
 * the conversation showed cannot disagree.
 */
export function AuditExport({ onClose, open, taskId, taskTitle }: AuditExportProps) {
  const [document, setDocument] = useState<string | null>(null);
  const [state, setState] = useState<ActivityState>("loading");
  const [error, setError] = useState<unknown>(null);
  const [written, setWritten] = useState<Written | null>(null);
  const [attempt, setAttempt] = useState(0);
  const { ref, onBackdropMouseDown } = useModalFocus<HTMLDivElement>({ active: open, onClose });

  const retry = useCallback(() => setAttempt((current) => current + 1), []);

  // Rendered when the surface opens rather than behind another click: there is
  // nothing to decide about first, and somebody who asked to see the record
  // should see it.
  useEffect(() => {
    if (!open || !taskId) return;
    let active = true;
    invoke<string>("task_audit", { taskId })
      .then((rendered) => {
        if (!active) return;
        setDocument(rendered);
        setWritten(null);
        setError(null);
        setState("done");
      })
      .catch((caught: unknown) => {
        if (!active) return;
        setError(caught);
        setState("failed");
      });
    return () => {
      active = false;
    };
  }, [attempt, open, taskId]);

  async function saveToDisk() {
    if (!taskId) return;
    const destination = await saveDialog({
      defaultPath: auditFileName(taskTitle, taskId, new Date()),
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    // No path means the dialog was dismissed, which is a choice and not a
    // failure; nothing is written and nothing is said about it.
    if (!destination) return;
    try {
      setWritten(await invoke<Written>("write_task_audit", { taskId, destination }));
      setError(null);
    } catch (caught) {
      setError(caught);
      setState("failed");
    }
  }

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-sunken/70 p-8"
      onMouseDown={onBackdropMouseDown}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label="Activity record"
        tabIndex={-1}
        className="flex h-full w-[min(56rem,96vw)] flex-col border border-border bg-surface outline-none"
      >
        <header className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
          <FileTextIcon className="size-3.5 shrink-0 text-faint" />
          <MetaLabel className="text-foreground">Activity record</MetaLabel>
          {document ? (
            <Mono className="text-faint">{document.length.toLocaleString()} bytes</Mono>
          ) : null}
          <button
            type="button"
            disabled={!document}
            onClick={() => void saveToDisk()}
            className="ml-auto flex items-center gap-1.5 border border-border px-2 py-0.5 text-[11px] text-foreground hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <DownloadIcon className="size-3" />
            Save as Markdown
          </button>
          <button
            type="button"
            onClick={onClose}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground hover:border-border-strong"
          >
            Close
            <KeyHint className="border-transparent" keys="Esc" />
          </button>
        </header>

        {written ? (
          <p className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1.5">
            <StateLabel state="done" detail={`${written.byteSize.toLocaleString()} bytes written`} />
            <Mono className="min-w-0 truncate text-faint">{written.path}</Mono>
          </p>
        ) : null}

        <div className="min-h-0 flex-1 overflow-auto bg-background">
          {state === "loading" ? (
            <StateBlock state="loading" title="Reading the record" />
          ) : state === "failed" ? (
            <FailureBlock
              error={error}
              onRemedy={retry}
              remedyLabel="Try again"
              title="The record could not be exported"
            />
          ) : document ? (
            // Shown as the exact bytes that will be written. Rendering the
            // Markdown here would put a second interpretation between the
            // reader and the artefact they are about to hand to someone.
            <pre className="whitespace-pre-wrap px-3 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
              {document}
            </pre>
          ) : (
            <StateBlock
              state="idle"
              title="No task selected"
              description="An activity record belongs to one task. Pick one and export it from there."
            />
          )}
        </div>

        <p className="shrink-0 border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground">
          Rendered from the append-only activity log alone — transcript, commands, diffs and
          permission decisions, including the requests nobody answered.
          {error && state !== "failed" ? (
            <>
              {" "}
              <Mono className="text-destructive">{failureMessage(error)}</Mono>
            </>
          ) : null}
        </p>
      </div>
    </div>
  );
}
