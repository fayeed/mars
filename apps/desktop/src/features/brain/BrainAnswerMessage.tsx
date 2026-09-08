import type {
  BrainAnswer as BrainAnswerValue,
  Citation,
  Claim,
  Gap,
} from "@mars/shared-types";
import { LightbulbIcon, QuoteIcon, TriangleAlertIcon } from "lucide-react";

import { cn } from "@/lib/utils";
import { formatDuration } from "@/features/activity/ActivityLog";
import type { ActivityState } from "@/shell/activity-state";
import {
  DerivedFrom,
  DerivedFromSummary,
  MetaLabel,
  Mono,
  StateBlock,
  StateLabel,
} from "@/shell/primitives";
import { anchorVerdict, type BlobAnchor, type WorkingTreeShas } from "@/shell/staleness";

export interface BrainAnswerMessageProps {
  question: string;
  answer: BrainAnswerValue | null;
  /** Claims already on the stream, rendered before the call resolves. */
  streamed?: Claim[];
  streamedCitations?: Citation[];
  state: ActivityState;
  /** What the pipeline is doing, while it is doing it. */
  stage?: string | null;
  error?: string | null;
  /**
   * The working tree's shas for the cited paths.
   *
   * An answer is a photograph of the repository at the moment it was asked.
   * The conversation keeps it forever, and the next commit can make every
   * line number in it wrong. Comparing each citation's blob sha against this
   * is what lets the answer admit that instead of ageing quietly.
   */
  shas?: WorkingTreeShas;
  onOpenCitation: (path: string, line: number) => void;
}

const NO_SHAS: WorkingTreeShas = new Map();

/** Why an answer fell short, in words rather than an enum name. */
const GAP_LABEL: Record<Gap["kind"], string> = {
  notIndexed: "not indexed",
  ambiguous: "ambiguous",
  requiresRuntime: "needs a running program",
  stale: "evidence changed",
  budgetExhausted: "context budget",
  noReasoning: "no reasoning",
};

/**
 * One citation, as a thing you can click and a version you can check.
 *
 * The line number is the part that rots. `parser.ts:81` stays perfectly
 * readable after the edit that pushed line 81 down by forty lines, so the chip
 * carries the blob sha the span was read from and marks itself when the working
 * tree has moved past it. A stale chip still opens the file — the reader is
 * told what they are about to see, not stopped from seeing it.
 */
function CitationChip({
  citation,
  index,
  onOpen,
  shas,
}: {
  citation: Citation;
  index: number;
  onOpen: () => void;
  shas: WorkingTreeShas;
}) {
  const anchor: BlobAnchor = { path: citation.path, sha: citation.blobSha };
  const verdict = anchorVerdict(anchor, shas);
  return (
    <button
      type="button"
      onClick={onOpen}
      title={citation.why}
      data-verdict={verdict}
      className={cn(
        "inline-flex items-baseline gap-1 border px-1 py-px align-baseline outline-none",
        "hover:border-border-strong focus-visible:border-border-strong",
        verdict === "fresh" || verdict === "unverified"
          ? "border-border"
          : "border-dashed border-border-strong",
      )}
    >
      <Mono className="text-faint">{index}</Mono>
      <Mono className="max-w-40 truncate text-muted-foreground">
        {citation.path.split("/").at(-1)}:{citation.lineStart}
      </Mono>
      <DerivedFrom anchor={anchor} shas={shas} />
    </button>
  );
}

/**
 * One claim.
 *
 * A fact and an inference are drawn differently on purpose, and the difference
 * is structural rather than decorative: a fact is what a cited span of source
 * says, and carries its citations inline; an inference is a conclusion drawn
 * from facts, is marked as one, and names the claims it reasons over. A reader
 * must never have to guess which of the two they are looking at.
 */
