import type { PaletteHit } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ShellProvider } from "@/shell/shell-store";

import { CommandPalette } from "./CommandPalette";

const hits: PaletteHit[] = [
  {
    kind: "file",
    label: "lib.rs",
    detail: "crates/ingest/src/lib.rs",
    target: "crates/ingest/src/lib.rs",
    line: null,
    taskId: null,
    sequence: null,
  },
  {
    kind: "task",
    label: "Rate limit ingest",
    detail: "mars/rate-limit",
    target: "task-1",
    line: null,
    taskId: "task-1",
    sequence: null,
  },
  {
    kind: "memory",
    label: "Limits are keyed on the token",
    detail: "convention · repo",
    target: "memory-1",
    line: null,
    taskId: null,
    sequence: null,
  },
];

/**
 * The palette is a shell surface: it claims the keyboard from the shell's own
 * chords while it is open, so it is rendered inside the provider that owns
 * them rather than on its own.
 */
function renderPalette(props: Partial<Parameters<typeof CommandPalette>[0]> = {}) {
  return render(
    <ShellProvider>
      <CommandPalette
        hits={hits}
        onClose={vi.fn()}
        onJump={vi.fn()}
        onSearch={vi.fn()}
        open
        state="idle"
        {...props}
      />
    </ShellProvider>,
  );
}

describe("CommandPalette", () => {
  it("groups results by kind rather than ranking them against each other", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "limit");

    expect(screen.getByText("Tasks")).toBeVisible();
    expect(screen.getByText("Files")).toBeVisible();
    expect(screen.getByText("Memories")).toBeVisible();
  });

  it("puts tasks before files, whatever order they arrived in", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "limit");

    const headings = screen.getAllByText(/Tasks|Files|Memories/).map((node) => node.textContent);
    expect(headings).toEqual(["Tasks", "Files", "Memories"]);
  });

  it("asks for results on every keystroke", async () => {
    const user = userEvent.setup();
    const onSearch = vi.fn();
    renderPalette({ onSearch });

    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "ab");
    expect(onSearch).toHaveBeenLastCalledWith("ab");
  });

  it("walks the whole list with the arrows, across group boundaries", async () => {
    const user = userEvent.setup();
    const onJump = vi.fn();
    renderPalette({ onJump });
    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "limit");

    // First option is the task; two downs reaches the memory, in a third group.
    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ kind: "memory" }));
  });

  it("wraps around rather than trapping the cursor at the end", async () => {
    const user = userEvent.setup();
    const onJump = vi.fn();
    renderPalette({ onJump });
    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "limit");

    await user.keyboard("{ArrowUp}{Enter}");
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ kind: "memory" }));
  });

  it("closes on Escape", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    renderPalette({ onClose });

    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "x");
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("says why something might be missing rather than only that nothing matched", async () => {
    const user = userEvent.setup();
    renderPalette({ hits: [] });

    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "zzz");
    expect(screen.getByText(/anything unindexed will not appear here/)).toBeVisible();
  });

  it("renders nothing at all when closed", () => {
    const { container } = renderPalette({ open: false });
    expect(container).toBeEmptyDOMElement();
  });

  it("jumps on click", async () => {
    const user = userEvent.setup();
    const onJump = vi.fn();
    renderPalette({ onJump });
    await user.type(screen.getByRole("combobox", { name: "Search everything" }), "limit");

    const files = screen.getByText("Files").closest("section");
    await user.click(within(files as HTMLElement).getByText("lib.rs"));
    expect(onJump).toHaveBeenCalledWith(expect.objectContaining({ kind: "file" }));
  });
});
