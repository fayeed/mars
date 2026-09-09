import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { Composer } from "./Composer";
import { SHELL_BINDINGS } from "./keymap";
import type { ComposerAgent, ShellTask } from "./model";
import { ShellFrame } from "./ShellFrame";
import { ShellProvider } from "./shell-store";
import { ShortcutSheet } from "./ShortcutSheet";
import { ThemeProvider } from "./theme";

const agents: ComposerAgent[] = [
  {
    id: "claude",
    name: "Claude",
    permissionScope: "read files in this task's worktree",
    asks: ["run_command"],
  },
];

const tasks: ShellTask[] = [
  { id: "task-1", title: "Extract the broker", state: "streaming", branch: "mars/broker" },
  { id: "task-2", title: "Recovery runbook", state: "blocked", branch: "mars/recovery" },
];

function renderShell(overrides: Parameters<typeof ShellFrame>[0] = {}) {
  const memory = new Map<string, string>();
  const storage = {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => void memory.set(key, value),
  };

  return render(
    <ThemeProvider storage={storage}>
      <ShellProvider storage={storage}>
        <ShellFrame
          taskRail={{ tasks, selectedTaskId: "task-1" }}
          conversation={{ composer: <Composer agents={agents} /> }}
          workbench={{ editor: { content: <p>editor surface</p> } }}
          {...overrides}
        />
      </ShellProvider>
    </ThemeProvider>,
  );
}

