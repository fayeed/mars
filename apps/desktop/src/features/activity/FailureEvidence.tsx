import { ArrowRightIcon, WrenchIcon } from "lucide-react";

import { failureEvidence } from "@/features/activity/failure-evidence";
import { MetaLabel, Mono } from "@/shell/primitives";

export interface FailureEvidenceViewProps {
  text: string;
  onOpenFile?: (path: string, line: number | null) => void;
  /**
   * The one thing to do about this failure.
   *
   * Singular on purpose. A failure with a menu beside it asks the reader to
   * choose a strategy before they have finished reading the evidence; a failure
   * with one obvious next step lets them get on with it, and the composer is
   * still there for anything else.
   */
  primaryAction?: { label: string; onAct: () => void };
}

/**
 * What a failure actually says, expanded.
 *
 * The assertion and the two compared values are lifted out when the runner
 * named them, and every `file:line` becomes a link — but the output is always
 * shown in full underneath. Nothing recognised here replaces the evidence; it
 * only saves the reader from hunting for it.
 */
export function FailureEvidenceView({
  onOpenFile,
  primaryAction,
  text,
}: FailureEvidenceViewProps) {
  const evidence = failureEvidence(text);

  return (
    <div className="flex flex-col gap-2">
      {evidence.assertion ? (
        <div className="flex flex-col gap-1">
          <MetaLabel>Assertion</MetaLabel>
          <Mono className="break-all text-destructive">{evidence.assertion}</Mono>
        </div>
      ) : null}

      {evidence.compared ? (
        <div className="flex flex-col gap-1">
          <MetaLabel>Compared</MetaLabel>
          <div className="flex flex-col gap-px border border-border">
            <div className="flex items-baseline gap-2 px-1.5 py-1">
              <span className="w-16 shrink-0 text-[11px] text-faint">
                {evidence.compared.expectedLabel}
              </span>
              <Mono className="min-w-0 flex-1 break-all text-muted-foreground">
                {evidence.compared.expected}
              </Mono>
            </div>
            <div className="flex items-baseline gap-2 border-t border-border px-1.5 py-1">
              <span className="w-16 shrink-0 text-[11px] text-faint">
                {evidence.compared.actualLabel}
              </span>
              <Mono className="min-w-0 flex-1 break-all text-destructive">
                {evidence.compared.actual}
              </Mono>
            </div>
          </div>
        </div>
      ) : null}

      {evidence.references.length > 0 ? (
        <div className="flex flex-col gap-1">
          <MetaLabel>Where</MetaLabel>
          <ul className="flex flex-col gap-px">
            {evidence.references.map((reference) => (
              <li key={`${reference.path}:${reference.line}:${reference.column ?? ""}`}>
                <button
                  type="button"
                  disabled={!onOpenFile}
                  onClick={() => onOpenFile?.(reference.path, reference.line)}
                  className="group flex w-full items-baseline gap-1.5 text-left outline-none disabled:cursor-default"
                >
                  <ArrowRightIcon className="size-3 shrink-0 self-center text-faint opacity-0 group-enabled:group-hover:opacity-100 group-enabled:group-focus-visible:opacity-100" />
                  <Mono className="min-w-0 truncate text-muted-foreground group-enabled:group-hover:text-foreground group-enabled:group-focus-visible:text-foreground">
                    {reference.path}:{reference.line}
                    {reference.column === null ? "" : `:${reference.column}`}
                  </Mono>
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="flex flex-col gap-1">
        <MetaLabel>Output</MetaLabel>
        <pre className="max-h-72 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-muted-foreground">
          {text}
        </pre>
      </div>

      {primaryAction ? (
        <button
          type="button"
          onClick={primaryAction.onAct}
          className="flex items-center gap-1.5 self-start border border-border bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground outline-none"
        >
          <WrenchIcon className="size-3.5" />
          {primaryAction.label}
        </button>
      ) : null}
    </div>
  );
}
