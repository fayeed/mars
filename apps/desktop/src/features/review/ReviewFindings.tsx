import type {
  CapabilityCeiling,
  FindingSeverity,
  FindingStatus,
  ReviewFinding,
  ReviewRecord,
} from "@mars/shared-types";
import { ArrowRightIcon, CheckIcon, WrenchIcon, XIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { MetaLabel, Mono, StateBlock, StateLabel } from "@/shell/primitives";

export interface ReviewFindingsProps {
  review: ReviewRecord | null;
  state: ActivityState;
  error?: string | null;
  /** What the reviewing agent was allowed to do while it read. */
  reviewerCeiling?: CapabilityCeiling | null;
  /** The agent that wrote the code being reviewed. */
  implementerLabel?: string;
  onOpenFinding: (path: string, line: number) => void;
  onDecide: (finding: ReviewFinding, status: FindingStatus) => void;
  onAskForFix: (finding: ReviewFinding) => void;
  busyFindingId?: string | null;
  /** Hands the change to a second agent to read. Absent when none can run. */
  onRunReview?: () => void;
  /** The agent that would read it, named before it is asked to. */
  reviewerLabel?: string;
  /**
   * The sha256 of the diff as it stands now.
   *
   * A finding is a claim about one exact diff: `parser.ts:81` means line 81 of
   * the diff the reviewer read, and stays perfectly readable after the edit
   * that made it point somewhere else. Comparing this against the review's own
   * digest is the only way the panel can say so. Absent while the diff is being
   * read, which reads as unchecked rather than as unchanged.
   */
  currentDiffSha?: string | null;
}

/**
 * Whether the change has moved since the reviewer read it.
 *
 * Deliberately three-valued. A review recorded before the digest was stored,
 * or a diff that has not been read yet, is `unknown` — the panel says nothing
 * rather than implying either answer.
 */
export function reviewDrift(
  review: Pick<ReviewRecord, "diffSha256">,
  currentDiffSha: string | null | undefined,
): "same" | "changed" | "unknown" {
  if (!review.diffSha256 || !currentDiffSha) return "unknown";
  return review.diffSha256 === currentDiffSha ? "same" : "changed";
}

/**
 * Severity as the shell says it. Only a blocker and an error earn the second
 * hue; a warning and a note are stated rather than coloured, so the two that
 * stop a change stay distinguishable at a glance.
 */
const SEVERITY_TONE: Record<FindingSeverity, string> = {
  blocker: "text-destructive",
  error: "text-destructive",
  warning: "text-foreground",
  info: "text-muted-foreground",
};

const SEVERITY_ORDER: FindingSeverity[] = ["blocker", "error", "warning", "info"];

const STATUS_LABEL: Record<FindingStatus, string> = {
  open: "open",
  sent: "sent to the implementer",
  resolved: "accepted",
  dismissed: "dismissed",
};

/** What a reviewer could do, stated as the ceiling it actually was. */
function ceilingSummary(ceiling: CapabilityCeiling): string {
  const held: string[] = ["read files"];
  if (ceiling.writeFiles) held.push("write files");
  if (ceiling.commands !== "none") held.push(`run ${ceiling.commands} commands`);
  if (ceiling.network) held.push("use the network");
  if (ceiling.gitPush) held.push("push");
  return held.join(", ");
}

function Finding({
  busy,
  finding,
  onAskForFix,
  onDecide,
  onOpenFinding,
}: {
  busy: boolean;
  finding: ReviewFinding;
  onAskForFix: (finding: ReviewFinding) => void;
  onDecide: (finding: ReviewFinding, status: FindingStatus) => void;
  onOpenFinding: (path: string, line: number) => void;
}) {
  const settled = finding.status === "resolved" || finding.status === "dismissed";
  return (
    <li
      className={cn(
        "flex flex-col gap-1.5 border-b border-border px-3 py-2",
        settled && "opacity-60",
      )}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <Mono className={cn("shrink-0", SEVERITY_TONE[finding.severity])}>{finding.severity}</Mono>
        <span className="min-w-0 flex-1 text-[12px] text-foreground">{finding.title}</span>
        <Mono className="shrink-0 text-faint">{STATUS_LABEL[finding.status]}</Mono>
      </div>

      <p className="text-[11px] leading-relaxed text-muted-foreground">{finding.body}</p>

      <button
        type="button"
        onClick={() => onOpenFinding(finding.relativePath, finding.line)}
        className="group flex items-baseline gap-1.5 self-start outline-none"
      >
        <ArrowRightIcon className="size-3 shrink-0 self-center text-faint opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
        <Mono className="text-muted-foreground group-hover:text-foreground group-focus-visible:text-foreground">
          {finding.relativePath}:{finding.line}
        </Mono>
      </button>

      {finding.evidence.length > 0 ? (
        <pre className="max-h-32 overflow-auto whitespace-pre-wrap border border-border px-1.5 py-1 font-mono text-[11px] leading-relaxed text-muted-foreground">
          {finding.evidence.join("\n")}
        </pre>
      ) : null}

      {settled ? null : (
        <div className="flex flex-wrap items-center gap-1">
          <button
            type="button"
            disabled={busy}
            onClick={() => onDecide(finding, "resolved")}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <CheckIcon className="size-3" />
            Accept
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => onDecide(finding, "dismissed")}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-muted-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <XIcon className="size-3" />
            Dismiss
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => onAskForFix(finding)}
            className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] text-foreground outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <WrenchIcon className="size-3" />
            Ask for a fix
          </button>
        </div>
      )}
    </li>
  );
}

