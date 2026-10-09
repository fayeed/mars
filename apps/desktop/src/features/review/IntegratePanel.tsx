import type {
  CleanupPlan,
  CleanupView,
  CommandFailure,
  CommitInfo,
  IntegrationPreview,
  IntegrationStrategy,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { GitMergeIcon, RotateCcwIcon, Trash2Icon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface IntegratePanelProps {
  taskId: string;
  taskTitle: string;
  /** The worktree exists; removal and merging are offered only then. */
  worktreeReady: boolean;
  /** Called after a merge, a removal or a restore. */
  onChanged?: () => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

const STRATEGIES: { id: IntegrationStrategy; label: string; note: string }[] = [
  { id: "merge", label: "Merge commit", note: "keeps every commit the task made" },
  { id: "squash", label: "Squash", note: "lands the whole task as one commit" },
  { id: "fast_forward", label: "Fast-forward", note: "moves the base to the task; only when nothing else landed" },
];

/**
 * Bringing a task home, and putting its worktree away.
 *
 * The merge is worked out first in Git's object store — which commits, which
 * files, and which of those would conflict — without touching the base or the
 * worktree, and is applied only against that preview's fingerprint. A conflict
 * stops it and the base is left exactly as it was. Removing the worktree
 * archives its branch first, so it can be put back where it was.
 */
export function IntegratePanel({ onChanged, taskId, taskTitle, worktreeReady }: IntegratePanelProps) {
  const [preview, setPreview] = useState<IntegrationPreview | null>(null);
  const [previewState, setPreviewState] = useState<ActivityState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [strategy, setStrategy] = useState<IntegrationStrategy>("merge");
  const [message, setMessage] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [merged, setMerged] = useState<CommitInfo | null>(null);
  const [cleanup, setCleanup] = useState<CleanupPlan | null>(null);
  const [discardUncommitted, setDiscardUncommitted] = useState(false);
  const [lastCleanup, setLastCleanup] = useState<CleanupView | null>(null);

  const loadPreview = useCallback(() => {
    if (!worktreeReady) return;
    setPreviewState("loading");
    setError(null);
    invoke<IntegrationPreview>("integration_preview", { taskId })
      .then((next) => {
        setPreview(next);
        setPreviewState("idle");
        setStrategy(next.fastForward ? "fast_forward" : "merge");
        setMessage(`${taskTitle}`);
      })
      .catch((caught: unknown) => {
        setPreview(null);
        setPreviewState("failed");
        setError(messageFor(caught));
      });
  }, [taskId, taskTitle, worktreeReady]);

  useEffect(() => {
    invoke<CleanupView | null>("worktree_last_cleanup", { taskId })
      .then(setLastCleanup)
      .catch(() => setLastCleanup(null));
  }, [taskId, merged]);

  async function apply() {
    if (!preview) return;
    setBusy(true);
    setError(null);
    try {
      const result = await invoke<CommitInfo>("integration_apply", {
        taskId,
        fingerprint: preview.fingerprint,
        strategy,
        message,
      });
      setMerged(result);
      setConfirming(false);
      setPreview(null);
      onChanged?.();
    } catch (caught) {
      setError(messageFor(caught));
      setConfirming(false);
      loadPreview();
    } finally {
      setBusy(false);
    }
  }

  async function planRemoval() {
    setError(null);
    try {
      setCleanup(await invoke<CleanupPlan>("worktree_cleanup_plan", { taskId }));
      setDiscardUncommitted(false);
    } catch (caught) {
      setError(messageFor(caught));
    }
  }

  async function remove() {
    if (!cleanup) return;
    setBusy(true);
    try {
      await invoke("worktree_cleanup", {
        taskId,
        fingerprint: cleanup.fingerprint,
        discardUncommitted,
      });
      setCleanup(null);
      setLastCleanup(await invoke<CleanupView | null>("worktree_last_cleanup", { taskId }));
      onChanged?.();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function restore() {
    if (!lastCleanup) return;
    setBusy(true);
    try {
      await invoke("worktree_restore", { cleanupId: lastCleanup.id });
      setLastCleanup(await invoke<CleanupView | null>("worktree_last_cleanup", { taskId }));
      onChanged?.();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  const ready = preview !== null && preview.blockers.length === 0 && preview.conflicts.length === 0;
  const ffImpossible = preview !== null && strategy === "fast_forward" && !preview.fastForward;

  return (
    <div className="flex flex-col gap-3 p-3" data-slot="integrate-panel">
      {error ? (
        <p role="alert" className="text-[11px] text-destructive">
          {error}
        </p>
      ) : null}
      {merged ? (
        <StateLabel state="done" detail={`landed as ${merged.shortOid} — ${merged.subject}`} />
      ) : null}

      <section aria-label="Merge into the base" className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <MetaLabel>Bring this task home</MetaLabel>
          <button
            type="button"
            className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:text-faint"
            disabled={!worktreeReady || previewState === "loading"}
            onClick={loadPreview}
          >
            <GitMergeIcon className="size-3" />
            {preview ? "Preview again" : "Preview the merge"}
          </button>
        </div>
        {!worktreeReady ? (
          <p className="text-[11px] text-muted-foreground">The worktree is not there to merge from.</p>
        ) : previewState === "loading" ? (
          <StateBlock state="loading" title="Working out the merge" />
        ) : preview ? (
          <div className="flex flex-col gap-2">
            <p className="text-[12px] text-foreground">
              <Mono>{preview.head}</Mono> into <Mono>{preview.base}</Mono> in{" "}
              <Mono className="text-faint">{preview.root}</Mono>
            </p>
            <div className="flex flex-wrap gap-3">
              <Mono className="text-muted-foreground">{preview.commits.length} commit(s)</Mono>
              <Mono className="text-muted-foreground">{preview.filesChanged.length} file(s)</Mono>
              {preview.conflicts.length === 0 ? (
                <StateLabel state="done" detail="merges cleanly" />
              ) : (
                <StateLabel state="failed" detail={`${preview.conflicts.length} conflict(s)`} />
              )}
            </div>
            {preview.conflicts.length > 0 ? (
              <div className="flex flex-col gap-0.5">
                <MetaLabel>Would conflict</MetaLabel>
                {preview.conflicts.map((path) => (
                  <Mono key={path} className="text-destructive">
                    {path}
                  </Mono>
                ))}
                <p className="text-[11px] text-muted-foreground">
                  Bring the base into the task first — ask its agent to merge {preview.base} and resolve
                  these — then preview again.
                </p>
              </div>
            ) : null}
            {preview.blockers.map((blocker) => (
              <StateLabel key={blocker} state="blocked" detail={blocker} />
            ))}
            <details className="text-[11px] text-muted-foreground">
              <summary className="cursor-pointer outline-none">Commits and files</summary>
              <ul className="mt-1 flex flex-col">
                {preview.commits.map((commit) => (
                  <li key={commit.oid}>
                    <Mono className="text-faint">{commit.shortOid}</Mono> {commit.subject}
                  </li>
                ))}
              </ul>
              <ul className="mt-1 flex flex-col">
                {preview.filesChanged.map((line) => (
                  <li key={line}>
                    <Mono className="text-faint">{line}</Mono>
                  </li>
                ))}
              </ul>
            </details>
            {ready ? (
              <>
                <fieldset className="flex flex-col gap-1">
                  <legend className="sr-only">How to merge</legend>
                  {STRATEGIES.map((option) => (
                    <label key={option.id} className="flex items-center gap-1.5 text-[11px] text-foreground">
                      <input
                        type="radio"
                        name="integration-strategy"
                        checked={strategy === option.id}
                        disabled={option.id === "fast_forward" && !preview.fastForward}
                        onChange={() => setStrategy(option.id)}
                      />
                      {option.label}
                      <span className="text-faint">— {option.note}</span>
                    </label>
                  ))}
                </fieldset>
                {strategy !== "fast_forward" ? (
                  <input
                    aria-label="Merge message"
                    className="border border-border bg-transparent px-1.5 py-0.5 text-[12px] text-foreground outline-none"
                    onChange={(event) => setMessage(event.target.value)}
                    value={message}
                  />
                ) : null}
                {confirming ? (
                  <div role="alertdialog" aria-label="Confirm merge" className="flex flex-wrap items-center gap-2">
                    <span className="text-[12px] text-foreground">
                      Merge into <Mono>{preview.base}</Mono> now? It changes {preview.base} in your repository.
                    </span>
                    <button
                      type="button"
                      className="border border-border bg-primary px-2 py-0.5 text-[11px] text-primary-foreground outline-none"
                      disabled={busy}
                      onClick={() => void apply()}
                    >
                      {busy ? "Merging" : "Merge"}
                    </button>
                    <button
                      type="button"
                      className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none"
                      onClick={() => setConfirming(false)}
                    >
                      Not yet
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    className="w-fit border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:bg-transparent disabled:text-faint"
                    disabled={ffImpossible}
                    onClick={() => setConfirming(true)}
                  >
                    Merge into {preview.base}
                  </button>
                )}
              </>
            ) : null}
          </div>
        ) : (
          <p className="text-[11px] text-muted-foreground">
            Preview first: it says exactly what would land and whether it conflicts, and changes nothing.
          </p>
        )}
      </section>

      <section aria-label="Worktree" className="flex flex-col gap-2 border-t border-border pt-3">
        <MetaLabel>The worktree</MetaLabel>
        {worktreeReady ? (
          cleanup ? (
            <div className="flex flex-col gap-1.5">
              <p className="text-[12px] text-foreground">
                Remove <Mono>{cleanup.worktreePath}</Mono>? Its branch tip is archived to{" "}
                <Mono>{cleanup.archiveRef}</Mono> first, so it can be restored.
              </p>
              <Mono className="text-faint">{cleanup.commitsAheadOfBase} commit(s) ahead of the base</Mono>
              {cleanup.uncommitted.length > 0 ? (
                <>
                  <p className="text-[11px] text-destructive">
                    These uncommitted files cannot be brought back: {cleanup.uncommitted.join(", ")}
                  </p>
                  <label className="flex items-center gap-1.5 text-[11px] text-foreground">
                    <input
                      type="checkbox"
                      checked={discardUncommitted}
                      onChange={(event) => setDiscardUncommitted(event.target.checked)}
                    />
                    Discard them
                  </label>
                </>
              ) : null}
              <div className="flex gap-2">
                <button
                  type="button"
                  className="border border-destructive px-2 py-0.5 text-[11px] text-destructive outline-none disabled:border-border disabled:text-faint"
                  disabled={busy || (cleanup.uncommitted.length > 0 && !discardUncommitted)}
                  onClick={() => void remove()}
                >
                  Remove worktree
                </button>
                <button
                  type="button"
                  className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none"
                  onClick={() => setCleanup(null)}
                >
                  Keep it
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              className="flex w-fit items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong"
              onClick={() => void planRemoval()}
            >
              <Trash2Icon className="size-3" />
              Remove this worktree…
            </button>
          )
        ) : lastCleanup && lastCleanup.state === "removed" ? (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-[12px] text-foreground">
              Removed from <Mono>{lastCleanup.removedPath}</Mono>
              {lastCleanup.archivedRef ? (
                <>
                  ; archived at <Mono>{lastCleanup.archivedRef}</Mono>
                </>
              ) : null}
            </span>
            <button
              type="button"
              className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong"
              disabled={busy}
              onClick={() => void restore()}
            >
              <RotateCcwIcon className="size-3" />
              Restore it
            </button>
          </div>
        ) : (
          <p className="text-[11px] text-muted-foreground">There is no worktree for this task right now.</p>
        )}
      </section>
    </div>
  );
}