function ClaimRow({
  citations,
  claim,
  index,
  onOpenCitation,
  settled,
  shas,
}: {
  citations: Citation[];
  claim: Claim;
  index: Map<string, number>;
  onOpenCitation: (path: string, line: number) => void;
  /** The answer has resolved, so what is missing is missing rather than late. */
  settled: boolean;
  shas: WorkingTreeShas;
}) {
  const cited = claim.citations
    .map((id) => citations.find((candidate) => candidate.id === id))
    .filter((candidate): candidate is Citation => Boolean(candidate));
  // Named by the claim but not on the stream yet.
  const pending = claim.citations.length - cited.length;

  return (
    <li
      className={cn(
        "flex flex-col gap-1 border-l-2 py-1 pl-2",
        claim.kind === "fact" ? "border-border-strong" : "border-primary",
      )}
    >
      <div className="flex items-baseline gap-1.5">
        {claim.kind === "fact" ? (
          <QuoteIcon className="size-3 shrink-0 self-center text-faint" />
        ) : (
          <LightbulbIcon className="size-3 shrink-0 self-center text-primary" />
        )}
        <span className="text-[13px] leading-relaxed text-foreground">{claim.text}</span>
      </div>

      {claim.kind === "fact" ? (
        cited.length > 0 ? (
          <div className="flex flex-wrap items-baseline gap-1">
            {cited.map((citation) => (
              <CitationChip
                citation={citation}
                index={index.get(citation.id) ?? 0}
                key={citation.id}
                onOpen={() => onOpenCitation(citation.path, citation.lineStart)}
                shas={shas}
              />
            ))}
            {pending > 0 ? (
              <Mono className="text-faint">{pending} more arriving</Mono>
            ) : null}
          </div>
        ) : settled ? (
          // The model says a fact always carries a citation. If one arrives
          // without, saying so is better than rendering it as if it had one.
          <span className="text-[11px] text-destructive">
            stated as a fact but arrived with no citation
          </span>
        ) : (
          // Mid-stream a claim can outrun its citations. Accusing it of being
          // uncited before the stream has finished would be a lie that clears
          // itself up a moment later — and readers remember the accusation.
          <span className="text-[11px] text-faint">waiting for its citations</span>
        )
      ) : (
        <span className="text-[11px] text-faint">
          inference
          {claim.supports.length > 0
            ? ` from ${claim.supports.length} claim(s) above`
            : " — not drawn from any listed claim"}
        </span>
      )}
    </li>
  );
}

/**
 * A Project Brain answer, in the conversation where it was asked.
 *
 * Everything that makes the answer checkable travels with it: what is a fact
 * and what is a conclusion, where each fact came from, how long it took, how
 * much it read, and — the part that matters most — what it could not find out.
 */
