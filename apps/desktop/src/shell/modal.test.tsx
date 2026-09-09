import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { focusableWithin, useModalFocus, wrapTabTarget } from "./modal";
import { ShellFrame } from "./ShellFrame";
import { ShellProvider } from "./shell-store";
import { ThemeProvider } from "./theme";

describe("wrapTabTarget", () => {
  function elements(count: number): HTMLElement[] {
    return Array.from({ length: count }, () => document.createElement("button"));
  }

  it("leaves the browser alone in the middle of the order", () => {
    const order = elements(3);
    expect(wrapTabTarget(order, order[1], false)).toBeNull();
    expect(wrapTabTarget(order, order[1], true)).toBeNull();
  });

  it("wraps at both ends rather than letting Tab escape the dialog", () => {
    const order = elements(3);
    expect(wrapTabTarget(order, order[2], false)).toBe(order[0]);
    expect(wrapTabTarget(order, order[0], true)).toBe(order[2]);
  });

  it("pulls focus back in when it is somewhere outside", () => {
    const order = elements(3);
    const stray = document.createElement("button");
    expect(wrapTabTarget(order, stray, false)).toBe(order[0]);
    expect(wrapTabTarget(order, stray, true)).toBe(order[2]);
  });

  it("has nothing to say about an empty dialog", () => {
    expect(wrapTabTarget([], null, false)).toBeNull();
  });
});

function Dialog({ onClose }: { onClose: () => void }) {
  const { ref, onBackdropMouseDown } = useModalFocus<HTMLDivElement>({ onClose });
  return (
    <div data-testid="backdrop" onMouseDown={onBackdropMouseDown}>
      <div ref={ref} role="dialog" aria-label="Test dialog" tabIndex={-1}>
        <button type="button">First</button>
        <button type="button">Last</button>
      </div>
    </div>
  );
}

/**
 * The dialog and an opener, with nothing else in the tree.
 *
 * Used for the focus contract itself. `ShellFrame` is brought in only where the
 * test is about the shell's chords: its resizable handles move focus of their
 * own accord under jsdom, which would make the restore assertions test the
 * panel library rather than this hook.
 */
function Harness({ frame = false }: { frame?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <ThemeProvider>
      <ShellProvider>
        <button type="button" onClick={() => setOpen(true)}>
          Open
        </button>
        {open ? <Dialog onClose={() => setOpen(false)} /> : null}
        {frame ? <ShellFrame conversation={{ composer: <textarea aria-label="Message" /> }} /> : null}
      </ShellProvider>
    </ThemeProvider>
  );
}

describe("useModalFocus", () => {
  it("takes focus on open and gives it back to whatever opened it", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open" });

    await user.click(opener);
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();

    await user.keyboard("{Escape}");
    // The dialog was a detour, not a dead end: the reader ends up back on the
    // control they left, rather than on `document.body` with the next Tab
    // starting again from the top of the page.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(opener).toHaveFocus();
  });

  it("wraps Tab inside the dialog instead of walking into the shell behind it", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Open" }));

    await user.tab();
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(screen.getByRole("button", { name: "Last" })).toHaveFocus();
  });

  it("stands the shell's chords down so they cannot fling focus behind the dialog", async () => {
    const user = userEvent.setup();
    render(<Harness frame />);
    await user.click(screen.getByRole("button", { name: "Open" }));

    // Ctrl+J would open the utility panel and Ctrl+2 would jump to the editor.
    // Obeying either would leave the dialog on screen with the cursor
    // somewhere the reader cannot see.
    await user.keyboard("{Control>}j{/Control}");
    await user.keyboard("{Control>}2{/Control}");
    expect(screen.getByRole("button", { name: "First" })).toHaveFocus();
    expect(screen.getByRole("dialog")).toBeVisible();
  });
});

describe("the composer is always one key away", () => {
  it("takes down whatever is covering it rather than leaving it stranded", async () => {
    const user = userEvent.setup();
    const onDismissModals = vi.fn();

    function OneKey() {
      const [open, setOpen] = useState(true);
      return (
        <ThemeProvider>
          <ShellProvider>
            {open ? <Dialog onClose={() => setOpen(false)} /> : null}
            <ShellFrame
              conversation={{ composer: <textarea aria-label="Message" /> }}
              keys={{
                onDismissModals: () => {
                  onDismissModals();
                  setOpen(false);
                },
              }}
            />
          </ShellProvider>
        </ThemeProvider>
      );
    }

    render(<OneKey />);
    await user.keyboard("{Control>}l{/Control}");

    expect(onDismissModals).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("focusableWithin", () => {
  it("skips what a keyboard cannot reach", () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <button>ok</button>
      <button disabled>disabled</button>
      <div tabindex="-1">script-only</div>
      <div hidden><button>hidden</button></div>
      <div aria-hidden="true"><button>aria-hidden</button></div>
      <input />
    `;
    document.body.append(container);
    const found = focusableWithin(container).map((element) => element.textContent || element.tagName);
    container.remove();

    expect(found).toEqual(["ok", "INPUT"]);
  });
});
