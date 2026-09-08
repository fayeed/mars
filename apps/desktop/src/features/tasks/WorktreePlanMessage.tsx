import type {
  PreflightFinding,
  SetupCommandInput,
  WorktreeProposal,
} from "@mars/shared-types";
import { CheckIcon, PlusIcon, RotateCcwIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/utils";
import type { ActivityState } from "@/shell/activity-state";
import { KeyHint, MetaLabel, Mono, StateLabel } from "@/shell/primitives";
import { chordHint } from "@/shell/keymap";

export interface PlanEdits {
  expectedScope: string[];
  setupCommands: SetupCommandInput[];
  /** The fingerprint of the plan that was on screen when Approve was pressed. */
  fingerprint: string;
  /** False when nothing was edited, so the caller can approve without amending. */
  edited: boolean;
}

export interface WorktreePlanMessageProps {
  proposal: WorktreeProposal;
  /** `loading` while an amend or approval is in flight. */
  state?: ActivityState;
  error?: string | null;
  onApprove: (edits: PlanEdits) => void;
}

function shortOid(oid: string) {
  return oid.slice(0, 7);
}

function formatBytes(bytes: number) {
  const gib = bytes / 1024 ** 3;
  if (gib >= 1) return `${gib.toFixed(1)} GiB`;
  return `${Math.round(bytes / 1024 ** 2)} MiB`;
}

/**
 * How a classified setup command reads. Only `destructive` earns the second
 * hue; the rest are stated plainly, because "mutating" is what a build command
 * normally is and colouring it would cry wolf.
 */
function classificationTone(classification: string) {
  return classification === "destructive" ? "text-destructive" : "text-faint";
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-16 shrink-0 text-[11px] text-faint">{label}</span>
      <span className="min-w-0 flex-1">{value}</span>
    </div>
  );
}

