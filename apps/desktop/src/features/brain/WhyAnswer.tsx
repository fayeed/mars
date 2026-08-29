import type { CommandFailure, EntityRef, WhyAnswer as WhyAnswerValue } from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangleIcon, ArrowUpRightIcon, CircleHelpIcon, LoaderCircleIcon } from "lucide-react";
import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface WhyAnswerProps {
  onOpenCode: (path: string, line: number) => void;
  repositoryId: string;
  subject: EntityRef;
}

export function WhyAnswer({ onOpenCode, repositoryId, subject }: WhyAnswerProps) {
  const [answer, setAnswer] = useState<WhyAnswerValue | null>(null);
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function ask() {
    setBusy(true);
    setError(null);
    try {
      setAnswer(await invoke<WhyAnswerValue>("why_query", {
        repositoryId,
        subject,
        maxDepth: 5,
        maxPaths: 10,
      }));
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
    }
  }

  function openSource(location: string) {
    setSelectedSource(location);
    const match = /^diff:(.+):(\d+)$/.exec(location);
    if (match) onOpenCode(match[1], Number(match[2]));
  }

  return (
    <section className="grid gap-3 rounded-lg border bg-muted/15 p-3" aria-label="Why answer">
      <Button disabled={busy} onClick={() => void ask()} size="sm" type="button" variant="outline">
        {busy ? <LoaderCircleIcon className="animate-spin" /> : <CircleHelpIcon />}
        Ask why this exists
      </Button>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {answer ? (
        <div className="grid gap-3">
          <div className="flex items-center gap-2">
            <Badge variant={answer.status === "insufficient_evidence" ? "outline" : "secondary"}>
              {answer.status.replaceAll("_", " ")}
            </Badge>
            <span className="text-[10px] text-muted-foreground">facts and inference are shown separately</span>
          </div>
          {answer.facts.map((fact) => (
            <article className="grid gap-1" key={fact.knowledgeId}>
              <h4 className="text-xs font-medium">{fact.title}</h4>
              <p className="text-xs text-muted-foreground">{fact.body}</p>
              <span className="text-[10px] text-muted-foreground">FACT · {fact.provenance} · {Math.round(fact.confidence * 100)}%</span>
            </article>
          ))}
          {answer.inferences.map((inference) => <p className="text-xs" key={inference}>INFERENCE · {inference}</p>)}
          {answer.warnings.map((warning) => (
            <p className="flex gap-2 text-xs text-amber-700 dark:text-amber-300" key={warning}>
              <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" /> {warning}
            </p>
          ))}
          {answer.paths.map((path, index) => (
            <div className="flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground" key={`${path.relation}:${index}`}>
              {path.steps.map((step, stepIndex) => (
                <span key={`${step.location}:${stepIndex}`}>
                  {stepIndex > 0 ? " → " : ""}{step.kind}:{step.label}
                </span>
              ))}
            </div>
          ))}
          <div className="flex flex-wrap gap-2">
            {answer.sources.map((source) => (
              <Button key={source.location} onClick={() => openSource(source.location)} size="xs" type="button" variant="ghost">
                <ArrowUpRightIcon /> {source.location}
              </Button>
            ))}
          </div>
          {selectedSource ? (
            <div className="rounded-md border bg-background p-2 text-[10px]" aria-label="Opened evidence source">
              Opened source: <code>{selectedSource}</code>
            </div>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function messageFor(error: unknown) {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  return error instanceof Error ? error.message : String(error);
}
