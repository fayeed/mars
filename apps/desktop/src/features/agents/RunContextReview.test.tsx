import type { RunContextPack } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { RunContextReview } from "./RunContextReview";

const pack: RunContextPack = {
  taskId: "task-1",
  agentId: "claude",
  role: "implementer",
  adapter: "claude",
  entries: [
    {
      section: "instructions",
      kind: "memory",
      entityRef: "memory:m-1",
      title: "convention (repo)",
      provenance: "HUMAN approved by fayeed",
      freshness: "approved 2026-09-01T10:00:00Z",
      sourceLocation: "conversation:conv-7:msg-12",
      reason: "names the error-mapping rule this task will touch",
      byteSize: 512,
    },
    {
      section: "code",
      kind: "file",
      entityRef: "file:crates/ingest/src/lib.rs",
      title: "crates/ingest/src/lib.rs",
      provenance: "EXTRACTED_CODE",
      freshness: "indexed at 511c628",
      sourceLocation: "crates/ingest/src/lib.rs:1-80",
      reason: "the endpoint named in the request lives here",
      byteSize: 4096,
    },
  ],
  sectionUsage: [
    { section: "instructions", maxBytes: 8192, usedBytes: 512 },
    { section: "code", maxBytes: 32768, usedBytes: 4096 },
  ],
  exclusions: [
    { entityRef: "file:crates/ingest/tests/big.rs", reason: "removed in pack preview" },
    { entityRef: "file:vendor/blob.rs", reason: "over the section budget" },
  ],
  tools: [
    { name: "read_file", description: "Read a file", readOnly: true },
    { name: "run_command", description: "Run a command", readOnly: false },
  ],
  permissions: {
    role: "implementer",
    ceiling: {
      readFiles: true,
      writeFiles: true,
      commands: "destructive",
      network: true,
      gitPush: true,
    },
    declaredScope: ["crates/ingest/**"],
    standingGrants: [
      {
        capability: "run_command",
        pattern: "cargo test*",
        decision: "allow_task",
        grantedBy: "fayeed",
        expiresAt: null,
      },
    ],
    everythingElseAsks: true,
  },
  prompt: "You are working in the mars repository.\nThe assignment: rate limit the ingest endpoint.",
  totalBytes: 4608,
  digest: "9f8e7d6c5b4a39281706",
};

function renderReview(props: Partial<Parameters<typeof RunContextReview>[0]> = {}) {
  return render(
    <RunContextReview
      onStart={vi.fn()}
      onToggleEntry={vi.fn()}
      pack={pack}
      removed={["file:crates/ingest/tests/big.rs"]}
      state="idle"
      {...props}
    />,
  );
}

describe("RunContextReview: what the agent will see", () => {
  it("groups entries by kind and counts each one in bytes and approximate tokens", () => {
    renderReview();

    expect(screen.getByText("Memories")).toBeVisible();
    expect(screen.getByText("Files")).toBeVisible();
    expect(screen.getByText("4.0 KiB")).toBeVisible();
    // The budget is bytes; the token figure is marked as the estimate it is.
    expect(screen.getByText("≈1,024 tok")).toBeVisible();
    expect(screen.getByText(/the token figure is an estimate/)).toBeVisible();
  });

  it("traces an injected memory back to the turn it was approved from", () => {
    renderReview();

    expect(screen.getByText("HUMAN approved by fayeed")).toBeVisible();
    expect(screen.getByText("conversation:conv-7:msg-12")).toBeVisible();
    expect(screen.getByText(/each traceable to the turn it was approved from/)).toBeVisible();
  });

  it("gives the reason each item was chosen", () => {
    renderReview();

    expect(screen.getByText("the endpoint named in the request lives here")).toBeVisible();
  });

  it("shows the section budget it was measured against", () => {
    renderReview();

    expect(screen.getByText("512 B / 8.0 KiB")).toBeVisible();
  });
});

describe("RunContextReview: removing things", () => {
  it("removes an entry", async () => {
    const user = userEvent.setup();
    const onToggleEntry = vi.fn();
    renderReview({ onToggleEntry });

    await user.click(screen.getByRole("button", { name: "Remove crates/ingest/src/lib.rs" }));
    expect(onToggleEntry).toHaveBeenCalledWith("file:crates/ingest/src/lib.rs", false);
  });

  it("offers to add back only what this person removed", async () => {
    const user = userEvent.setup();
    const onToggleEntry = vi.fn();
    renderReview({ onToggleEntry });

    // Two things are excluded; only the person's own removal is reversible here.
    const addBack = screen.getAllByRole("button", { name: /Add back/ });
    expect(addBack).toHaveLength(1);
    expect(screen.getByText("over the section budget")).toBeVisible();

    await user.click(addBack[0]);
    expect(onToggleEntry).toHaveBeenCalledWith("file:crates/ingest/tests/big.rs", true);
  });
});

describe("RunContextReview: what the agent may do", () => {
  it("calls out mutating tools and a destructive command ceiling", () => {
    renderReview();

    expect(screen.getByText("mutating")).toBeVisible();
    expect(screen.getByText("destructive")).toBeVisible();
    expect(screen.getByText(/No decision made during the run can widen it/)).toBeVisible();
  });

  it("shows the declared scope and the grants that will not ask again", () => {
    renderReview();

    expect(screen.getByText("crates/ingest/**")).toBeVisible();
    expect(screen.getByText("cargo test*")).toBeVisible();
    expect(screen.getByText(/stops the agent and asks you/)).toBeVisible();
  });

  it("says every effect asks when nothing stands", () => {
    renderReview({
      pack: {
        ...pack,
        permissions: { ...pack.permissions, standingGrants: [], declaredScope: [] },
      },
    });

    expect(screen.getByText(/Every effect this agent attempts will stop and ask/)).toBeVisible();
    expect(screen.getByText(/every write will stop and ask/)).toBeVisible();
  });
});

describe("RunContextReview: the prompt and the digest", () => {
  it("keeps the prompt readable verbatim", async () => {
    const user = userEvent.setup();
    renderReview();

    expect(screen.queryByText(/The assignment: rate limit/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /The prompt, verbatim/ }));
    expect(screen.getByText(/The assignment: rate limit the ingest endpoint/)).toBeVisible();
  });

  it("starts the run against the digest that was reviewed", async () => {
    const user = userEvent.setup();
    const onStart = vi.fn();
    renderReview({ onStart });

    expect(screen.getByText("9f8e7d6c5b4a")).toBeVisible();
    await user.click(screen.getByRole("button", { name: /Start run/ }));
    expect(onStart).toHaveBeenCalledWith(pack.digest);
  });

  it("says nothing was started when the pack could not be assembled", () => {
    renderReview({ pack: null, state: "failed", error: "budget must be at least 512 bytes" });

    expect(screen.getByText("The context pack could not be assembled")).toBeVisible();
    expect(screen.getByText("budget must be at least 512 bytes")).toBeVisible();
  });
});
