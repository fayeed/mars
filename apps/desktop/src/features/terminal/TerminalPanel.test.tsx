import type { ProjectSnapshot } from "@mars/shared-types";
import { render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TerminalPanel, type TerminalCommand } from "./TerminalPanel";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}));
// xterm draws to a canvas jsdom does not have. What is under test is what gets
// written to the shell, not how it is drawn.
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    loadAddon() {}
    open() {}
    onData() {
      return { dispose() {} };
    }
    write() {}
    focus() {}
    dispose() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));

const project: ProjectSnapshot = {
  projectId: "project-1",
  repositoryId: "repository-1",
  name: "mars",
  rootPath: "/work/mars",
  gitCommonDir: "/work/mars/.git",
};

afterEach(() => {
  vi.clearAllMocks();
});

function shellAnswers() {
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "spawn_terminal") {
      return Promise.resolve({ sessionId: "terminal-1", cwd: "/work/mars" });
    }
    return Promise.resolve(undefined);
  });
}

function writes() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "write_terminal");
}

describe("a command typed for the person", () => {
  it("starts a shell, types the command once, and lets go of it", async () => {
    shellAnswers();
    const onCommandSent = vi.fn();
    const command: TerminalCommand = { id: 1, text: "claude auth login" };
    const { rerender } = render(
      <TerminalPanel command={command} onCommandSent={onCommandSent} project={project} />,
    );

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("write_terminal", {
        sessionId: "terminal-1",
        data: "claude auth login\r",
      }),
    );
    expect(onCommandSent).toHaveBeenCalledWith(1);

    // The same request seen again on a re-render is not a second request.
    rerender(<TerminalPanel command={command} onCommandSent={onCommandSent} project={project} />);
    expect(writes()).toHaveLength(1);

    rerender(
      <TerminalPanel
        command={{ id: 2, text: "claude auth login" }}
        onCommandSent={onCommandSent}
        project={project}
      />,
    );
    await waitFor(() => expect(writes()).toHaveLength(2));
    expect(mocks.invoke.mock.calls.filter(([name]) => name === "spawn_terminal")).toHaveLength(1);
  });
});