describe("ShellFrame regions", () => {
  it("shows exactly three destinations plus theme and account", () => {
    renderShell();
    const rail = screen.getByRole("navigation", { name: "Destinations" });
    expect(within(rail).getAllByRole("button")).toHaveLength(5);
    expect(within(rail).getByRole("button", { name: "Conversation" })).toBeVisible();
    expect(within(rail).getByRole("button", { name: "Editor" })).toBeVisible();
    expect(within(rail).getByRole("button", { name: "Source control" })).toBeVisible();
  });

  it("keeps the conversation and its composer present on every destination", async () => {
    const user = userEvent.setup();
    renderShell();

    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Editor" }));
    expect(screen.getByRole("region", { name: "Editor" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Message" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Source control" }));
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
  });

  it("marks the current destination and moves the marker on navigation", async () => {
    const user = userEvent.setup();
    renderShell();

    expect(screen.getByRole("button", { name: "Conversation" })).toHaveAttribute(
      "aria-current",
      "page",
    );

    await user.keyboard("{Control>}2{/Control}");
    expect(screen.getByRole("button", { name: "Editor" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Conversation" })).not.toHaveAttribute(
      "aria-current",
    );
  });
});

describe("task rail", () => {
  it("lists tasks with their state and collapses to a labelled strip", async () => {
    const user = userEvent.setup();
    renderShell();

    expect(screen.getByRole("complementary", { name: "Tasks" })).toBeVisible();
    expect(screen.getByText("Extract the broker")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Collapse tasks" }));

    expect(screen.queryByRole("complementary", { name: "Tasks" })).not.toBeInTheDocument();
    const strip = screen.getByRole("button", { name: /Tasks 2/ });
    expect(strip).toBeVisible();

    await user.click(strip);
    expect(screen.getByRole("complementary", { name: "Tasks" })).toBeVisible();
  });

  it("toggles from the keyboard", async () => {
    const user = userEvent.setup();
    renderShell();

    await user.keyboard("{Control>}b{/Control}");
    expect(screen.queryByRole("complementary", { name: "Tasks" })).not.toBeInTheDocument();

    await user.keyboard("{Control>}b{/Control}");
    expect(screen.getByRole("complementary", { name: "Tasks" })).toBeVisible();
  });

  it("offers a retry rather than an empty list when the core did not answer", async () => {
    const user = userEvent.setup();
    let retried = 0;
    renderShell({
      taskRail: { tasks: [], state: "failed", onRetry: () => (retried += 1) },
    });

    expect(screen.getByRole("alert")).toHaveTextContent("Tasks could not be listed");
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(retried).toBe(1);
  });
});

describe("inspector", () => {
  it("collapses to a rail whose tabs expand it again", async () => {
    const user = userEvent.setup();
    renderShell({ inspector: { content: { plan: <p>the plan</p> } } });

    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Collapse inspector" }));
    expect(screen.queryByRole("complementary", { name: "Inspector" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Plan" }));
    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeVisible();
    expect(screen.getByText("the plan")).toBeVisible();
  });

  it("explains each empty section instead of showing a blank panel", () => {
    renderShell({ inspector: { statuses: { context: { state: "stale" } } } });
    expect(screen.getByText("Context is out of date")).toBeVisible();
    expect(
      screen.getByText(/The working tree moved since this was computed/),
    ).toBeVisible();
  });
});

describe("utility panel", () => {
  it("keeps its tab strip in place whether it is open or closed", async () => {
    const user = userEvent.setup();
    renderShell({ utility: { content: { tests: <p>test output</p> } } });

    const strip = screen.getByRole("tablist", { name: "Utility panel" });
    expect(within(strip).getAllByRole("tab")).toHaveLength(4);
    expect(screen.queryByText("test output")).not.toBeInTheDocument();

    await user.click(within(strip).getByRole("tab", { name: /Tests/ }));
    expect(screen.getByText("test output")).toBeVisible();
    expect(screen.getByRole("tablist", { name: "Utility panel" })).toBe(strip);

    await user.click(within(strip).getByRole("tab", { name: /Tests/ }));
    expect(screen.queryByText("test output")).not.toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "Utility panel" })).toBe(strip);
  });

  it("opens from the keyboard without replacing the conversation", async () => {
    const user = userEvent.setup();
    renderShell({ utility: { content: { problems: <p>no problems</p> } } });

    await user.keyboard("{Control>}j{/Control}");
    expect(screen.getByText("no problems")).toBeVisible();
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
    expect(screen.getByRole("combobox", { name: "Message" })).toBeVisible();
  });
});

describe("keyboard navigation", () => {
  it("walks the regions with F6 and returns to the composer on demand", async () => {
    const user = userEvent.setup();
    renderShell();

    // Regions are walked in reading order: tasks, conversation, composer,
    // inspector. Focus starts on the conversation.
    await user.keyboard("{F6}");
    expect(screen.getByRole("combobox", { name: "Message" })).toHaveFocus();

    await user.keyboard("{Shift>}{F6}{/Shift}");
    expect(screen.getByRole("region", { name: "Conversation" })).toHaveFocus();

    await user.keyboard("{Control>}l{/Control}");
    expect(screen.getByRole("combobox", { name: "Message" })).toHaveFocus();
  });
});

describe("status bar", () => {
  it("states where the work is happening", () => {
    renderShell({
      status: {
        repository: "mars",
        branch: "mars/broker",
        agents: { state: "streaming", count: 1 },
        permissions: { state: "blocked", pending: 2 },
      },
    });

    const status = screen.getByRole("contentinfo", { name: "Status" });
    expect(within(status).getByText("mars/broker")).toBeVisible();
    expect(within(status).getByText("2 pending")).toBeVisible();
  });
});

describe("tab strips are walkable", () => {
  it("keeps focus with the selection so a second arrow steps on from the first", async () => {
    // The roving tabindex used to change the selection without moving focus.
    // Focus stayed on the tab it started from, so every further arrow press
    // stepped from there — pressing Right twice landed on the same tab both
    // times, and the strip could not be walked at all.
    const user = userEvent.setup();
    renderShell();

    const strip = screen.getByRole("tablist", { name: "Utility panel" });
    const tabs = within(strip).getAllByRole("tab");
    tabs[0].focus();

    await user.keyboard("{ArrowRight}");
    expect(tabs[1]).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(tabs[2]).toHaveFocus();
    await user.keyboard("{ArrowLeft}");
    expect(tabs[1]).toHaveFocus();
  });

  it("wraps around the end of the strip", async () => {
    const user = userEvent.setup();
    renderShell();

    const strip = screen.getByRole("tablist", { name: "Utility panel" });
    const tabs = within(strip).getAllByRole("tab");
    tabs[0].focus();

    await user.keyboard("{ArrowLeft}");
    expect(tabs[tabs.length - 1]).toHaveFocus();
  });
});

describe("the keyboard is documented from the keyboard", () => {
  it("opens the shortcut sheet on its own binding and lists every shell chord", async () => {
    const user = userEvent.setup();

    function WithSheet() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <ShortcutSheet onClose={() => setOpen(false)} open={open} />
          <ShellFrame
            conversation={{ composer: <Composer agents={agents} /> }}
            keys={{ onShortcuts: () => setOpen((current) => !current) }}
          />
        </>
      );
    }

    const memory = new Map<string, string>();
    const storage = {
      getItem: (key: string) => memory.get(key) ?? null,
      setItem: (key: string, value: string) => void memory.set(key, value),
    };
    render(
      <ThemeProvider storage={storage}>
        <ShellProvider storage={storage}>
          <WithSheet />
        </ShellProvider>
      </ThemeProvider>,
    );

    await user.keyboard("{Control>}/{/Control}");
    const sheet = screen.getByRole("dialog", { name: "Keyboard shortcuts" });

    // Generated from SHELL_BINDINGS rather than transcribed, so a binding
    // cannot be added without appearing here.
    for (const binding of SHELL_BINDINGS) {
      expect(within(sheet).getByText(binding.label)).toBeVisible();
    }
  });
});
