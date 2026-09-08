import type { ReviewFinding, TaskProjection } from "@mars/shared-types";
import { AlertTriangleIcon, ShieldCheckIcon } from "lucide-react";
import { useState } from "react";

import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface ApprovalGateProps {
  projection: TaskProjection;
  /** Findings not yet accepted or dismissed. */
  outstanding: ReviewFinding[];
  branchName?: string | null;
  /** Recorded as a message in the conversation, in the approver's own words. */
  onApprove: (note: string) => void;
  busy?: boolean;
  error?: string | null;
  approvedBy?: string | null;
  approvedAt?: string | null;
}

function passed(status: string) {
  return status === "passed" || status === "completed";
}

/**
 * The gate before anything leaves this machine.
 *
 * It is explicit and it is a person's: nothing here approves itself, times out,
 * or infers consent from the absence of an objection. The three things it puts
 * in front of the approver are the ones they would otherwise have to go and
 * assemble — what changed, whether it was checked, and what is still open —
 * and a blocker is stated as a blocker rather than being allowed to hide in a
 * list. The approval is recorded as a message, so who released this and what
 * they said about it is in the same record as everything else.
 */
export function ApprovalGate({
  approvedAt,
  approvedBy,
  branchName,
  busy = false,
  error,
  onApprove,
  outstanding,
  projection,
}: ApprovalGateProps) {
  const [note, setNote] = useState("");

  const verified = projection.tests.filter((test) => passed(test.status));
  const failed = projection.tests.filter((test) => !passed(test.status));
  const blockers = outstanding.filter(
    (finding) => finding.severity === "blocker" || finding.severity === "error",
  );
  const approved = Boolean(approvedAt);

  return (
    <article
      aria-label="Awaiting human review"
      data-slot="approval-gate"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · awaiting your review</MetaLabel>
        <StateLabel
          state={approved ? "done" : "blocked"}
          detail={approved ? "approved" : "nothing has left this machine"}
        />
        {branchName ? <Mono className="ml-auto text-faint">{branchName}</Mono> : null}
      </header>

      <section className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Change</MetaLabel>
          <Mono className="text-faint">{projection.filesTouched.length} file(s)</Mono>
        </div>
        <ul className="flex flex-col gap-0.5">
          {projection.filesTouched.slice(0, 6).map((file) => (
            <li key={file.path} className="flex items-baseline gap-2">
              <Mono className="min-w-0 flex-1 truncate text-foreground">{file.path}</Mono>
              <Mono className="shrink-0 text-faint">{file.change}</Mono>
            </li>
          ))}
          {projection.filesTouched.length > 6 ? (
            <li className="text-[11px] text-faint">
              and {projection.filesTouched.length - 6} more
            </li>
          ) : null}
        </ul>
      </section>

      <section className="flex flex-col gap-1">
        <MetaLabel>Verification</MetaLabel>
        {projection.tests.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            No test run was recorded. Approving this releases work nothing checked.
          </p>
        ) : (
          <p className="text-[11px] text-muted-foreground">
            <Mono className="text-foreground">{verified.length}</Mono> test run(s) passed
            {failed.length > 0 ? (
              <>
                {", "}
                <Mono className="text-destructive">{failed.length}</Mono> did not
              </>
            ) : null}
            .
          </p>
        )}
      </section>

      <section className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2">
          <MetaLabel>Outstanding findings</MetaLabel>
          <Mono className="text-faint">{outstanding.length}</Mono>
        </div>
        {outstanding.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">
            Every finding has been accepted or dismissed.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {outstanding.map((finding) => (
              <li key={finding.id} className="flex items-baseline gap-2">
                <Mono
                  className={
                    finding.severity === "blocker" || finding.severity === "error"
                      ? "shrink-0 text-destructive"
                      : "shrink-0 text-faint"
                  }
                >
                  {finding.severity}
                </Mono>
                <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {finding.title}
                </span>
                <Mono className="shrink-0 text-faint">
                  {finding.relativePath}:{finding.line}
                </Mono>
              </li>
            ))}
          </ul>
        )}
      </section>

      {blockers.length > 0 ? (
        <p className="flex items-baseline gap-1.5 border border-destructive/40 px-2 py-1">
          <AlertTriangleIcon className="size-3 shrink-0 self-center text-destructive" />
          <span className="text-[11px] leading-relaxed text-destructive">
            {blockers.length} finding(s) at error or blocker severity are still open. Approving does
            not resolve them; it releases the change with them open.
          </span>
        </p>
      ) : null}

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      {approved ? (
        <footer className="text-[11px] text-muted-foreground">
          Approved by <span className="text-foreground">{approvedBy}</span>{" "}
          <Mono className="text-faint">{approvedAt}</Mono>
        </footer>
      ) : (
        <footer className="flex flex-col gap-1.5">
          <MetaLabel>Approval</MetaLabel>
          <input
            aria-label="What you are approving, in your words"
            className="w-full border border-border bg-transparent px-1.5 py-1 text-[12px] text-foreground outline-none focus-visible:border-border-strong"
            disabled={busy}
            onChange={(event) => setNote(event.target.value)}
            placeholder="Optional: what you checked, and what you are accepting."
            value={note}
          />
          <div className="flex flex-wrap items-center gap-2">
            <span className="min-w-0 flex-1 text-[11px] text-faint">
              This is recorded as a message in this conversation, under your name.
            </span>
            <button
              type="button"
              disabled={busy}
              onClick={() => onApprove(note.trim())}
              className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
            >
              <ShieldCheckIcon className="size-3.5" />
              {busy ? "Recording…" : "Approve this change"}
            </button>
          </div>
        </footer>
      )}
    </article>
  );
}
