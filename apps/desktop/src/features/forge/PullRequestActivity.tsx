import type { PullRequestSnapshot, ReviewComment } from "@mars/shared-types";
import { ExternalLinkIcon, RefreshCwIcon, WrenchIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateDot, StateLabel } from "@/shell/primitives";

export interface PullRequestActivityProps {
  snapshot: PullRequestSnapshot;
  state: ActivityState;
  error?: string | null;
  onRefresh: () => void;
  onOpenUrl: (url: string) => void;
  onOpenFile: (path: string, line: number | null) => void;
  /** Asks the agent to answer one review comment, in the thread it belongs to. */
  onReplyWithFix: (comment: ReviewComment) => void;
  busy?: boolean;
}

function checkState(status: string, conclusion: string | null): ActivityState {
  if (status !== "completed") return "streaming";
  if (conclusion === "success") return "done";
  if (conclusion === null) return "idle";
  return "failed";
}

/**
 * GitHub, back in the conversation.
 *
 * A review comment is a message from a person about this change, so it is drawn
 * as one rather than as a row in a table — and the reply an agent makes is
 * attached to the comment it answers, because a fix that is not visibly tied to
 * the remark that prompted it leaves the reader to guess what was addressed.
 * Check runs sit above them because they are the fastest thing to change and
 * the first thing anyone looks at.
 */
export function PullRequestActivity({
  busy = false,
  error,
  onOpenFile,
  onOpenUrl,
  onRefresh,
  onReplyWithFix,
  snapshot,
  state,
}: PullRequestActivityProps) {
  const failing = snapshot.checks.filter(
    (check) => check.status === "completed" && check.conclusion !== "success",
  );

  return (
    <article
      aria-label="Pull request"
      data-slot="pull-request"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">GitHub · #{snapshot.number}</MetaLabel>
        <StateLabel
          state={snapshot.state === "open" ? "streaming" : "done"}
          detail={snapshot.draft ? `${snapshot.state} · draft` : snapshot.state}
        />
        <button
          type="button"
          disabled={busy}
          onClick={onRefresh}
          className="ml-auto flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          <RefreshCwIcon className={cn("size-3", state === "loading" && "shell-pulse")} />
          Refresh
        </button>
        <button
          type="button"
          onClick={() => onOpenUrl(snapshot.url)}
          className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong"
        >
          <ExternalLinkIcon className="size-3" />
          Open
        </button>
      </header>

      <div className="flex flex-col gap-0.5">
        <span className="text-[13px] leading-relaxed text-foreground">{snapshot.title}</span>
        <Mono className="text-faint">
          {snapshot.head} → {snapshot.base} · {snapshot.headSha.slice(0, 7)}
        </Mono>
        {snapshot.mergeable === false ? (
          <span className="text-[11px] text-destructive">
            GitHub reports this cannot merge as it stands.
          </span>
        ) : null}
      </div>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <section className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Checks</MetaLabel>
          <Mono className="text-faint">{snapshot.checks.length}</Mono>
          {failing.length > 0 ? (
            <Mono className="text-destructive">{failing.length} failing</Mono>
          ) : null}
        </div>
        {snapshot.checks.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No check has reported yet. This is what the forge has scheduled, refreshed when you ask.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {snapshot.checks.map((check) => (
              <li key={check.providerId} className="flex items-baseline gap-2">
                <StateDot
                  className="self-center"
                  state={checkState(check.status, check.conclusion)}
                />
                <span className="min-w-0 flex-1 truncate text-[11px] text-foreground">
                  {check.name}
                </span>
                <Mono className="shrink-0 text-faint">{check.conclusion ?? check.status}</Mono>
                <button
                  type="button"
                  onClick={() => onOpenUrl(check.url)}
                  className="shrink-0 text-faint outline-none hover:text-foreground focus-visible:text-foreground"
                >
                  <ExternalLinkIcon className="size-3" />
                  <span className="sr-only">Logs for {check.name}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Review comments</MetaLabel>
          <Mono className="text-faint">{snapshot.comments.length}</Mono>
        </div>
        {snapshot.comments.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">Nobody has commented yet.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {snapshot.comments.map((comment) => (
              <li
                key={comment.providerId}
                className="flex flex-col gap-1 border-l-2 border-border pl-2"
              >
                <div className="flex flex-wrap items-baseline gap-2">
                  <span className="text-[12px] text-foreground">{comment.author}</span>
                  <Mono className="text-faint">{comment.createdAt}</Mono>
                  {comment.path ? (
                    <button
                      type="button"
                      onClick={() => onOpenFile(comment.path as string, comment.line)}
                      className="outline-none"
                    >
                      <Mono className="text-faint hover:text-foreground">
                        {comment.path}
                        {comment.line === null ? "" : `:${comment.line}`}
                      </Mono>
                    </button>
                  ) : null}
                </div>
                <p className="text-[12px] leading-relaxed whitespace-pre-wrap text-muted-foreground">
                  {comment.body}
                </p>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onReplyWithFix(comment)}
                  className="flex items-center gap-1 self-start border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
                >
                  <WrenchIcon className="size-3" />
                  Answer this with a fix
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </article>
  );
}
