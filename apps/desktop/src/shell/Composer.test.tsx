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

  it("offers Stop instead of Send while a run is streaming", async () => {
    const user = userEvent.setup();
    const onStop = vi.fn();
    renderComposer({ state: "streaming", onStop });

    expect(screen.queryByRole("button", { name: /Send/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Stop" }));
    expect(onStop).toHaveBeenCalled();
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