export function BrainAnswerMessage({
  answer,
  error,
  onOpenCitation,
  question,
  shas = NO_SHAS,
  stage,
  state,
  streamed = [],
  streamedCitations = [],
}: BrainAnswerMessageProps) {
  // While the call is in flight the stream is all there is; once it resolves,
  // the returned answer is authoritative and replaces it — the core prunes
  // citations that failed validation, and the stream cannot be trusted to have
  // dropped the same ones.
  const claims = answer?.claims ?? streamed;
  const citations = answer?.citations ?? streamedCitations;
  const settled = state !== "loading" && state !== "streaming";
  const index = new Map(citations.map((citation, position) => [citation.id, position + 1]));
  const anchors: BlobAnchor[] = citations.map((citation) => ({
    path: citation.path,
    sha: citation.blobSha,
  }));

  const facts = claims.filter((claim) => claim.kind === "fact");
  const inferences = claims.filter((claim) => claim.kind === "inference");

  return (
    <article
      aria-label="Project Brain answer"
      data-slot="brain-answer"
      className="flex flex-col gap-3 border-b border-border px-3 py-3"
    >
      <header className="flex flex-wrap items-center gap-2">
        <MetaLabel className="text-foreground">Project Brain</MetaLabel>
        {state === "loading" || state === "streaming" ? (
          <StateLabel
            state={claims.length > 0 ? "streaming" : "loading"}
            detail={stage ?? (claims.length > 0 ? `${claims.length} claims so far` : "thinking")}
          />
        ) : state === "failed" ? (
          <StateLabel state="failed" detail="the question was not answered" />
        ) : answer ? (
          <StateLabel
            state={answer.truncated ? "stale" : "done"}
            detail={`${formatDuration(answer.latencyMs) ?? ""} · ${answer.sources} sources`}
          />
        ) : null}
        {answer && answer.retrieved > answer.sources ? (
          <Mono className="text-faint">
            {answer.retrieved} retrieved, {answer.sources} packed
          </Mono>
        ) : null}
        {/* The answer as a whole, once: a reader scanning a long conversation
            should be able to see that a turn has aged without opening it. */}
        <DerivedFromSummary anchors={anchors} shas={shas} />
      </header>

      <p className="text-[12px] leading-relaxed text-muted-foreground">{question}</p>

      {error ? (
        <p className="border border-destructive/40 px-2 py-1 text-[11px] text-destructive">
          <Mono>{error}</Mono>
        </p>
      ) : null}

      {claims.length === 0 && settled && !error ? (
        <StateBlock
          className="h-auto py-4"
          state="idle"
          title="No answer"
          description="The brain read what it could and drew nothing it was willing to state."
        />
      ) : null}

      {facts.length > 0 ? (
        <section className="flex flex-col gap-1.5">
          <div className="flex items-baseline gap-2">
            <MetaLabel>Facts</MetaLabel>
            <Mono className="text-faint">{facts.length}</Mono>
            <span className="text-[11px] text-faint">what the cited source says</span>
          </div>
          <ul className="flex flex-col gap-1.5">
            {facts.map((claim) => (
              <ClaimRow
                citations={citations}
                claim={claim}
                index={index}
                key={claim.id}
                onOpenCitation={onOpenCitation}
                settled={settled}
                shas={shas}
              />
            ))}
          </ul>
        </section>
      ) : null}

      {inferences.length > 0 ? (
        <section className="flex flex-col gap-1.5">
          <div className="flex items-baseline gap-2">
            <MetaLabel>Inferences</MetaLabel>
            <Mono className="text-faint">{inferences.length}</Mono>
            <span className="text-[11px] text-faint">concluded, not read</span>
          </div>
          <ul className="flex flex-col gap-1.5">
            {inferences.map((claim) => (
              <ClaimRow
                citations={citations}
                claim={claim}
                index={index}
                key={claim.id}
                onOpenCitation={onOpenCitation}
                settled={settled}
                shas={shas}
              />
            ))}
          </ul>
        </section>
      ) : null}

      {answer ? (
        <section className="flex flex-col gap-1.5">
          <div className="flex items-baseline gap-2">
            <MetaLabel>Gaps</MetaLabel>
            <Mono className="text-faint">{answer.gaps.length}</Mono>
          </div>
          {answer.gaps.length === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              Nothing was reported as missing. That is not a promise the answer is complete.
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {answer.gaps.map((gap) => (
                <li
                  className="flex items-baseline gap-1.5"
                  key={`${gap.kind}:${gap.subject}:${gap.detail}`}
                >
                  <TriangleAlertIcon className="size-3 shrink-0 self-center text-primary" />
                  <Mono className="shrink-0 text-faint">{GAP_LABEL[gap.kind]}</Mono>
                  <span className="min-w-0 flex-1 text-[11px] leading-relaxed text-muted-foreground">
                    <span className="text-foreground">{gap.subject}</span> — {gap.detail}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {answer.truncated ? (
            <p className="text-[11px] text-muted-foreground">
              The context budget was reached, so this answer was drawn from part of what was found.
            </p>
          ) : null}
        </section>
      ) : null}
    </article>
  );
}