/**
 * One agent read the change; another can fix it; the person decides.
 *
 * Who found a thing is part of the finding: a reviewer that could only read is
 * making a different kind of claim from one that could run the tests, and the
 * header says which this was. Nothing here resolves itself — accept, dismiss
 * and ask-for-a-fix are all a person's move.
 */
export function ReviewFindings({
  busyFindingId,
  currentDiffSha,
  error,
  implementerLabel,
  onAskForFix,
  onDecide,
  onOpenFinding,
  onRunReview,
  review,
  reviewerCeiling,
  reviewerLabel,
  state,
}: ReviewFindingsProps) {
  if (state === "loading") {
    return <StateBlock state="loading" title="Reading the change" />;
  }
  if (state === "failed") {
    return (
      <StateBlock
        state="failed"
        title="The review could not be run"
        description={error ?? "Nothing was changed."}
      />
    );
  }
  if (!review) {
    return (
      <StateBlock
        state="idle"
        title="No review yet"
        description="A second agent can read this change and report what it finds. It cannot alter anything while it reads."
        action={
          onRunReview ? (
            <button
              type="button"
              onClick={onRunReview}
              className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none"
            >
              <ArrowRightIcon className="size-3.5" />
              Have {reviewerLabel ?? "another agent"} read it
            </button>
          ) : null
        }
      />
    );
  }

  const findings = [...review.findings].sort(
    (a, b) => SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity),
  );
  const open = findings.filter((finding) => finding.status === "open");
  const drift = reviewDrift(review, currentDiffSha);

  return (
    <section aria-label="Review findings" className="flex flex-col">
      <div className="flex flex-col gap-1.5 border-b border-border px-3 py-2.5">
        <div className="flex flex-wrap items-center gap-2">
          <MetaLabel className="text-foreground">Cross-agent review</MetaLabel>
          <StateLabel
            state={review.verdict === "pass" ? "done" : "blocked"}
            detail={review.verdict === "pass" ? "no changes requested" : "changes requested"}
          />
          <Mono className="ml-auto text-faint">{review.handoffDigest.slice(0, 12)}</Mono>
        </div>

        {drift === "changed" ? (
          <StateLabel
            state="stale"
            detail="the change has moved since this was read — every line below points into the older diff"
          />
        ) : drift === "unknown" ? (
          <Mono className="text-faint">
            {review.diffSha256
              ? "the current change has not been read back, so these have not been re-checked"
              : "recorded before the reviewed diff was versioned; it cannot be re-checked"}
          </Mono>
        ) : null}

        <p className="text-[12px] leading-relaxed text-foreground">{review.summary}</p>

        <p className="text-[11px] text-muted-foreground">
          <span className="text-foreground">{review.reviewerAdapter}</span> read the change
          {implementerLabel ? (
            <>
              {" written by "}
              <span className="text-foreground">{implementerLabel}</span>
            </>
          ) : null}
          {reviewerCeiling ? (
            <>
              {" · it could "}
              <Mono className="text-faint">{ceilingSummary(reviewerCeiling)}</Mono>
            </>
          ) : null}
        </p>
      </div>

      {findings.length === 0 ? (
        <StateBlock
          className="h-auto py-6"
          state="done"
          title="Nothing to report"
          description="The reviewer read the change and raised no findings."
        />
      ) : (
        <>
          <div className="flex items-baseline gap-2 px-3 pt-2">
            <MetaLabel>Findings</MetaLabel>
            <Mono className="text-faint">{findings.length}</Mono>
            {open.length > 0 ? (
              <Mono className="text-primary">{open.length} still open</Mono>
            ) : null}
          </div>
          <ul className="flex flex-col">
            {findings.map((finding) => (
              <Finding
                busy={busyFindingId === finding.id}
                finding={finding}
                key={finding.id}
                onAskForFix={onAskForFix}
                onDecide={onDecide}
                onOpenFinding={onOpenFinding}
              />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
