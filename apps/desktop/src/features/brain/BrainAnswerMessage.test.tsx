import type { BrainAnswer, Citation, Claim } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BrainAnswerMessage } from "./BrainAnswerMessage";

const citation: Citation = {
  id: "c-1",
  path: "crates/ingest/src/lib.rs",
  lineStart: 80,
  lineEnd: 92,
  blobSha: "abc123",
  snippet: "self.limiter.check()?;",
  why: "matched the rate-limit query",
};

const fact: Claim = {
  id: "claim-1",
  kind: "fact",
  text: "Requests are limited per API token in Ingest::accept.",
  citations: ["c-1"],
  supports: [],
  confidence: 0.9,
};

const inference: Claim = {
  id: "claim-2",
  kind: "inference",
  text: "Two clients sharing a token therefore share one budget.",
  citations: [],
  supports: ["claim-1"],
  confidence: 0.6,
};

const answer: BrainAnswer = {
  question: "How is rate limiting keyed?",
  rewrittenQueries: [],
  claims: [fact, inference],
  citations: [citation],
  sources: 22,
  retrieved: 40,
  latencyMs: 1100,
  confidence: 0.8,
  gaps: [
    {
      kind: "requiresRuntime",
      subject: "the effective limit",
      detail: "the value is read from config at start-up, which cannot be read statically",
    },
  ],
  sourceDetail: [],
  truncated: false,
};

function renderAnswer(props: Partial<Parameters<typeof BrainAnswerMessage>[0]> = {}) {
  return render(
    <BrainAnswerMessage
      answer={answer}
      onOpenCitation={vi.fn()}
      question="How is rate limiting keyed?"
      state="idle"
      {...props}
    />,
  );
}

describe("BrainAnswerMessage", () => {
  it("separates what was read from what was concluded", () => {
    renderAnswer();

    const facts = screen.getByText("Facts").closest("section");
    const inferences = screen.getByText("Inferences").closest("section");

    expect(within(facts as HTMLElement).getByText(fact.text)).toBeVisible();
    expect(within(inferences as HTMLElement).getByText(inference.text)).toBeVisible();
    // The reader is told which is which, not left to infer it from styling.
    expect(screen.getByText("what the cited source says")).toBeVisible();
    expect(screen.getByText("concluded, not read")).toBeVisible();
  });

  it("says what an inference was drawn from", () => {
    renderAnswer();

    expect(screen.getByText("inference from 1 claim(s) above")).toBeVisible();
  });

  it("gives every fact a citation that opens the file at the cited lines", async () => {
    const user = userEvent.setup();
    const onOpenCitation = vi.fn();
    renderAnswer({ onOpenCitation });

    await user.click(screen.getByRole("button", { name: /lib\.rs:80/ }));
    expect(onOpenCitation).toHaveBeenCalledWith("crates/ingest/src/lib.rs", 80);
  });

  it("carries how long it took and how much it read", () => {
    renderAnswer();

    expect(screen.getByText("1.1s · 22 sources")).toBeVisible();
    expect(screen.getByText("40 retrieved, 22 packed")).toBeVisible();
  });

  it("states its gaps explicitly", () => {
    renderAnswer();

    expect(screen.getByText("needs a running program")).toBeVisible();
    expect(screen.getByText(/read from config at start-up/)).toBeVisible();
  });

  it("does not let no gaps read as a guarantee", () => {
    renderAnswer({ answer: { ...answer, gaps: [] } });

    expect(
      screen.getByText(/That is not a promise the answer is complete/),
    ).toBeVisible();
  });

  it("calls out a fact that arrived without the citation its model requires", () => {
    renderAnswer({
      answer: { ...answer, claims: [{ ...fact, citations: [] }], citations: [] },
    });

    expect(screen.getByText("stated as a fact but arrived with no citation")).toBeVisible();
  });

  it("says the answer was drawn from part of what was found when truncated", () => {
    renderAnswer({ answer: { ...answer, truncated: true } });

    expect(screen.getByText(/drawn from part of what was found/)).toBeVisible();
  });

  it("renders claims already on the stream before the answer resolves", () => {
    renderAnswer({
      answer: null,
      state: "loading",
      stage: "reading 40 candidates",
      streamed: [fact],
      streamedCitations: [citation],
    });

    expect(screen.getByText("reading 40 candidates")).toBeVisible();
    expect(screen.getByText(fact.text)).toBeVisible();
  });

  it("reports a failure without inventing an answer", () => {
    renderAnswer({ answer: null, state: "failed", error: "the index is empty" });

    expect(screen.getByText("the index is empty")).toBeVisible();
    expect(screen.getByText("No answer")).toBeVisible();
  });
});
