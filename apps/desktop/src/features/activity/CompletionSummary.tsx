import type { KnowledgeItem, MemoryProposal, TaskProjection } from "@mars/shared-types";
import { GitPullRequestIcon, PlayIcon, SplitIcon } from "lucide-react";

import { formatDuration } from "@/features/activity/ActivityLog";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface CompletionSummaryProps {
  projection: TaskProjection;
  branchName?: string | null;
  /** Memory the agent proposed. None of it is in effect until approved. */
  proposals?: MemoryProposal[];
  /** Decisions recorded in this task that are still waiting on a person. */
  decisions?: KnowledgeItem[];
  onReviewDecisions?: () => void;
  onReviewDiff: () => void;
  onRunChecks: () => void;
  onPublish: () => void;
  onReviewMemory?: () => void;
  busy?: boolean;
}

/**
 * A test the log recorded as passing. Anything else — failed, errored, or a
 * status this side does not recognise — is not evidence that something works.
 */
function passed(status: string) {
  return status === "passed" || status === "completed";
}

/**
 * What the run changed, what it proved, and what it did not.
 *
 * The third column is the one that earns this card: a summary that only lists
 * what went well is a worse summary than none, because it invites the reader to
 * assume the rest was checked. Everything here is folded from the log, so it
 * cannot drift from the transcript above it.
 */
export function CompletionSummary({
  branchName,
  busy = false,
  decisions = [],
  onPublish,
  onReviewDecisions,
  onReviewDiff,
  onReviewMemory,
  onRunChecks,
  projection,
  proposals = [],
}: CompletionSummaryProps) {
  const tests = projection.tests;
  const verified = tests.filter((test) => passed(test.status));
  const failed = tests.filter((test) => !passed(test.status));

  // A file edited after the last test ran was never covered by it, whatever
  // that test said.
  const lastTestSequence = tests.at(-1)?.sequence ?? -1;
  const editedAfterTests = projection.filesTouched.filter(
    (file) => file.lastSequence > lastTestSequence,
  );

  const unverified: string[] = [];
  if (tests.length === 0) {
    unverified.push("No test run was recorded for this work.");
  }
  if (editedAfterTests.length > 0) {
    unverified.push(
      `${editedAfterTests.length} file(s) changed after the last test run: ${editedAfterTests
        .map((file) => file.path)
        .slice(0, 3)
        .join(", ")}${editedAfterTests.length > 3 ? "…" : ""}`,
    );
  }
  if (failed.length > 0) {
    unverified.push(`${failed.length} test run(s) did not pass.`);
  }
  if (projection.errors.length > 0) {
    unverified.push(`${projection.errors.length} error(s) were recorded during the run.`);
  }

  const pendingProposals = proposals.filter((proposal) => proposal.status === "pending");

  return (
    <article
      aria-label="Completion summary"
      data-slot="completion-summary"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · summary</MetaLabel>
        <StateLabel state={failed.length > 0 ? "failed" : "done"} />
        {branchName ? <Mono className="ml-auto text-faint">{branchName}</Mono> : null}
      </header>

      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>What changed</MetaLabel>
          <Mono className="text-faint">{projection.filesTouched.length}</Mono>
        </div>
        {projection.filesTouched.length === 0 ? (
          <p className="text-[11px] text-faint">No files were changed.</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {projection.filesTouched.map((file) => (
              <li key={file.path} className="flex items-baseline gap-2">
                <Mono className="min-w-0 flex-1 truncate text-foreground">{file.path}</Mono>
                <Mono className="shrink-0 text-faint">{file.change}</Mono>
                <Mono className="w-16 shrink-0 text-right text-faint">
                  {file.edits} edit{file.edits === 1 ? "" : "s"}
                </Mono>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>What was verified</MetaLabel>
          <Mono className="text-faint">{verified.length}</Mono>
        </div>
        {verified.length === 0 ? (
          <p className="text-[11px] text-faint">Nothing was verified by a recorded test run.</p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {verified.map((test) => (
              <li key={test.sequence} className="flex items-baseline gap-2">
                <Mono className="min-w-0 flex-1 truncate text-muted-foreground">{test.command}</Mono>
                <Mono className="shrink-0 text-faint">{test.status}</Mono>
                <Mono className="w-12 shrink-0 text-right text-faint">
                  {formatDuration(test.durationMs) ?? ""}
                </Mono>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <MetaLabel>What was not</MetaLabel>
          <Mono className="text-faint">{unverified.length}</Mono>
        </div>
        {unverified.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            Every changed file was covered by a test run that passed after it changed.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {unverified.map((statement) => (
              <li key={statement} className="text-[11px] leading-relaxed text-muted-foreground">
                {statement}
              </li>
            ))}
          </ul>
        )}
      </section>

      {decisions.length > 0 ? (
        <section
          aria-label="Decisions awaiting your review"
          className="flex flex-col gap-1.5 border border-border px-2 py-1.5"
        >
          <div className="flex items-baseline gap-2">
            <MetaLabel>Decisions awaiting your review</MetaLabel>
            <Mono className="text-faint">{decisions.length}</Mono>
          </div>
          <ul className="flex flex-col gap-0.5">
            {decisions.slice(0, 3).map((item) => (
              <li key={item.record.id} className="flex items-baseline gap-2">
                <Mono className="shrink-0 text-faint">{item.record.kind}</Mono>
                <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {item.record.title}
                </span>
              </li>
            ))}
          </ul>
          <p className="text-[11px] text-faint">
            The agent chose these on its own. Approved, they are what later agents are told.
          </p>
          {onReviewDecisions ? (
            <button
              type="button"
              onClick={onReviewDecisions}
              className="self-start border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              Review decisions
            </button>
          ) : null}
        </section>
      ) : null}

      {pendingProposals.length > 0 ? (
        <section className="flex flex-col gap-1.5 border border-border px-2 py-1.5">
          <div className="flex items-baseline gap-2">
            <MetaLabel>Memory awaiting your approval</MetaLabel>
            <Mono className="text-faint">{pendingProposals.length}</Mono>
          </div>
          <ul className="flex flex-col gap-0.5">
            {pendingProposals.slice(0, 3).map((proposal) => (
              <li key={proposal.id} className="flex items-baseline gap-2">
                <Mono className="shrink-0 text-faint">{proposal.kind}</Mono>
                <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {proposal.text}
                </span>
              </li>
            ))}
          </ul>
          <p className="text-[11px] text-faint">
            None of it is in effect. Nothing reaches an agent's context until you approve it.
          </p>
          {onReviewMemory ? (
            <button
              type="button"
              onClick={onReviewMemory}
              className="self-start border border-border px-2 py-0.5 text-[11px] outline-none hover:border-border-strong"
            >
              Review memory
            </button>
          ) : null}
        </section>
      ) : null}

      <footer className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={onReviewDiff}
          className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
        >
          <SplitIcon className="size-3.5" />
          Review the diff
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onRunChecks}
          className="flex items-center gap-1.5 border border-border px-2 py-1 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          <PlayIcon className="size-3.5" />
          Run checks
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onPublish}
          className="flex items-center gap-1.5 border border-border px-2 py-1 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
        >
          <GitPullRequestIcon className="size-3.5" />
          Publish
        </button>
      </footer>
    </article>
  );
}
