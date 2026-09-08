import type { BrainAnswer as BrainAnswerValue } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

import { BrainAnswer } from "./BrainAnswer";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock(import("@tauri-apps/api/event"), () => ({ listen: mocks.listen }));

const answer: BrainAnswerValue = {
  question: "How does the retry engine decide how many attempts to make?",
  rewrittenQueries: ["retry engine attempts", "retry backoff attempt"],
  claims: [
    {
      id: "claim:0",
      kind: "fact",
      text: "src/engine.rs defines `run_with_retries`.",
      citations: ["cite:0"],
      supports: [],
      confidence: 0.9,
    },
    {
      id: "claim:1",
      kind: "inference",
      text: "The attempt ceiling is a compile-time constant, not configuration.",
      citations: [],
      supports: ["claim:0"],
      confidence: 0.6,
    },
  ],
  citations: [
    {
      id: "cite:0",
      path: "src/engine.rs",
      lineStart: 39,
      lineEnd: 44,
      blobSha: "abc123",
      snippet: "pub fn run_with_retries(",
      why: "hybrid search for the keyword query matched run_with_retries",
    },
  ],
  sources: 22,
  retrieved: 64,
  latencyMs: 1100,
  confidence: 0.78,
  gaps: [
    {
      kind: "requiresRuntime",
      subject: "the attempt count actually used",
      detail: "It depends on the argument passed at the call site.",
    },
  ],
  sourceDetail: [],
  truncated: false,
};

afterEach(() => vi.clearAllMocks());

it("reports sources and latency verbatim and keeps facts apart from inferences", async () => {
  const user = userEvent.setup();
  const onOpenSource = vi.fn();
  mocks.listen.mockResolvedValue(() => {});
  mocks.invoke.mockResolvedValue(answer);

  render(<BrainAnswer onOpenSource={onOpenSource} repositoryId="repository-1" />);
  await user.type(screen.getByLabelText("Ask Project Brain"), "how does retry work");
  await user.click(screen.getByRole("button", { name: "Ask" }));

  // The figures the pipeline measured, shown as measured.
  expect(await screen.findByLabelText("Answer summary")).toHaveTextContent("1.1s · 22 sources");
  expect(screen.getByLabelText("Answer summary")).toHaveTextContent("78% confidence");

  // Facts and inferences live in separate regions; neither may contain the other.
  const facts = screen.getByLabelText("Facts");
  const inferences = screen.getByLabelText("Inferences");
  expect(within(facts).getByText(/run_with_retries/)).toBeVisible();
  expect(within(facts).queryByText(/compile-time constant/)).toBeNull();
  expect(within(inferences).getByText(/compile-time constant/)).toBeVisible();
  expect(within(inferences).getByText(/rests on 1 fact\(s\)/)).toBeVisible();

  // Gaps say what could not be determined, and why.
  expect(screen.getByLabelText("Gaps")).toHaveTextContent("needs a running program");
});

it("opens the exact cited span", async () => {
  const user = userEvent.setup();
  const onOpenSource = vi.fn();
  mocks.listen.mockResolvedValue(() => {});
  mocks.invoke.mockResolvedValue(answer);

  render(<BrainAnswer onOpenSource={onOpenSource} repositoryId="repository-1" />);
  await user.type(screen.getByLabelText("Ask Project Brain"), "how does retry work");
  await user.click(screen.getByRole("button", { name: "Ask" }));

  await user.click(await screen.findByRole("button", { name: /src\/engine\.rs:39-44/ }));
  expect(onOpenSource).toHaveBeenCalledWith("src/engine.rs", 39);
  expect(mocks.invoke).toHaveBeenCalledWith("brain_ask", {
    repositoryId: "repository-1",
    question: "how does retry work",
    streamId: "1",
  });
});

it("renders claims as they stream, before the answer resolves", async () => {
  const user = userEvent.setup();
  let emit: ((payload: { payload: unknown }) => void) | undefined;
  mocks.listen.mockImplementation((_name: string, handler: (payload: { payload: unknown }) => void) => {
    emit = handler;
    return Promise.resolve(() => {});
  });
  // A call that never settles, so only streamed claims can be on screen.
  mocks.invoke.mockReturnValue(new Promise(() => {}));

  render(<BrainAnswer onOpenSource={vi.fn()} repositoryId="repository-1" />);
  await user.type(screen.getByLabelText("Ask Project Brain"), "how does retry work");
  await user.click(screen.getByRole("button", { name: "Ask" }));

  emit?.({
    payload: {
      streamId: "1",
      event: { type: "packed", sources: 22, tokens: 7100, dropped: 42 },
    },
  });
  emit?.({
    payload: { streamId: "1", event: { type: "citation", ...answer.citations[0] } },
  });
  emit?.({
    payload: { streamId: "1", event: { type: "claim", ...answer.claims[0] } },
  });

  expect(await screen.findByText(/packed 22 sources into 7100 tokens/)).toBeVisible();
  expect(within(screen.getByLabelText("Facts")).getByText(/run_with_retries/)).toBeVisible();
});
