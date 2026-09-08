import type { PlannedCheck, PullRequestPlan, TaskCommit } from "@mars/shared-types";
import { GitPullRequestIcon, UploadIcon, XIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface PublishPreviewProps {
  plan: PullRequestPlan | null;
  commits: TaskCommit[];
  checks: PlannedCheck[];
  state: ActivityState;
  error?: string | null;
  /** Re-plans against the edited title, body, draft flag and reviewers. */
  onRevise: (next: {
    title: string;
    body: string;
    draft: boolean;
    reviewers: string[];
  }) => void;
  /** Both approvals are given by the same deliberate act. */
  onPublish: (plan: PullRequestPlan) => void;
  busy?: boolean;
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-20 shrink-0 text-[11px] text-faint">{label}</span>
      <span className="min-w-0 flex-1">{value}</span>
    </div>
  );
}

/**
 * The exact push and the exact pull request, before either happens.
 *
 * Nothing here is a summary of an intention: the push command is the command
 * that will run, the commits are the commits that will arrive, and the title
 * and body are the ones that will be sent. Editing any of them re-plans, which
 * changes the plan's fingerprint — and the core refuses to apply a plan whose
 * fingerprint has moved, so this screen cannot be skipped or gone stale behind.
 */
export function PublishPreview({
  busy = false,
  checks,
  commits,
  error,
  onPublish,
  onRevise,
  plan,
  state,
}: PublishPreviewProps) {
  const [title, setTitle] = useState(plan?.title ?? "");
  const [body, setBody] = useState(plan?.body ?? "");
  const [draft, setDraft] = useState(plan?.draft ?? false);
  const [reviewers, setReviewers] = useState<string[]>(plan?.reviewers ?? []);
  const [newReviewer, setNewReviewer] = useState("");

  if (state === "loading" || !plan) {
    return (
      <StateBlock
        state={state === "failed" ? "failed" : "loading"}
        title={state === "failed" ? "The publish plan could not be built" : "Planning the push"}
        description={
          state === "failed"
            ? error ?? "Nothing was pushed and no pull request was opened."
            : "Reading the remote, the commits and the working tree."
        }
      />
    );
  }

  const edited =
    title !== plan.title ||
    body !== plan.body ||
    draft !== plan.draft ||
    reviewers.join(",") !== plan.reviewers.join(",");

  function addReviewer() {
    const login = newReviewer.trim().replace(/^@/, "");
    if (!login || reviewers.includes(login)) return;
    setReviewers((current) => [...current, login]);
    setNewReviewer("");
  }

  return (
    <article
      aria-label="Publish preview"
      data-slot="publish-preview"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · publish</MetaLabel>
        <StateLabel state="blocked" detail="nothing has been pushed" />
        <Mono className="ml-auto text-faint">{plan.fingerprint.slice(0, 12)}</Mono>
      </header>

      <section className="flex flex-col gap-1">
        <MetaLabel>The push</MetaLabel>
        <Row
          label="Remote"
          value={<Mono className="break-all text-muted-foreground">{plan.remoteUrl}</Mono>}
        />
        <Row label="From" value={<Mono className="text-foreground">{plan.head}</Mono>} />
        <Row label="Onto" value={<Mono className="text-foreground">{plan.base}</Mono>} />
        <Row label="At" value={<Mono className="text-faint">{plan.headSha.slice(0, 12)}</Mono>} />
        <Row
          label="Command"
          value={<Mono className="break-all text-muted-foreground">{plan.pushCommand}</Mono>}
        />
        {plan.clean ? null : (
          <p className="text-[11px] text-primary">
            The worktree has uncommitted changes. They are not part of this push.
          </p>
        )}
      </section>

      <section className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Commits</MetaLabel>
          <Mono className="text-faint">{plan.commitsAhead}</Mono>
        </div>
        {commits.length === 0 ? (
          <p className="text-[11px] text-faint">
            {plan.commitsAhead} commit(s) ahead; the list could not be read.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {commits.map((commit) => (
              <li key={commit.oid} className="flex items-baseline gap-2">
                <Mono className="shrink-0 text-faint">{commit.oid.slice(0, 7)}</Mono>
                <span className="min-w-0 flex-1 truncate text-[11px] text-foreground">
                  {commit.subject}
                </span>
                <Mono className="shrink-0 text-faint">{commit.author}</Mono>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-1.5">
        <MetaLabel>The pull request</MetaLabel>
        <input
          aria-label="Pull request title"
          className="w-full border border-border bg-transparent px-1.5 py-1 text-[12px] text-foreground outline-none focus-visible:border-border-strong"
          disabled={busy}
          onChange={(event) => setTitle(event.target.value)}
          placeholder="Title"
          value={title}
        />
        <textarea
          aria-label="Pull request body"
          className="min-h-24 w-full resize-y border border-border bg-transparent px-1.5 py-1 text-[12px] leading-relaxed text-foreground outline-none focus-visible:border-border-strong"
          disabled={busy}
          onChange={(event) => setBody(event.target.value)}
          placeholder="Body"
          value={body}
        />
        <label className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <input
            checked={draft}
            disabled={busy}
            onChange={(event) => setDraft(event.target.checked)}
            type="checkbox"
          />
          Open as a draft
        </label>
      </section>

      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Reviewers</MetaLabel>
          <Mono className="text-faint">{reviewers.length}</Mono>
        </div>
        {reviewers.length === 0 ? (
          <p className="text-[11px] text-faint">Nobody will be asked to review.</p>
        ) : (
          <ul className="flex flex-wrap gap-1">
            {reviewers.map((login) => (
              <li key={login} className="flex items-center gap-1 border border-border px-1.5 py-0.5">
                <Mono className="text-muted-foreground">{login}</Mono>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => setReviewers((current) => current.filter((one) => one !== login))}
                  className="text-faint outline-none hover:text-destructive focus-visible:text-destructive disabled:pointer-events-none"
                >
                  <XIcon className="size-3" />
                  <span className="sr-only">Remove {login}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex items-center gap-1">
          <input
            aria-label="Add a reviewer"
            className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none focus-visible:border-border-strong"
            disabled={busy}
            onChange={(event) => setNewReviewer(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              addReviewer();
            }}
            placeholder="github-login"
            value={newReviewer}
          />
          <button
            type="button"
            disabled={busy || newReviewer.trim().length === 0}
            onClick={addReviewer}
            className="border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            Add
          </button>
        </div>
      </section>

      <section className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Checks that will run</MetaLabel>
          <Mono className="text-faint">{checks.filter((check) => check.onPullRequest).length}</Mono>
        </div>
        {checks.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No workflow files were found in this worktree. What the forge runs is configured there.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {checks.map((check) => (
              <li key={check.path} className="flex items-baseline gap-2">
                <span
                  className={cn(
                    "min-w-0 flex-1 truncate text-[11px]",
                    check.onPullRequest ? "text-foreground" : "text-faint",
                  )}
                >
                  {check.name}
                </span>
                <Mono className="shrink-0 text-faint">
                  {check.onPullRequest ? "on pull_request" : "not on pull_request"}
                </Mono>
              </li>
            ))}
          </ul>
        )}
        <p className="text-[11px] text-faint">
          Read from the workflow files in this worktree. Nothing is scheduled until the pull request
          exists.
        </p>
      </section>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <footer className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 text-[11px] text-faint">
          {edited
            ? "Edited. The plan is re-checked against the remote before anything is pushed."
            : "Publishing pushes the branch and opens the pull request. Both happen because you asked."}
        </span>
        {edited ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => onRevise({ title, body, draft, reviewers })}
            className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
          >
            <GitPullRequestIcon className="size-3.5" />
            Re-check this plan
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={() => onPublish(plan)}
            className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
          >
            <UploadIcon className="size-3.5" />
            {busy ? "Publishing…" : "Push and open the pull request"}
          </button>
        )}
      </footer>
    </article>
  );
}
