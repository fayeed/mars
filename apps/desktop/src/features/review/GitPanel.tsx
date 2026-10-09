import type {
  BranchInfo,
  CommandFailure,
  CommitInfo,
  GitFileChange,
  GitTarget,
  RepoStatus,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  GitBranchIcon,
  MinusIcon,
  PlusIcon,
  RefreshCwIcon,
  Undo2Icon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

import { DiffText } from "./DiffText";

export interface GitPanelProps {
  target: GitTarget;
  /** Called after anything changes the working directory or its history. */
  onChanged?: () => void;
  onOpenFile?: (path: string, line: number | null) => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

const LETTER: Record<string, string> = {
  M: "modified",
  A: "added",
  D: "deleted",
  R: "renamed",
  C: "copied",
  T: "type changed",
  U: "conflict",
  "?": "untracked",
};

/** The groups a change falls into, the way a person works through them. */
export function groupChanges(changes: GitFileChange[]) {
  return {
    conflicted: changes.filter((change) => change.conflicted),
    staged: changes.filter((change) => !change.conflicted && !change.untracked && change.staged !== "."),
    unstaged: changes.filter(
      (change) => !change.conflicted && !change.untracked && change.unstaged !== ".",
    ),
    untracked: changes.filter((change) => change.untracked),
  };
}

type Tab = "changes" | "history";
type Selection = { path: string; staged: boolean } | { commit: string } | null;

/**
 * One working directory under Git, the way a terminal would show it and
 * nothing more: what changed, what is staged, what was committed, and where
 * the branch stands against its upstream. Every write is one explicit action
 * on exactly what is named; nothing is staged as a side effect of
 * committing, and publishing asks first.
 */
export function GitPanel({ onChanged, onOpenFile, target: requested }: GitPanelProps) {
  // A new object naming the same directory is the same target; reading again
  // for it would throw away the reader's selection for nothing.
  const targetKey = JSON.stringify(requested);
  const target = useMemo<GitTarget>(() => JSON.parse(targetKey) as GitTarget, [targetKey]);
  const [tab, setTab] = useState<Tab>("changes");
  const [status, setStatus] = useState<RepoStatus | null>(null);
  const [statusState, setStatusState] = useState<ActivityState>("loading");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [selection, setSelection] = useState<Selection>(null);
  /** A diff, with the selection it was read for; anything else is still loading. */
  const [loaded, setLoaded] = useState<{ key: string; text: string } | null>(null);
  const [message, setMessage] = useState("");
  const [amend, setAmend] = useState(false);
  /** Null until the first page arrives. */
  const [commits, setCommits] = useState<CommitInfo[] | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [newBranch, setNewBranch] = useState("");
  const [confirmDiscard, setConfirmDiscard] = useState<string[] | null>(null);
  const [confirmPush, setConfirmPush] = useState<{ remote: string; branch: string } | null>(null);
  const [remotes, setRemotes] = useState<string[]>([]);

  const isRepository = target.kind === "repository";

  const refresh = useCallback(() => {
    return invoke<RepoStatus>("git_status", { target })
      .then((next) => {
        setStatus(next);
        setStatusState("idle");
        setError(null);
      })
      .catch((caught: unknown) => {
        setStatusState("failed");
        setError(messageFor(caught));
      });
  }, [target]);

  // The panel is keyed on its target by whoever renders it, so a different
  // working directory starts from nothing rather than resetting in here.
  useEffect(() => {
    void refresh();
    invoke<string[]>("git_remotes", { target })
      .then(setRemotes)
      .catch(() => setRemotes([]));
    if (isRepository) {
      invoke<BranchInfo[]>("git_branches", { target })
        .then(setBranches)
        .catch(() => setBranches([]));
    }
  }, [isRepository, refresh, target]);

  useEffect(() => {
    if (tab !== "history") return;
    invoke<CommitInfo[]>("git_log", { target, limit: 100, skip: 0 })
      .then(setCommits)
      .catch((caught: unknown) => {
        setCommits([]);
        setError(messageFor(caught));
      });
  }, [tab, target, status?.headOid]);

  const selectionKey =
    selection === null
      ? null
      : "commit" in selection
        ? `commit:${selection.commit}`
        : `${selection.staged ? "staged" : "unstaged"}:${selection.path}`;

  // Whatever is selected is read again whenever the working directory moves.
  // The previous reading stays on screen until the new one arrives.
  useEffect(() => {
    if (!selection || !selectionKey) return;
    const request =
      "commit" in selection
        ? invoke<string>("git_show", { target, oid: selection.commit })
        : invoke<string>("git_diff", { target, path: selection.path, staged: selection.staged });
    request
      .then((text) => setLoaded({ key: selectionKey, text }))
      .catch((caught: unknown) => {
        setLoaded({ key: selectionKey, text: "" });
        setError(messageFor(caught));
      });
  }, [selection, selectionKey, target, status]);

  async function run(label: string, action: () => Promise<unknown>) {
    setBusy(label);
    setError(null);
    try {
      const result = await action();
      if (result && typeof result === "object" && "changes" in (result as object)) {
        setStatus(result as RepoStatus);
      } else {
        await refresh();
      }
      if (isRepository) {
        invoke<BranchInfo[]>("git_branches", { target })
          .then(setBranches)
          .catch(() => undefined);
      }
      onChanged?.();
      return true;
    } catch (caught) {
      setError(messageFor(caught));
      return false;
    } finally {
      setBusy(null);
    }
  }

  const groups = useMemo(() => groupChanges(status?.changes ?? []), [status]);
  const stagedCount = groups.staged.length;

  if (statusState === "loading" && !status) {
    return <StateBlock state="loading" title="Reading Git" />;
  }
  if (statusState === "failed" && !status) {
    return <StateBlock state="failed" title="Git could not be read" description={error ?? undefined} />;
  }
  if (!status) return null;

  const branch = status.branch ?? "detached HEAD";
  const upstreamRemote = status.upstream?.split("/")[0] ?? remotes[0] ?? null;

  const fileRow = (change: GitFileChange, staged: boolean) => {
    const letter = staged ? change.staged : change.unstaged;
    const selected =
      selection !== null && "path" in selection && selection.path === change.path && selection.staged === staged;
    return (
      <li key={`${staged ? "s" : "u"}:${change.path}`} className="group flex items-center gap-1 pr-1">
        <button
          type="button"
          onClick={() => setSelection({ path: change.path, staged })}
          aria-current={selected ? "true" : undefined}
          className={cn(
            "flex min-w-0 flex-1 items-center gap-2 px-2 py-0.5 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised",
            selected && "bg-surface-raised",
          )}
          aria-label={`${change.path}, ${LETTER[letter] ?? "changed"}${staged ? ", staged" : ""}`}
        >
          <Mono className="w-3 shrink-0 text-faint">{letter === "." ? "" : letter}</Mono>
          <span className="min-w-0 truncate font-mono text-[11px] text-foreground">{change.path}</span>
          {change.originalPath ? (
            <Mono className="truncate text-faint">from {change.originalPath}</Mono>
          ) : null}
        </button>
        {staged ? (
          <button
            type="button"
            aria-label={`Unstage ${change.path}`}
            className="text-faint outline-none hover:text-foreground focus-visible:text-foreground"
            disabled={busy !== null}
            onClick={() =>
              void run("unstage", () => invoke("git_unstage", { target, paths: [change.path] }))
            }
          >
            <MinusIcon className="size-3.5" />
          </button>
        ) : (
          <>
            <button
              type="button"
              aria-label={`Discard ${change.path}`}
              className="text-faint outline-none hover:text-destructive focus-visible:text-destructive"
              disabled={busy !== null}
              onClick={() => setConfirmDiscard([change.path])}
            >
              <Undo2Icon className="size-3.5" />
            </button>
            <button
              type="button"
              aria-label={`Stage ${change.path}`}
              className="text-faint outline-none hover:text-foreground focus-visible:text-foreground"
              disabled={busy !== null}
              onClick={() =>
                void run("stage", () => invoke("git_stage", { target, paths: [change.path] }))
              }
            >
              <PlusIcon className="size-3.5" />
            </button>
          </>
        )}
      </li>
    );
  };

  const group = (title: string, changes: GitFileChange[], staged: boolean, action?: React.ReactNode) =>
    changes.length === 0 ? null : (
      <section aria-label={title} className="border-b border-border py-1">
        <div className="flex items-center gap-2 px-2 py-0.5">
          <MetaLabel>{title}</MetaLabel>
          <Mono className="text-faint">{changes.length}</Mono>
          <span className="ml-auto">{action}</span>
        </div>
        <ul>{changes.map((change) => fileRow(change, staged))}</ul>
      </section>
    );

  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="git-panel">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
        <GitBranchIcon className="size-3.5 text-faint" />
        {isRepository && branches.length > 0 ? (
          <select
            aria-label="Branch"
            className="border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none"
            value={status.branch ?? ""}
            disabled={busy !== null}
            onChange={(event) =>
              void run("switch", () =>
                invoke("git_switch", { target, branch: event.target.value }),
              )
            }
          >
            {status.branch === null ? <option value="">detached HEAD</option> : null}
            {branches.map((candidate) => (
              <option
                key={candidate.name}
                value={candidate.name}
                disabled={candidate.checkedOutElsewhere}
              >
                {candidate.name}
                {candidate.checkedOutElsewhere ? " (in a worktree)" : ""}
              </option>
            ))}
          </select>
        ) : (
          <Mono className="text-foreground">{branch}</Mono>
        )}
        {status.upstream ? (
          <Mono className="text-faint">
            {status.upstream} · ↑{status.ahead} ↓{status.behind}
          </Mono>
        ) : (
          <Mono className="text-faint">no upstream</Mono>
        )}
        {status.operation ? <StateLabel state="blocked" detail={`${status.operation} in progress`} /> : null}
        <div className="ml-auto flex items-center gap-1">
          <button
            type="button"
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:text-faint"
            disabled={busy !== null || remotes.length === 0}
            onClick={() => void run("fetch", () => invoke("git_fetch", { target }))}
          >
            <RefreshCwIcon className="size-3" />
            {busy === "fetch" ? "Fetching" : "Fetch"}
          </button>
          <button
            type="button"
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:text-faint"
            disabled={busy !== null || !status.upstream || status.behind === 0}
            onClick={() => void run("pull", () => invoke("git_pull", { target }))}
          >
            <ArrowDownIcon className="size-3" />
            {busy === "pull" ? "Pulling" : "Pull"}
          </button>
          <button
            type="button"
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:text-faint"
            disabled={busy !== null || !status.branch || !upstreamRemote}
            onClick={() =>
              status.branch && upstreamRemote
                ? setConfirmPush({ remote: upstreamRemote, branch: status.branch })
                : undefined
            }
          >
            <ArrowUpIcon className="size-3" />
            Push
          </button>
        </div>
      </div>

      {confirmPush ? (
        <div
          role="alertdialog"
          aria-label="Confirm push"
          className="flex flex-wrap items-center gap-2 border-b border-border bg-surface-raised px-2 py-1.5"
        >
          <span className="text-[12px] text-foreground">
            Publish <Mono>{confirmPush.branch}</Mono> to{" "}
            {remotes.length > 1 ? (
              <select
                aria-label="Remote"
                className="border border-border bg-transparent px-1 font-mono text-[11px]"
                value={confirmPush.remote}
                onChange={(event) => setConfirmPush({ ...confirmPush, remote: event.target.value })}
              >
                {remotes.map((remote) => (
                  <option key={remote}>{remote}</option>
                ))}
              </select>
            ) : (
              <Mono>{confirmPush.remote}</Mono>
            )}
            ? Anyone with access to it will see these commits.
          </span>
          <button
            type="button"
            className="ml-auto border border-border bg-primary px-2 py-0.5 text-[11px] text-primary-foreground outline-none"
            disabled={busy !== null}
            onClick={() => {
              const { branch: pushBranch, remote } = confirmPush;
              setConfirmPush(null);
              void run("push", () => invoke("git_push", { target, remote, branch: pushBranch }));
            }}
          >
            Push
          </button>
          <button
            type="button"
            className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none"
            onClick={() => setConfirmPush(null)}
          >
            Cancel
          </button>
        </div>
      ) : null}

      {confirmDiscard ? (
        <div
          role="alertdialog"
          aria-label="Confirm discard"
          className="flex flex-wrap items-center gap-2 border-b border-border bg-surface-raised px-2 py-1.5"
        >
          <span className="text-[12px] text-foreground">
            Throw away the changes to <Mono>{confirmDiscard.join(", ")}</Mono>? This cannot be undone.
          </span>
          <button
            type="button"
            className="ml-auto border border-destructive px-2 py-0.5 text-[11px] text-destructive outline-none"
            onClick={() => {
              const paths = confirmDiscard;
              setConfirmDiscard(null);
              setSelection(null);
              void run("discard", () => invoke("git_discard", { target, paths }));
            }}
          >
            Discard
          </button>
          <button
            type="button"
            className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none"
            onClick={() => setConfirmDiscard(null)}
          >
            Keep
          </button>
        </div>
      ) : null}

      {error ? (
        <p role="alert" className="border-b border-border px-2 py-1 text-[11px] text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex items-center gap-1 border-b border-border px-2 py-1" role="tablist" aria-label="Git views">
        {(["changes", "history"] as const).map((candidate) => (
          <button
            key={candidate}
            type="button"
            role="tab"
            aria-selected={tab === candidate}
            onClick={() => {
              setTab(candidate);
              setSelection(null);
            }}
            className={cn(
              "border px-2 py-0.5 text-[11px] outline-none",
              tab === candidate ? "border-primary text-foreground" : "border-border text-muted-foreground",
            )}
          >
            {candidate === "changes" ? `Changes (${status.changes.length})` : "History"}
          </button>
        ))}
        {isRepository ? (
          <form
            className="ml-auto flex items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault();
              const name = newBranch.trim();
              if (!name) return;
              void run("branch", () => invoke("git_create_branch", { target, name, start: null })).then(
                (ok) => {
                  if (ok) setNewBranch("");
                },
              );
            }}
          >
            <input
              aria-label="New branch"
              className="w-36 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] outline-none placeholder:text-faint"
              onChange={(event) => setNewBranch(event.target.value)}
              placeholder="new-branch"
              value={newBranch}
            />
            <button
              type="submit"
              className="border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none disabled:text-faint"
              disabled={busy !== null || !newBranch.trim()}
            >
              Create branch
            </button>
          </form>
        ) : null}
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="flex w-[22rem] shrink-0 flex-col border-r border-border">
          {tab === "changes" ? (
            <>
              <div className="min-h-0 flex-1 overflow-auto">
                {status.changes.length === 0 ? (
                  <StateBlock state="done" title="Nothing changed" description="The working directory matches its last commit." />
                ) : (
                  <>
                    {group("Conflicted", groups.conflicted, false)}
                    {group(
                      "Staged",
                      groups.staged,
                      true,
                      <button
                        type="button"
                        className="text-[11px] text-muted-foreground outline-none hover:text-foreground"
                        disabled={busy !== null}
                        onClick={() =>
                          void run("unstage", () =>
                            invoke("git_unstage", {
                              target,
                              paths: groups.staged.map((change) => change.path),
                            }),
                          )
                        }
                      >
                        Unstage all
                      </button>,
                    )}
                    {group(
                      "Changes",
                      groups.unstaged,
                      false,
                      <button
                        type="button"
                        className="text-[11px] text-muted-foreground outline-none hover:text-foreground"
                        disabled={busy !== null}
                        onClick={() =>
                          void run("stage", () =>
                            invoke("git_stage", {
                              target,
                              paths: groups.unstaged.map((change) => change.path),
                            }),
                          )
                        }
                      >
                        Stage all
                      </button>,
                    )}
                    {group(
                      "Untracked",
                      groups.untracked,
                      false,
                      <button
                        type="button"
                        className="text-[11px] text-muted-foreground outline-none hover:text-foreground"
                        disabled={busy !== null}
                        onClick={() =>
                          void run("stage", () =>
                            invoke("git_stage", {
                              target,
                              paths: groups.untracked.map((change) => change.path),
                            }),
                          )
                        }
                      >
                        Stage all
                      </button>,
                    )}
                  </>
                )}
              </div>
              <form
                aria-label="Commit"
                className="flex shrink-0 flex-col gap-1 border-t border-border p-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  void run("commit", () => invoke("git_commit", { target, message, amend })).then((ok) => {
                    if (ok) {
                      setMessage("");
                      setAmend(false);
                    }
                  });
                }}
              >
                <textarea
                  aria-label="Commit message"
                  className="min-h-14 resize-none border border-border bg-transparent px-1.5 py-1 text-[12px] text-foreground outline-none placeholder:text-faint"
                  onChange={(event) => setMessage(event.target.value)}
                  placeholder={
                    stagedCount === 0 && !amend
                      ? "Stage the files this commit should contain first"
                      : `Message for ${stagedCount} staged file(s)`
                  }
                  value={message}
                />
                <div className="flex items-center gap-2">
                  <label className="flex items-center gap-1 text-[11px] text-muted-foreground">
                    <input type="checkbox" checked={amend} onChange={(event) => setAmend(event.target.checked)} />
                    Amend last commit
                  </label>
                  <button
                    type="submit"
                    className="ml-auto border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:bg-transparent disabled:text-faint"
                    disabled={busy !== null || !message.trim() || (stagedCount === 0 && !amend)}
                  >
                    {busy === "commit" ? "Committing" : "Commit"}
                  </button>
                </div>
              </form>
            </>
          ) : commits === null ? (
            <StateBlock state="loading" title="Reading history" />
          ) : commits.length === 0 ? (
            <StateBlock state="idle" title="No commits yet" />
          ) : (
            <ul aria-label="History" className="min-h-0 flex-1 overflow-auto">
              {commits.map((commit) => {
                const selected = selection !== null && "commit" in selection && selection.commit === commit.oid;
                return (
                  <li key={commit.oid}>
                    <button
                      type="button"
                      onClick={() => setSelection({ commit: commit.oid })}
                      aria-current={selected ? "true" : undefined}
                      className={cn(
                        "flex w-full flex-col gap-0.5 border-b border-border px-2 py-1 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised",
                        selected && "bg-surface-raised",
                      )}
                    >
                      <span className="truncate text-[12px] text-foreground">{commit.subject}</span>
                      <span className="flex items-center gap-2">
                        <Mono className="text-faint">{commit.shortOid}</Mono>
                        <Mono className="truncate text-faint">{commit.author}</Mono>
                        <Mono className="ml-auto shrink-0 text-faint">
                          {new Date(commit.authoredAt).toLocaleDateString()}
                        </Mono>
                      </span>
                      {commit.refs.length > 0 ? (
                        <Mono className="truncate text-primary">{commit.refs.join(", ")}</Mono>
                      ) : null}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        <div className="min-w-0 flex-1 overflow-auto">
          {selection === null ? (
            <StateBlock
              state="idle"
              title={tab === "changes" ? "Pick a file to see its change" : "Pick a commit to see what it changed"}
            />
          ) : loaded?.key !== selectionKey ? (
            <StateBlock state="loading" title="Reading the change" />
          ) : (
            <DiffText
              diff={loaded?.text ?? ""}
              emptyLabel="Nothing to show for this file"
              onOpenFile={onOpenFile}
            />
          )}
        </div>
      </div>
    </div>
  );
}