function Finding({ finding }: { finding: PreflightFinding }) {
  const blocking = finding.severity === "blocking";
  return (
    <li
      className={cn(
        "flex flex-col gap-1 border-l-2 py-1 pl-2",
        blocking ? "border-destructive" : "border-primary",
      )}
    >
      <span className="flex items-center gap-2">
        <Mono className={blocking ? "text-destructive" : "text-primary"}>{finding.code}</Mono>
        <span className="text-[11px] text-faint">{finding.severity}</span>
      </span>
      <span className="text-[11px] leading-relaxed text-foreground">{finding.detail}</span>
      {finding.resolutions.length > 0 ? (
        <ul className="flex flex-col gap-0.5">
          {finding.resolutions.map((resolution) => (
            <li key={resolution.action} className="text-[11px] text-muted-foreground">
              <span className="text-foreground">{resolution.action}</span>
              {" — "}
              {resolution.consequence}
            </li>
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/**
 * The worktree plan, in the conversation, as a message.
 *
 * It is not a wizard and not a modal: it arrives in the transcript where the
 * request that produced it is, and it stays there afterwards as the record of
 * what was approved. Scope and setup commands are edited in place. Approval is
 * recorded against the plan's fingerprint, so editing invalidates it — which is
 * why one button both saves the edits and approves what they produced, and why
 * it says which of those it is about to do.
 */
export function WorktreePlanMessage({
  error,
  onApprove,
  proposal,
  state = "idle",
}: WorktreePlanMessageProps) {
  const [scope, setScope] = useState<string[]>(proposal.expectedScope);
  const [commands, setCommands] = useState<SetupCommandInput[]>(
    proposal.setupCommands.map(({ command, purpose }) => ({ command, purpose })),
  );
  const [newGlob, setNewGlob] = useState("");
  const [newCommand, setNewCommand] = useState("");

  const edited = useMemo(() => {
    const sameScope =
      scope.length === proposal.expectedScope.length &&
      scope.every((glob, index) => glob === proposal.expectedScope[index]);
    const sameCommands =
      commands.length === proposal.setupCommands.length &&
      commands.every(
        (command, index) =>
          command.command === proposal.setupCommands[index].command &&
          command.purpose === proposal.setupCommands[index].purpose,
      );
    return !(sameScope && sameCommands);
  }, [commands, proposal.expectedScope, proposal.setupCommands, scope]);

  const blocking = proposal.preflight.filter((finding) => finding.severity === "blocking");
  const warnings = proposal.preflight.filter((finding) => finding.severity === "warning");
  const busy = state === "loading";
  const approved = proposal.approvedAt !== null;
  // A blocking finding is about the plan as the core last assembled it. Edits
  // may well be the fix, so saving them stays available while approving does not.
  const canApprove = !busy && !approved && (edited || blocking.length === 0);

  function addGlob() {
    const glob = newGlob.trim();
    if (!glob || scope.includes(glob)) return;
    setScope((current) => [...current, glob]);
    setNewGlob("");
  }

  function addCommand() {
    const command = newCommand.trim();
    if (!command) return;
    setCommands((current) => [...current, { command, purpose: "added before approval" }]);
    setNewCommand("");
  }

  return (
    <article
      data-slot="worktree-plan"
      aria-label="Worktree plan"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · worktree plan</MetaLabel>
        <StateLabel state={approved ? "done" : blocking.length > 0 ? "blocked" : "queued"} />
        <Mono className="ml-auto text-faint">{proposal.fingerprint.slice(0, 12)}</Mono>
      </header>

      <p className="text-[13px] leading-relaxed text-foreground">{proposal.intent}</p>

      <div className="flex flex-col gap-1">
        <Row
          label="Base"
          value={
            <span className="flex items-baseline gap-2">
              <Mono className="text-muted-foreground">{proposal.baseRef}</Mono>
              <Mono className="text-faint">{shortOid(proposal.baseOid)}</Mono>
            </span>
          }
        />
        <Row label="Branch" value={<Mono className="text-muted-foreground">{proposal.branchName}</Mono>} />
        <Row
          label="Path"
          value={<Mono className="break-all text-faint">{proposal.worktreePath}</Mono>}
        />
        <Row
          label="Disk"
          value={
            <Mono className={proposal.disk.low ? "text-destructive" : "text-faint"}>
              {proposal.disk.availableBytes === null
                ? "free space could not be read"
                : `${formatBytes(proposal.disk.availableBytes)} free`}
            </Mono>
          }
        />
      </div>

      <section className="flex flex-col gap-1.5">
        <MetaLabel>Scope</MetaLabel>
        <p className="text-[11px] text-muted-foreground">
          Where the agent may write without asking. A write outside these reaches the permission
          broker instead of happening quietly.
        </p>
        {scope.length === 0 ? (
          <p className="text-[11px] text-faint">
            No scope declared — every write will stop and ask.
          </p>
        ) : (
          <ul className="flex flex-wrap gap-1">
            {scope.map((glob) => (
              <li key={glob} className="flex items-center gap-1 border border-border px-1.5 py-0.5">
                <Mono className="text-muted-foreground">{glob}</Mono>
                <button
                  type="button"
                  disabled={busy || approved}
                  onClick={() => setScope((current) => current.filter((item) => item !== glob))}
                  className="text-faint outline-none hover:text-destructive focus-visible:text-destructive disabled:pointer-events-none"
                >
                  <XIcon className="size-3" />
                  <span className="sr-only">Remove {glob}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {approved ? null : (
          <div className="flex items-center gap-1">
            <input
              aria-label="Add a scope glob"
              className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none focus-visible:border-border-strong"
              disabled={busy}
              onChange={(event) => setNewGlob(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                addGlob();
              }}
              placeholder="crates/ingest/**"
              value={newGlob}
            />
            <button
              type="button"
              disabled={busy || newGlob.trim().length === 0}
              onClick={addGlob}
              className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
            >
              <PlusIcon className="size-3" />
              Add
            </button>
          </div>
        )}
      </section>

      <section className="flex flex-col gap-1.5">
        <MetaLabel>Setup commands</MetaLabel>
        {commands.length === 0 ? (
          <p className="text-[11px] text-faint">None. The worktree is checked out and left alone.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {commands.map((command, index) => {
              // Classification comes from the core's classifier. A command added
              // here has not been through it yet, and says so rather than
              // borrowing a neighbour's rating.
              const classification = proposal.setupCommands.find(
                (candidate) => candidate.command === command.command,
              )?.classification;
              return (
                <li
                  key={`${command.command}:${index}`}
                  className="flex flex-col gap-0.5 border border-border px-1.5 py-1"
                >
                  <span className="flex items-center gap-2">
                    <Mono className="min-w-0 flex-1 break-all text-foreground">
                      {command.command}
                    </Mono>
                    <Mono className={classification ? classificationTone(classification) : "text-faint"}>
                      {classification ?? "not yet classified"}
                    </Mono>
                    <button
                      type="button"
                      disabled={busy || approved}
                      onClick={() =>
                        setCommands((current) => current.filter((_, position) => position !== index))
                      }
                      className="text-faint outline-none hover:text-destructive focus-visible:text-destructive disabled:pointer-events-none"
                    >
                      <XIcon className="size-3" />
                      <span className="sr-only">Remove {command.command}</span>
                    </button>
                  </span>
                  <span className="text-[11px] text-muted-foreground">{command.purpose}</span>
                </li>
              );
            })}
          </ul>
        )}
        {approved ? null : (
          <div className="flex items-center gap-1">
            <input
              aria-label="Add a setup command"
              className="min-w-0 flex-1 border border-border bg-transparent px-1.5 py-0.5 font-mono text-[11px] text-foreground outline-none focus-visible:border-border-strong"
              disabled={busy}
              onChange={(event) => setNewCommand(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                addCommand();
              }}
              placeholder="pnpm install --frozen-lockfile"
              value={newCommand}
            />
            <button
              type="button"
              disabled={busy || newCommand.trim().length === 0}
              onClick={addCommand}
              className="flex items-center gap-1 border border-border px-1.5 py-0.5 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
            >
              <PlusIcon className="size-3" />
              Add
            </button>
          </div>
        )}
      </section>

      {proposal.preflight.length > 0 ? (
        <section className="flex flex-col gap-1.5">
          <MetaLabel>Preflight</MetaLabel>
          <ul className="flex flex-col gap-2">
            {blocking.map((finding) => (
              <Finding finding={finding} key={`${finding.code}:${finding.detail}`} />
            ))}
            {warnings.map((finding) => (
              <Finding finding={finding} key={`${finding.code}:${finding.detail}`} />
            ))}
          </ul>
        </section>
      ) : null}

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <footer className="flex flex-wrap items-center gap-2">
        {approved ? (
          <span className="text-[11px] text-muted-foreground">
            Approved by <span className="text-foreground">{proposal.approvedBy}</span>{" "}
            <Mono className="text-faint">{proposal.approvedAt}</Mono>
          </span>
        ) : (
          <span className="min-w-0 flex-1 text-[11px] text-faint">
            {edited
              ? "Edited. Saving re-runs preflight against the change before anything is approved."
              : blocking.length > 0
                ? "Blocked. Resolve the finding above, or edit the plan so it no longer applies."
                : "Nothing has touched Git yet. Approving creates the branch and the worktree."}
          </span>
        )}

        {edited && !approved ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setScope(proposal.expectedScope);
              setCommands(
                proposal.setupCommands.map(({ command, purpose }) => ({ command, purpose })),
              );
            }}
            className="flex items-center gap-1 border border-border px-2 py-1 text-[11px] outline-none hover:border-border-strong disabled:cursor-not-allowed disabled:text-faint"
          >
            <RotateCcwIcon className="size-3" />
            Discard edits
          </button>
        ) : null}

        {approved ? null : (
          <button
            type="button"
            disabled={!canApprove}
            onClick={() =>
              onApprove({
                expectedScope: scope,
                setupCommands: commands,
                fingerprint: proposal.fingerprint,
                edited,
              })
            }
            className="flex items-center gap-1.5 border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none disabled:cursor-not-allowed disabled:border-border disabled:bg-transparent disabled:text-faint"
          >
            <CheckIcon className="size-3.5" />
            {busy ? "Working…" : edited ? "Save and approve" : "Approve"}
            <KeyHint
              className="border-transparent text-current opacity-70"
              keys={chordHint({ key: "Enter", mod: true, shift: true })}
            />
          </button>
        )}
      </footer>
    </article>
  );
}
