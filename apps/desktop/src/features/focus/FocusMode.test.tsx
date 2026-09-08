import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FocusMode } from "./FocusMode";

function renderFocus(props: Partial<Parameters<typeof FocusMode>[0]> = {}) {
  return render(
    <FocusMode
      editor={<p>the editor</p>}
      explorer={<p>the explorer</p>}
      onReturn={vi.fn()}
      onToggleTerminal={vi.fn()}
      returnLabel="Rate limit ingest"
      terminal={<p>the terminal</p>}
      terminalOpen={false}
      {...props}
    />,
  );
}

describe("FocusMode", () => {
  it("gives a real editor and explorer the full height", () => {
    renderFocus();

    expect(screen.getByText("the editor")).toBeVisible();
    expect(screen.getByText("the explorer")).toBeVisible();
  });

  it("keeps the terminal one click away rather than always present", async () => {
    const user = userEvent.setup();
    const onToggleTerminal = vi.fn();
    renderFocus({ onToggleTerminal });

    expect(screen.queryByText("the terminal")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Terminal" }));
    expect(onToggleTerminal).toHaveBeenCalled();
  });

  it("shows the terminal when it is open", () => {
    renderFocus({ terminalOpen: true });

    expect(screen.getByText("the terminal")).toBeVisible();
  });

  it("names what it is borrowed from, in two places that both return", async () => {
    const user = userEvent.setup();
    const onReturn = vi.fn();
    renderFocus({ onReturn });

    // One in the header, one in the footer: the way back is never off-screen.
    await user.click(screen.getByRole("button", { name: /Back to Rate limit ingest/ }));
    await user.click(screen.getByRole("button", { name: "Return to Rate limit ingest" }));
    expect(onReturn).toHaveBeenCalledTimes(2);
  });

  it("says this mode is borrowed and that saves are recorded", () => {
    renderFocus();

    expect(
      screen.getByText(/Borrowed from the conversation. Every save here is recorded/),
    ).toBeVisible();
  });

  it("keeps reporting the conversation while nobody is watching it", () => {
    renderFocus({ conversationState: "streaming", unread: 4 });

    expect(screen.getByText("4 new in the conversation")).toBeVisible();
  });

  it("says the conversation is live even with nothing new", () => {
    renderFocus({ conversationState: "streaming" });

    expect(screen.getByText("the conversation is live")).toBeVisible();
  });

  it("stays quiet when there is nothing to report", () => {
    renderFocus();

    expect(screen.queryByText(/in the conversation/)).not.toBeInTheDocument();
  });

  it("shows what is open and on which branch", () => {
    renderFocus({ openPath: "crates/ingest/src/lib.rs", branchName: "mars/rate-limit" });

    expect(screen.getByText("crates/ingest/src/lib.rs")).toBeVisible();
    expect(screen.getByText("mars/rate-limit")).toBeVisible();
  });
});
