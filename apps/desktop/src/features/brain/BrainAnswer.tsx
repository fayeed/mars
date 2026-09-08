import type {
  BrainAnswer as BrainAnswerValue,
  BrainStreamPayload,
  Citation,
  Claim,
  CommandFailure,
  Gap,
} from "@mars/shared-types";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangleIcon,
  ArrowUpRightIcon,
  LightbulbIcon,
  LoaderCircleIcon,
  QuoteIcon,
  SendIcon,
} from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

interface BrainAnswerProps {
  onOpenSource: (path: string, line: number) => void;
  repositoryId: string;
}

/** Human-readable labels for the reasons an answer can fall short. */
const GAP_LABELS: Record<Gap["kind"], string> = {
  notIndexed: "not indexed",
  ambiguous: "ambiguous",
  requiresRuntime: "needs a running program",
  stale: "evidence changed",
  budgetExhausted: "context budget",
  noReasoning: "no reasoning",
};

export function BrainAnswer({ onOpenSource, repositoryId }: BrainAnswerProps) {
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState<BrainAnswerValue | null>(null);
  // Claims arriving on the stream, rendered before the call resolves so a slow
  // answer builds in front of the reader instead of appearing all at once.
  const [streamed, setStreamed] = useState<Claim[]>([]);
  const [streamedCitations, setStreamedCitations] = useState<Citation[]>([]);
  const [stage, setStage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const streamId = useRef(0);

  useEffect(() => {
    const subscription = listen<BrainStreamPayload>("brain-event", ({ payload }) => {
      if (payload.streamId !== String(streamId.current)) return;
      const event = payload.event;
      switch (event.type) {
        case "planned":
          setStage(`rewrote the question into ${event.queries.length} queries`);
          break;
        case "retrieved":
          setStage(`retrieved ${event.candidates} candidates`);
          break;
        case "packed":
          setStage(`packed ${event.sources} sources into ${event.tokens} tokens`);
          break;
        case "synthesized":
          setStage(`checking ${event.claims} claims against the working tree`);
          break;
        case "citation":
          setStreamedCitations((current) => [...current, event]);
          break;
        case "claim":
          setStreamed((current) => [...current, event]);
          break;
        default:
          break;
      }
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

  async function ask(event: FormEvent) {
    event.preventDefault();
    if (!question.trim()) return;
    streamId.current += 1;
    setBusy(true);
    setError(null);
    setAnswer(null);
    setStreamed([]);
    setStreamedCitations([]);
    setStage(null);
    try {
      setAnswer(
        await invoke<BrainAnswerValue>("brain_ask", {
          repositoryId,
          question,
          streamId: String(streamId.current),
        }),
      );
    } catch (caught) {
      setError(messageFor(caught));
    } finally {
      setBusy(false);
      setStage(null);
    }
  }

  const claims = answer?.claims ?? streamed;
  const citations = answer?.citations ?? streamedCitations;
  const facts = claims.filter((claim) => claim.kind === "fact");
  const inferences = claims.filter((claim) => claim.kind === "inference");

  function citationFor(id: string) {
    return citations.find((citation) => citation.id === id);
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-card" aria-label="Project Brain answer">
      <form className="flex shrink-0 gap-2 border-b p-3" onSubmit={(event) => void ask(event)}>
        <Input
          aria-label="Ask Project Brain"
          onChange={(event) => setQuestion(event.target.value)}
          placeholder="Ask about this repository…"
          value={question}
        />
        <Button disabled={busy} size="icon-sm" type="submit" aria-label="Ask">
          {busy ? <LoaderCircleIcon className="animate-spin" /> : <SendIcon />}
        </Button>
      </form>

      {error ? <p className="p-3 text-xs text-destructive">{error}</p> : null}
      {busy && stage ? (
        <p className="px-3 py-2 text-[10px] uppercase tracking-wider text-muted-foreground" aria-live="polite">
          {stage}
        </p>
      ) : null}

      {claims.length > 0 || answer ? (
        <ScrollArea className="min-h-0 flex-1">
          <div className="grid gap-4 p-3">
            {answer ? (
              <header className="flex flex-wrap items-center gap-2" aria-label="Answer summary">
                {/* Reported verbatim: what the answer rested on, and how long it took. */}
                <span className="text-xs font-medium">
                  {(answer.latencyMs / 1000).toFixed(1)}s · {answer.sources} sources
                </span>
                <Badge variant="secondary">{Math.round(answer.confidence * 100)}% confidence</Badge>
                {answer.truncated ? <Badge variant="outline">budget truncated</Badge> : null}
              </header>
            ) : null}

            {facts.length > 0 ? (
              <div className="grid gap-2" aria-label="Facts">
                <h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  Facts · read directly from source
                </h3>
                {facts.map((claim) => (
                  <article className="grid gap-1 rounded-lg border bg-background p-2" key={claim.id}>
                    <p className="text-xs">
                      <Badge className="mr-2 align-middle" variant="secondary">
                        FACT
                      </Badge>
                      {claim.text}
                    </p>
                    <div className="flex flex-wrap gap-1">
                      {claim.citations.map((id) => {
                        const citation = citationFor(id);
                        if (!citation) return null;
                        return (
                          <Button
                            key={id}
                            onClick={() => onOpenSource(citation.path, citation.lineStart)}
                            size="xs"
                            title={citation.why}
                            type="button"
                            variant="ghost"
                          >
                            <QuoteIcon />
                            {citation.path}:{citation.lineStart}-{citation.lineEnd}
                            <ArrowUpRightIcon />
                          </Button>
                        );
                      })}
                    </div>
                  </article>
                ))}
              </div>
            ) : null}

            {/* Inferences are a separate block, never interleaved with facts:
                the data model keeps them apart and so must the reading order. */}
            {inferences.length > 0 ? (
              <div className="grid gap-2" aria-label="Inferences">
                <h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  Inferences · reasoning over those facts
                </h3>
                {inferences.map((claim) => (
                  <article
                    className="grid gap-1 rounded-lg border border-dashed bg-muted/20 p-2"
                    key={claim.id}
                  >
                    <p className="text-xs">
                      <Badge className="mr-2 align-middle" variant="outline">
                        INFERENCE
                      </Badge>
                      {claim.text}
                    </p>
                    <p className="text-[10px] text-muted-foreground">
                      <LightbulbIcon className="mr-1 inline size-3" />
                      rests on {claim.supports.length} fact(s) · {Math.round(claim.confidence * 100)}%
                    </p>
                  </article>
                ))}
              </div>
            ) : null}

            {answer && answer.gaps.length > 0 ? (
              <div className="grid gap-2" aria-label="Gaps">
                <h3 className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                  Could not determine
                </h3>
                {answer.gaps.map((gap, index) => (
                  <p
                    className="flex gap-2 text-xs text-amber-700 dark:text-amber-300"
                    key={`${gap.kind}:${index}`}
                  >
                    <AlertTriangleIcon className="mt-0.5 size-3.5 shrink-0" />
                    <span>
                      <strong className="font-medium">{GAP_LABELS[gap.kind]}</strong> — {gap.subject}
                      : {gap.detail}
                    </span>
                  </p>
                ))}
              </div>
            ) : null}
          </div>
        </ScrollArea>
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
