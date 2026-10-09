import type {
  CommandFailure,
  ForgeIssue,
  ForgeIssueDetail,
  ForgePullRequest,
  ForgePullRequestDetail,
  IssueComment,
  ItemState,
  MergeMethod,
  MergeOutcome,
  RemoteForge,
  WorktreeProposal,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { ExternalLinkIcon, GitPullRequestIcon, MessageSquareIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface GitHubPanelProps {
  repositoryId: string;
  /** The branch a task started from an issue is based on. */
  baseBranch: string;
  /** A task was planned from an issue; the screen shows its plan. */
  onTaskPlanned?: (proposal: WorktreeProposal) => void;
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}

function openUrl(url: string) {
  void invoke("open_external_url", { url }).catch(() => undefined);
}

type View = "issues" | "pulls";
type Selected = { kind: "issue"; number: number } | { kind: "pull"; number: number } | null;

const CHECK_STATE: Record<string, ActivityState> = {
  success: "done",
  failure: "failed",
  cancelled: "failed",
  timed_out: "failed",
  action_required: "blocked",
  neutral: "idle",
  skipped: "idle",
};

/**
 * GitHub for this repository, without leaving the app: issues and pull
 * requests to browse and read, a conversation to answer, a pull request to
 * merge, and an issue to turn into a task. Every write names the account it
 * is made as and is confirmed first; a merge is pinned to the head on screen.
 */
export function GitHubPanel({ baseBranch, onTaskPlanned, repositoryId }: GitHubPanelProps) {
  const [remotes, setRemotes] = useState<RemoteForge[] | null>(null);
  const [remoteName, setRemoteName] = useState<string | null>(null);
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<View>("issues");
  const [itemState, setItemState] = useState<ItemState>("open");
  const [issues, setIssues] = useState<ForgeIssue[] | null>(null);
  const [pulls, setPulls] = useState<ForgePullRequest[] | null>(null);
  const [selected, setSelected] = useState<Selected>(null);
  const [issueDetail, setIssueDetail] = useState<ForgeIssueDetail | null>(null);
  const [pullDetail, setPullDetail] = useState<ForgePullRequestDetail | null>(null);
  const [reply, setReply] = useState("");
  const [confirmReply, setConfirmReply] = useState(false);
  const [mergeMethod, setMergeMethod] = useState<MergeMethod>("squash");
  const [confirmMerge, setConfirmMerge] = useState(false);
  const [busy, setBusy] = useState(false);
  const [scope, setScope] = useState<"ask" | "all">("ask");
  const [notice, setNotice] = useState<string | null>(null);

  const loadRemotes = useCallback(
    () =>
      invoke<RemoteForge[]>("forge_status", { repositoryId })
        .then((items) => {
          setRemotes(items);
          setRemoteName(
            (current) =>
              current ??
              items.find((remote) => remote.provider === "github" && remote.name === "origin")?.name ??
              items.find((remote) => remote.provider === "github")?.name ??
              null,
          );
        })
        .catch((caught: unknown) => {
          setRemotes([]);
          setError(messageFor(caught));
        }),
    [repositoryId],
  );

  useEffect(() => {
    void loadRemotes();
  }, [loadRemotes]);

  const remote = useMemo(
    () => remotes?.find((candidate) => candidate.name === remoteName) ?? null,
    [remoteName, remotes],
  );
  const connected = Boolean(remote?.connected);

  // Lists are read when the remote, the view or the filter changes.
  useEffect(() => {
    if (!remote || !connected) return;
    const args = { repositoryId, remoteName: remote.name, itemState, page: 1 };
    if (view === "issues") {
      invoke<ForgeIssue[]>("github_issues", args)
        .then((items) => {
          setIssues(items);
          setError(null);
        })
        .catch((caught: unknown) => {
          setIssues([]);
          setError(messageFor(caught));
        });
    } else {
      invoke<ForgePullRequest[]>("github_pull_requests", args)
        .then((items) => {
          setPulls(items);
          setError(null);
        })
        .catch((caught: unknown) => {
          setPulls([]);
          setError(messageFor(caught));
        });
    }
  }, [connected, itemState, remote, repositoryId, view]);

  const loadSelected = useCallback(() => {
    if (!selected || !remote) return;
    const args = { repositoryId, remoteName: remote.name, number: selected.number };
    if (selected.kind === "issue") {
      invoke<ForgeIssueDetail>("github_issue", args)
        .then(setIssueDetail)
        .catch((caught: unknown) => setError(messageFor(caught)));
    } else {
      invoke<ForgePullRequestDetail>("github_pull_request", args)
        .then(setPullDetail)
        .catch((caught: unknown) => setError(messageFor(caught)));
    }
  }, [remote, repositoryId, selected]);

  useEffect(() => {
    loadSelected();
  }, [loadSelected]);

  async function connect(withCli: boolean) {
    if (!remote) return;
    setConnecting(true);
    setError(null);
    try {
      if (withCli) {
        await invoke("connect_github_cli", { repositoryId, remoteName: remote.name });
      } else {
        await invoke("connect_github", { repositoryId, remoteName: remote.name, token });
      }
      setToken("");
      await loadRemotes();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setConnecting(false);
    }
  }

  async function postReply() {
    if (!selected || !remote) return;
    setBusy(true);
    try {
      await invoke<IssueComment>("github_comment", {
        repositoryId,
        remoteName: remote.name,
        number: selected.number,
        body: reply,
      });
      setReply("");
      setConfirmReply(false);
      loadSelected();
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  async function merge() {
    if (!pullDetail || !remote) return;
    setBusy(true);
    try {
      const outcome = await invoke<MergeOutcome>("github_merge", {
        repositoryId,
        remoteName: remote.name,
        number: pullDetail.pull.number,
        method: mergeMethod,
        expectedHead: pullDetail.pull.headSha,
      });
      setNotice(outcome.merged ? `Merged #${pullDetail.pull.number}` : outcome.message);
      setConfirmMerge(false);
      loadSelected();
    } catch (caught) {
      setError(messageFor(caught));
      setConfirmMerge(false);
    } finally {
      setBusy(false);
    }
  }

  async function startTask(issue: ForgeIssue) {
    if (!remote) return;
    setBusy(true);
    try {
      const proposal = await invoke<WorktreeProposal>("start_task_from_issue", {
        repositoryId,
        remoteName: remote.name,
        number: issue.number,
        baseRef: baseBranch,
        expectedScope: scope === "all" ? ["**"] : [],
      });
      setNotice(`Planned a task for #${issue.number}. Approve its plan in the conversation.`);
      onTaskPlanned?.(proposal);
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  if (remotes === null) return <StateBlock state="loading" title="Reading the repository's remotes" />;
  const github = remotes.filter((candidate) => candidate.provider === "github");
  if (github.length === 0) {
    return (
      <StateBlock
        state="idle"
        title="No GitHub remote"
        description="This repository has no remote on GitHub or GitHub Enterprise. Add one with git remote add, then come back."
      />
    );
  }

  const header = (
    <div className="flex flex-wrap items-center gap-2 border-b border-border px-2 py-1.5">
      {github.length > 1 ? (
        <select
          aria-label="Remote"
          className="border border-border bg-transparent px-1 font-mono text-[11px]"
          value={remoteName ?? ""}
          onChange={(event) => {
            setRemoteName(event.target.value);
            setSelected(null);
          }}
        >
          {github.map((candidate) => (
            <option key={candidate.name} value={candidate.name}>
              {candidate.name}
            </option>
          ))}
        </select>
      ) : null}
      {remote ? (
        <Mono className="text-foreground">
          {remote.owner}/{remote.repository}
        </Mono>
      ) : null}
      {remote?.connected ? (
        <Mono className="text-faint">as @{remote.account}</Mono>
      ) : null}
      {notice ? (
        <span role="status" className="ml-auto truncate text-[11px] text-muted-foreground">
          {notice}
        </span>
      ) : null}
    </div>
  );

  if (!remote || !connected) {
    return (
      <div className="flex flex-col">
        {header}
        <section aria-label="Connect GitHub" className="flex max-w-xl flex-col gap-2 p-3">
          <MetaLabel>Connect {remote?.host ?? "GitHub"}</MetaLabel>
          <p className="text-[12px] leading-relaxed text-muted-foreground">
            Mars reads issues and pull requests and posts as the account you connect. The token goes
            to your operating system's keychain; Mars keeps only a reference to it.
          </p>
          <button
            type="button"
            className="w-fit border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:bg-transparent disabled:text-faint"
            disabled={connecting}
            onClick={() => void connect(true)}
          >
            Use my GitHub CLI sign-in
          </button>
          <form
            className="flex items-center gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              void connect(false);
            }}
          >
            <input
              aria-label="Personal access token"
              autoComplete="off"
              className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] outline-none placeholder:text-faint"
              onChange={(event) => setToken(event.target.value)}
              placeholder="or paste a personal access token"
              type="password"
              value={token}
            />
            <button
              type="submit"
              className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none disabled:text-faint"
              disabled={connecting || !token.trim()}
            >
              Connect
            </button>
          </form>
          {error ? (
            <p role="alert" className="text-[11px] text-destructive">
              {error}
            </p>
          ) : null}
        </section>
      </div>
    );
  }

  const list = view === "issues" ? issues : pulls;
  return (
    <div className="flex h-full min-h-0 flex-col" data-slot="github-panel">
      {header}
      <div className="flex items-center gap-1 border-b border-border px-2 py-1">
        <div role="tablist" aria-label="GitHub views" className="flex items-center gap-1">
          {(
            [
              ["issues", "Issues"],
              ["pulls", "Pull requests"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={view === id}
              onClick={() => {
                setView(id);
                setSelected(null);
              }}
              className={cn(
                "border px-2 py-0.5 text-[11px] outline-none",
                view === id ? "border-primary text-foreground" : "border-border text-muted-foreground",
              )}
            >
              {label}
            </button>
          ))}
        </div>
        <select
          aria-label="State"
          className="ml-auto border border-border bg-transparent px-1 text-[11px]"
          value={itemState}
          onChange={(event) => setItemState(event.target.value as ItemState)}
        >
          <option value="open">Open</option>
          <option value="closed">Closed</option>
          <option value="all">All</option>
        </select>
      </div>
      {error ? (
        <p role="alert" className="border-b border-border px-2 py-1 text-[11px] text-destructive">
          {error}
        </p>
      ) : null}
      <div className="flex min-h-0 flex-1">
        <ul
          aria-label={view === "issues" ? "Issues" : "Pull requests"}
          className="w-[22rem] shrink-0 overflow-auto border-r border-border"
        >
          {list === null ? (
            <li>
              <StateBlock state="loading" title="Reading GitHub" />
            </li>
          ) : list.length === 0 ? (
            <li>
              <StateBlock state="idle" title={view === "issues" ? "No issues" : "No pull requests"} />
            </li>
          ) : (
            list.map((item) => {
              const current = selected?.number === item.number;
              return (
                <li key={item.number}>
                  <button
                    type="button"
                    aria-current={current ? "true" : undefined}
                    onClick={() => {
                      setSelected({ kind: view === "issues" ? "issue" : "pull", number: item.number });
                      setIssueDetail(null);
                      setPullDetail(null);
                      setConfirmMerge(false);
                      setConfirmReply(false);
                    }}
                    className={cn(
                      "flex w-full flex-col gap-0.5 border-b border-border px-2 py-1.5 text-left outline-none hover:bg-surface-raised focus-visible:bg-surface-raised",
                      current && "bg-surface-raised",
                    )}
                  >
                    <span className="flex items-baseline gap-2">
                      <Mono className="shrink-0 text-faint">#{item.number}</Mono>
                      <span className="min-w-0 truncate text-[12px] text-foreground">{item.title}</span>
                    </span>
                    <span className="flex items-center gap-2">
                      {"head" in item ? (
                        <Mono className="truncate text-faint">
                          {item.head} → {item.base}
                          {item.draft ? " · draft" : ""}
                        </Mono>
                      ) : (
                        <Mono className="truncate text-faint">
                          @{item.author}
                          {item.labels.length > 0 ? ` · ${item.labels.join(", ")}` : ""}
                        </Mono>
                      )}
                      <Mono className="ml-auto shrink-0 text-faint">{item.state}</Mono>
                    </span>
                  </button>
                </li>
              );
            })
          )}
        </ul>
        <div className="min-w-0 flex-1 overflow-auto">
          {!selected ? (
            <StateBlock
              state="idle"
              title={view === "issues" ? "Pick an issue to read it" : "Pick a pull request to read it"}
            />
          ) : selected.kind === "issue" && !issueDetail ? (
            <StateBlock state="loading" title={`Reading #${selected.number}`} />
          ) : selected.kind === "pull" && !pullDetail ? (
            <StateBlock state="loading" title={`Reading #${selected.number}`} />
          ) : (
            <article className="flex flex-col gap-3 p-3">
              {selected.kind === "issue" && issueDetail ? (
                <>
                  <header className="flex flex-wrap items-baseline gap-2">
                    <h2 className="text-[14px] font-medium text-foreground">{issueDetail.issue.title}</h2>
                    <Mono className="text-faint">
                      #{issueDetail.issue.number} · @{issueDetail.issue.author} · {issueDetail.issue.state}
                    </Mono>
                    <button
                      type="button"
                      aria-label="Open on GitHub"
                      className="ml-auto text-faint outline-none hover:text-foreground"
                      onClick={() => openUrl(issueDetail.issue.url)}
                    >
                      <ExternalLinkIcon className="size-3.5" />
                    </button>
                  </header>
                  <pre className="whitespace-pre-wrap font-sans text-[12px] leading-relaxed text-foreground">
                    {issueDetail.issue.body || "No description."}
                  </pre>
                  {issueDetail.issue.state === "open" ? (
                    <div className="flex flex-wrap items-center gap-2 border-t border-border pt-2">
                      <MetaLabel>Work on it</MetaLabel>
                      <select
                        aria-label="Where the agent may write"
                        className="border border-border bg-transparent px-1 text-[11px]"
                        value={scope}
                        onChange={(event) => setScope(event.target.value as "ask" | "all")}
                      >
                        <option value="ask">Ask before every write</option>
                        <option value="all">Anywhere in the repository</option>
                      </select>
                      <button
                        type="button"
                        className="border border-border bg-primary px-2 py-0.5 text-[11px] font-medium text-primary-foreground outline-none disabled:bg-transparent disabled:text-faint"
                        disabled={busy}
                        onClick={() => void startTask(issueDetail.issue)}
                      >
                        Plan a task from #{issueDetail.issue.number}
                      </button>
                      <Mono className="text-faint">based on {baseBranch}</Mono>
                    </div>
                  ) : null}
                  <Conversation comments={issueDetail.comments} />
                </>
              ) : pullDetail ? (
                <>
                  <header className="flex flex-wrap items-baseline gap-2">
                    <GitPullRequestIcon className="size-3.5 text-faint" />
                    <h2 className="text-[14px] font-medium text-foreground">{pullDetail.pull.title}</h2>
                    <Mono className="text-faint">
                      #{pullDetail.pull.number} · {pullDetail.pull.head} → {pullDetail.pull.base} ·{" "}
                      {pullDetail.pull.state}
                      {pullDetail.pull.draft ? " · draft" : ""}
                    </Mono>
                    <button
                      type="button"
                      aria-label="Open on GitHub"
                      className="ml-auto text-faint outline-none hover:text-foreground"
                      onClick={() => openUrl(pullDetail.pull.url)}
                    >
                      <ExternalLinkIcon className="size-3.5" />
                    </button>
                  </header>
                  <pre className="whitespace-pre-wrap font-sans text-[12px] leading-relaxed text-foreground">
                    {pullDetail.pull.body || "No description."}
                  </pre>
                  <section aria-label="Checks" className="flex flex-col gap-0.5">
                    <MetaLabel>Checks</MetaLabel>
                    {pullDetail.checks.length === 0 ? (
                      <Mono className="text-faint">None reported for {pullDetail.pull.headSha.slice(0, 7)}</Mono>
                    ) : (
                      pullDetail.checks.map((check) => (
                        <button
                          key={check.providerId}
                          type="button"
                          className="flex items-center gap-2 text-left outline-none hover:underline"
                          onClick={() => openUrl(check.url)}
                        >
                          <StateLabel
                            state={
                              check.status !== "completed"
                                ? "streaming"
                                : (CHECK_STATE[check.conclusion ?? ""] ?? "idle")
                            }
                            detail={check.conclusion ?? check.status}
                          />
                          <span className="text-[12px] text-foreground">{check.name}</span>
                        </button>
                      ))
                    )}
                  </section>
                  {pullDetail.reviewComments.length > 0 ? (
                    <section aria-label="Review comments" className="flex flex-col gap-1.5">
                      <MetaLabel>On the code</MetaLabel>
                      {pullDetail.reviewComments.map((comment) => (
                        <div key={comment.providerId} className="border-l border-border pl-2">
                          <Mono className="text-faint">
                            @{comment.author} · {comment.path}
                            {comment.line !== null ? `:${comment.line}` : ""}
                          </Mono>
                          <p className="whitespace-pre-wrap text-[12px] text-foreground">{comment.body}</p>
                        </div>
                      ))}
                    </section>
                  ) : null}
                  {pullDetail.pull.state === "open" ? (
                    <section aria-label="Merge" className="flex flex-wrap items-center gap-2 border-t border-border pt-2">
                      <MetaLabel>Merge</MetaLabel>
                      {pullDetail.pull.mergeable === false ? (
                        <StateLabel state="blocked" detail="GitHub says it cannot merge cleanly" />
                      ) : null}
                      <select
                        aria-label="Merge method"
                        className="border border-border bg-transparent px-1 text-[11px]"
                        value={mergeMethod}
                        onChange={(event) => setMergeMethod(event.target.value as MergeMethod)}
                      >
                        <option value="squash">Squash and merge</option>
                        <option value="merge">Merge commit</option>
                        <option value="rebase">Rebase and merge</option>
                      </select>
                      {confirmMerge ? (
                        <span role="alertdialog" aria-label="Confirm merge" className="flex items-center gap-2">
                          <span className="text-[12px] text-foreground">
                            Merge #{pullDetail.pull.number} into {pullDetail.pull.base} on GitHub as @
                            {remote.account}, at {pullDetail.pull.headSha.slice(0, 7)}?
                          </span>
                          <button
                            type="button"
                            className="border border-border bg-primary px-2 py-0.5 text-[11px] text-primary-foreground outline-none"
                            disabled={busy}
                            onClick={() => void merge()}
                          >
                            Merge
                          </button>
                          <button
                            type="button"
                            className="border border-border px-2 py-0.5 text-[11px] outline-none"
                            onClick={() => setConfirmMerge(false)}
                          >
                            Not yet
                          </button>
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="border border-border px-2 py-0.5 text-[11px] text-foreground outline-none disabled:text-faint"
                          disabled={pullDetail.pull.draft || pullDetail.pull.mergeable === false}
                          onClick={() => setConfirmMerge(true)}
                        >
                          Merge…
                        </button>
                      )}
                    </section>
                  ) : null}
                  <Conversation comments={pullDetail.comments} />
                </>
              ) : null}
              <section aria-label="Reply" className="flex flex-col gap-1 border-t border-border pt-2">
                <label className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <MessageSquareIcon className="size-3" />
                  Comment as @{remote.account}
                </label>
                <textarea
                  aria-label="Comment"
                  className="min-h-16 resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] text-foreground outline-none"
                  onChange={(event) => setReply(event.target.value)}
                  value={reply}
                />
                {confirmReply ? (
                  <span role="alertdialog" aria-label="Confirm comment" className="flex items-center gap-2">
                    <span className="text-[12px] text-foreground">
                      Post this publicly on #{selected.number} as @{remote.account}?
                    </span>
                    <button
                      type="button"
                      className="border border-border bg-primary px-2 py-0.5 text-[11px] text-primary-foreground outline-none"
                      disabled={busy}
                      onClick={() => void postReply()}
                    >
                      Post
                    </button>
                    <button
                      type="button"
                      className="border border-border px-2 py-0.5 text-[11px] outline-none"
                      onClick={() => setConfirmReply(false)}
                    >
                      Edit
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    className="w-fit border border-border px-2 py-0.5 text-[11px] text-foreground outline-none disabled:text-faint"
                    disabled={!reply.trim()}
                    onClick={() => setConfirmReply(true)}
                  >
                    Comment…
                  </button>
                )}
              </section>
            </article>
          )}
        </div>
      </div>
    </div>
  );
}

function Conversation({ comments }: { comments: IssueComment[] }) {
  if (comments.length === 0) return null;
  return (
    <section aria-label="Conversation" className="flex flex-col gap-2">
      <MetaLabel>Conversation</MetaLabel>
      {comments.map((comment) => (
        <div key={comment.providerId} className="border-l border-border pl-2">
          <Mono className="text-faint">
            @{comment.author} · {new Date(comment.createdAt).toLocaleString()}
          </Mono>
          <p className="whitespace-pre-wrap text-[12px] text-foreground">{comment.body}</p>
        </div>
      ))}
    </section>
  );
}
