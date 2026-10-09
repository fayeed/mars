import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Composer, type ComposerProps } from "./Composer";
import type { ComposerAgent, ComposerSkill, MentionTarget } from "./model";
import { ShellProvider } from "./shell-store";

const agents: ComposerAgent[] = [
  {
    id: "claude",
    name: "Claude",
    permissionScope: "read files in this task's worktree",
    asks: ["write_files", "run_command"],
  },
  { id: "codex", name: "Codex", permissionScope: "read files in this task's worktree" },
];

const mentions: MentionTarget[] = [
  { id: "a", kind: "file", label: "Composer.tsx", detail: "src/shell/Composer.tsx" },
  { id: "b", kind: "file", label: "App.tsx", detail: "src/App.tsx" },
];

const skills: ComposerSkill[] = [
  { id: "s1", name: "review", description: "Cross-agent review of the diff" },
  { id: "s2", name: "release", description: "Cut a release branch" },
];

function renderComposer(props: ComposerProps = {}) {
  const storage = { getItem: () => null, setItem: () => undefined };
  return render(
    <ShellProvider storage={storage}>
      <Composer agents={agents} mentions={mentions} skills={skills} {...props} />
    </ShellProvider>,
  );
}

describe("Composer mentions and skills", () => {
  it("completes an @ mention to its repository path", async () => {
    const user = userEvent.setup();
    renderComposer();

    const message = screen.getByRole("combobox", { name: "Message" });
    await user.click(message);
    await user.keyboard("look at @Compos");

    // The agent picker is a listbox too, so scope the assertion to the menu.
    const listbox = screen.getByRole("listbox", { name: "Files and symbols" });
    expect(within(listbox).getAllByRole("option")).toHaveLength(1);

    await user.keyboard("{Enter}");
    expect(message).toHaveValue("look at @src/shell/Composer.tsx ");
    expect(listbox).not.toBeInTheDocument();
  });

  it("walks the menu with the arrow keys and dismisses it with Escape", async () => {
    const user = userEvent.setup();
    renderComposer();

    await user.click(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("@");
    const listbox = screen.getByRole("listbox", { name: "Files and symbols" });
    expect(within(listbox).getAllByRole("option")).toHaveLength(2);

    await user.keyboard("{ArrowDown}");
    expect(within(listbox).getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{Escape}");
    expect(
      screen.queryByRole("listbox", { name: "Files and symbols" }),
    ).not.toBeInTheDocument();
  });

  it("offers skills at the start of a line but not inside a path", async () => {
    const user = userEvent.setup();
    renderComposer();

    await user.click(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("/rev");
    expect(screen.getByRole("listbox", { name: "Skills" })).toBeVisible();

    await user.clear(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("edit src/shell");
    expect(screen.queryByRole("listbox", { name: "Skills" })).not.toBeInTheDocument();
  });
});

describe("Composer sending", () => {
  it("sends on Ctrl+Enter with the chosen agent, then clears", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderComposer({ onSubmit, selectedAgentId: "codex" });

    const message = screen.getByRole("combobox", { name: "Message" });
    await user.click(message);
    await user.keyboard("split the broker");
    await user.keyboard("{Control>}{Enter}{/Control}");

    expect(onSubmit).toHaveBeenCalledWith({
      text: "split the broker",
      agentId: "codex",
      attachmentIds: [],
    });
    expect(message).toHaveValue("");
  });

  it("will not send an empty message", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderComposer({ onSubmit });

    expect(screen.getByRole("button", { name: /Send/ })).toBeDisabled();
    await user.click(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("   ");
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("offers Stop beside Queue while a run in this task is streaming", async () => {
    const user = userEvent.setup();
    const onStop = vi.fn();
    const onSubmit = vi.fn();
    renderComposer({
      state: "streaming",
      onStop,
      onSubmit,
      target: { mode: "task", taskTitle: "Tune retries" },
      queuedCount: 1,
    });

    // A message written now waits for the run rather than being refused.
    expect(screen.getByRole("button", { name: /Queue/ })).toBeInTheDocument();
    expect(screen.getByText("1 queued")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalled();

    await user.type(screen.getByRole("combobox", { name: "Message" }), "also cover the timeout");
    await user.click(screen.getByRole("button", { name: /Queue/ }));
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ text: "also cover the timeout" }),
    );
  });
});

describe("Composer target", () => {
  it("says which task a message continues and offers a new task instead", async () => {
    const user = userEvent.setup();
    const onTargetChange = vi.fn();
    renderComposer({ target: { mode: "task", taskTitle: "Tune retries" }, onTargetChange });

    expect(screen.getByText("Continuing")).toBeInTheDocument();
    expect(screen.getByText("Tune retries")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "New task instead" }));
    expect(onTargetChange).toHaveBeenCalledWith("new");
  });

  it("says a message starts a new task when none is open", () => {
    renderComposer({ target: { mode: "new" } });
    expect(screen.getByText("New task")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /instead/ })).not.toBeInTheDocument();
  });
});

