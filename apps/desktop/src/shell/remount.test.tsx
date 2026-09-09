import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect } from "react";
import { describe, expect, it } from "vitest";

import { ShellFrame } from "./ShellFrame";
import { ShellProvider } from "./shell-store";
import { ThemeProvider } from "./theme";

let counts = { conversation: 0 };

/**
 * Counts its own mounts.
 *
 * Standing in for Monaco, xterm and the conversation's scroll position: a
 * remount throws all three away, and none of them come back where they were.
 */
function Probe() {
  useEffect(() => {
    counts.conversation += 1;
  }, []);
  return <p>conversation body</p>;
}

function renderShell() {
  const memory = new Map<string, string>();
  const storage = {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => void memory.set(key, value),
  };
  return render(
    <ThemeProvider storage={storage}>
      <ShellProvider storage={storage}>
        <ShellFrame
          conversation={{ children: <Probe /> }}
          workbench={{ editor: { content: <p>editor body</p> } }}
          utility={{ content: { problems: <p>problems body</p> } }}
        />
      </ShellProvider>
    </ThemeProvider>,
  );
}

describe("layout changes must not remount what they move around", () => {
  it("keeps the conversation mounted when the utility panel opens", async () => {
    const user = userEvent.setup();
    counts = { conversation: 0 };
    renderShell();
    expect(counts.conversation).toBe(1);

    await user.keyboard("{Control>}j{/Control}");
    expect(screen.getByText("problems body")).toBeVisible();
    expect(counts.conversation).toBe(1);

    await user.keyboard("{Control>}j{/Control}");
    expect(counts.conversation).toBe(1);
  });

  it("keeps the conversation mounted when a workbench opens beside it", async () => {
    const user = userEvent.setup();
    counts = { conversation: 0 };
    renderShell();
    expect(counts.conversation).toBe(1);

    await user.keyboard("{Control>}2{/Control}");
    expect(counts.conversation).toBe(1);

    await user.keyboard("{Control>}1{/Control}");
    expect(counts.conversation).toBe(1);
  });
});
