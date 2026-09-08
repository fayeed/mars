import type {
  CommandClass,
  PendingPermissionRequest,
  PermissionDecisionChoice,
  SideEffect,
} from "@mars/shared-types";
import { AlertTriangleIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { MetaLabel, Mono, StateLabel } from "@/shell/primitives";

export interface PermissionRequestMessageProps {
  request: PendingPermissionRequest;
  onDecide: (choice: PermissionDecisionChoice) => void;
  busy?: boolean;
  error?: string | null;
}

/**
 * How consequential each class is. Only `destructive` earns the second hue —
 * a mutating command is what an ordinary build does, and colouring it would
 * spend the alarm on the common case.
 */
const CLASS_TONE: Record<CommandClass, string> = {
  read_only: "text-muted-foreground",
  mutating: "text-foreground",
  destructive: "text-destructive",
  network: "text-foreground",
};

/** The effect, rendered as the thing it actually is rather than a summary. */
function EffectDetail({ effect }: { effect: SideEffect }) {
  if (effect.kind === "shell") {
    return (
      <>
        <Row label="Command" value={<Mono className="break-all text-foreground">{effect.command}</Mono>} />
        <Row label="In" value={<Mono className="break-all text-faint">{effect.cwd}</Mono>} />
      </>
    );
  }
  if (effect.kind === "fileWrite") {
    return (
      <>
        <Row label="Write" value={<Mono className="break-all text-foreground">{effect.path}</Mono>} />
        <Row
          label="Declared"
          value={
            effect.declaredScope.length === 0 ? (
              <span className="text-[11px] text-faint">no scope was declared</span>
            ) : (
              <Mono className="break-all text-faint">{effect.declaredScope.join(" ")}</Mono>
            )
          }
        />
      </>
    );
  }
  if (effect.kind === "network") {
    return (
      <>
        <Row label="Host" value={<Mono className="text-foreground">{effect.host}</Mono>} />
        <Row label="Detail" value={<span className="text-[11px] text-muted-foreground">{effect.detail}</span>} />
      </>
    );
  }
  return (
    <>
      <Row label="Push" value={<Mono className="text-foreground">{effect.branch}</Mono>} />
      <Row label="To" value={<Mono className="text-foreground">{effect.remote}</Mono>} />
      <Row label="From" value={<Mono className="break-all text-faint">{effect.cwd}</Mono>} />
    </>
  );
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-16 shrink-0 text-[11px] text-faint">{label}</span>
      <span className="min-w-0 flex-1">{value}</span>
    </div>
  );
}

/**
 * The agent is stopped, and the question is part of the transcript.
 *
 * Not a modal: the editor, the diff, the terminal and the composer all stay
 * usable while this waits, because answering it well often means going and
 * looking at something first. The four decisions are the broker's own, and each
 * says exactly what it would grant — an `allow_pattern` that does not show its
 * pattern is a blank cheque.
 */
export function PermissionRequestMessage({
  busy = false,
  error,
  onDecide,
  request,
}: PermissionRequestMessageProps) {
  const destructive = request.classification === "destructive";

  const decisions: {
    choice: PermissionDecisionChoice;
    label: string;
    grants: string;
    primary?: boolean;
  }[] = [
    { choice: "allow_once", label: "Allow once", grants: "This one effect, this one time." },
    {
      choice: "allow_task",
      label: "Allow for this task",
      grants: "Every matching effect until this task ends.",
    },
    {
      choice: "allow_pattern",
      label: "Allow matching",
      grants: `Anything matching ${request.suggestedPattern}, until revoked in Settings.`,
    },
    { choice: "deny", label: "Deny", grants: "The agent is told no and carries on without it." },
  ];

  return (
    <article
      aria-label="Permission request"
      data-slot="permission-request"
      className={cn(
        "flex flex-col gap-3 border-b border-border px-3 py-3",
        destructive && "border-l-2 border-l-destructive",
      )}
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Mars · permission</MetaLabel>
        <StateLabel state="blocked" detail="the agent is stopped" />
        <span className={cn("ml-auto flex items-center gap-1", CLASS_TONE[request.classification])}>
          {destructive ? <AlertTriangleIcon className="size-3.5" /> : null}
          <Mono className="text-current">{request.classification}</Mono>
        </span>
      </header>

      <p className="text-[13px] leading-relaxed text-foreground">{request.summary}</p>

      <div className="flex flex-col gap-1">
        <EffectDetail effect={request.effect} />
      </div>

      <div className="flex flex-col gap-1">
        <MetaLabel>Why it is classified this way</MetaLabel>
        <p className="text-[11px] leading-relaxed text-muted-foreground">{request.reason}</p>
      </div>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      <div className="flex flex-col gap-1">
        <MetaLabel>Your answer</MetaLabel>
        <ul className="flex flex-col gap-1">
          {decisions.map((decision) => (
            <li key={decision.choice}>
              <button
                type="button"
                disabled={busy}
                onClick={() => onDecide(decision.choice)}
                className={cn(
                  "flex w-full flex-col gap-0.5 border px-2 py-1.5 text-left outline-none",
                  "hover:border-border-strong focus-visible:border-border-strong",
                  "disabled:cursor-not-allowed disabled:text-faint",
                  decision.choice === "deny" ? "border-border" : "border-border",
                )}
              >
                <span
                  className={cn(
                    "text-[12px]",
                    decision.choice === "deny" ? "text-destructive" : "text-foreground",
                  )}
                >
                  {decision.label}
                </span>
                <span className="text-[11px] text-muted-foreground">{decision.grants}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>

      <p className="text-[11px] text-faint">
        Nothing else is blocked. Open the file, read the diff, or run something in the terminal
        before you answer.
      </p>
    </article>
  );
}