describe("Composer permission scope", () => {
  it("states what the selected agent may already do and what it will ask for", () => {
    renderComposer({ selectedAgentId: "claude" });

    const hint = screen.getByText(/may read files in this task's worktree/);
    expect(hint).toBeVisible();
    expect(hint).toHaveTextContent("write_files, run_command");
  });

  it("follows the agent picker", async () => {
    const user = userEvent.setup();
    const onSelectAgent = vi.fn();
    const { rerender } = renderComposer({ selectedAgentId: "claude", onSelectAgent });

    await user.selectOptions(screen.getByRole("combobox", { name: "Agent" }), "codex");
    expect(onSelectAgent).toHaveBeenCalledWith("codex");

    rerender(
      <ShellProvider storage={{ getItem: () => null, setItem: () => undefined }}>
        <Composer agents={agents} mentions={mentions} skills={skills} selectedAgentId="codex" />
      </ShellProvider>,
    );
    expect(screen.getByText(/may read files in this task's worktree/)).toHaveTextContent("Codex");
  });

  it("says why sending will not help while a permission request is open", () => {
    renderComposer({ state: "blocked", blockedReason: "run_command: cargo test — waiting on you" });

    expect(screen.getByText("run_command: cargo test — waiting on you")).toBeVisible();
    expect(screen.getByRole("button", { name: /Send/ })).toBeDisabled();
  });
});

describe("Composer attachments", () => {
  it("lists attachments and removes them", async () => {
    const user = userEvent.setup();
    const onRemoveAttachment = vi.fn();
    renderComposer({
      attachments: [{ id: "att-1", label: "trace.log", detail: "18 KiB" }],
      onRemoveAttachment,
    });

    expect(screen.getByText("trace.log")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Remove trace.log" }));
    expect(onRemoveAttachment).toHaveBeenCalledWith("att-1");
  });
});

describe("Composer skills and scope", () => {
  it("shows a skill's own instructions before it is picked", async () => {
    const user = userEvent.setup();
    renderComposer({
      skills: [
        {
          id: "s1",
          name: "review",
          description: "Cross-agent review of the diff",
          instructions: "Read the diff. Report only defects you can point at.",
        },
      ],
    });

    await user.click(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("/rev");

    expect(screen.getByText("Read the diff. Report only defects you can point at.")).toBeVisible();
  });

  it("sends the scope the message was written under", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderComposer({
      onSubmit,
      scopes: [
        { id: "narrow", label: "Files I mention", globs: [] },
        { id: "crates", label: "crates/**", globs: ["crates/**"] },
      ],
      selectedScopeId: "crates",
    });

    await user.click(screen.getByRole("combobox", { name: "Message" }));
    await user.keyboard("split the broker");
    await user.keyboard("{Control>}{Enter}{/Control}");

    expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({ scopeId: "crates" }));
  });

  it("states the scope alongside what the agent may already do", () => {
    const { container } = renderComposer({
      scopes: [{ id: "crates", label: "crates/**", globs: ["crates/**"] }],
      selectedScopeId: "crates",
    });

    // The same glob is also an option label in the picker, so read the hint.
    const hint = container.querySelector('[data-slot="permission-hint"]');
    expect(hint).toHaveTextContent("within");
    expect(hint).toHaveTextContent("crates/**");
  });

  it("lets the screen drive the draft", () => {
    renderComposer({ value: "Explain what changed in 511c628" });

    expect(screen.getByRole("combobox", { name: "Message" })).toHaveValue(
      "Explain what changed in 511c628",
    );
  });
});

describe("an agent this machine does not have", () => {
  it("says so before a message is written, not after it is sent", () => {
    renderComposer({
      agents: [
        {
          id: "codex",
          name: "Codex",
          permissionScope: "read files in this task's worktree",
          unavailable: "is not installed on this machine",
        },
      ],
    });

    expect(screen.getByText("is not installed on this machine")).toBeVisible();
  });

  it("still lets the message be typed and sent, because /ask needs no agent", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    renderComposer({
      agents: [
        {
          id: "codex",
          name: "Codex",
          permissionScope: "read files",
          unavailable: "is not installed on this machine",
        },
      ],
      onSubmit,
    });

    await user.type(screen.getByRole("combobox", { name: "Message" }), "/ask what is this");
    await user.click(screen.getByRole("button", { name: /Send/ }));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});
