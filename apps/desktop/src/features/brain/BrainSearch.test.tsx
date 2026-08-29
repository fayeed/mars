import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BrainSearch } from "./BrainSearch";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const symbol = {
  id: "symbol-1",
  name: "run",
  kind: "function",
  qualifiedName: "run",
  relativePath: "src/main.ts",
  startLine: 4,
  endLine: 7,
  exported: true,
};

describe("BrainSearch", () => {
  it("indexes, searches, explains, and opens a source definition", async () => {
    const user = userEvent.setup();
    const onOpenSource = vi.fn();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "index_repository") {
        return Promise.resolve({
          runId: "run-1",
          indexedFiles: 6,
          skippedFiles: 0,
          deletedFiles: 0,
        });
      }
      if (command === "find_symbols") return Promise.resolve([symbol]);
      if (command === "brain_related_code") {
        return Promise.resolve({ root: symbol, symbols: [], edges: [] });
      }
      return Promise.reject(new Error(`unexpected command: ${command}`));
    });
    render(<BrainSearch onOpenSource={onOpenSource} repositoryId="repository-1" />);

    await user.click(screen.getByRole("button", { name: "Index project" }));
    expect(await screen.findByText("6 indexed · 0 unchanged")).toBeVisible();

    await user.type(screen.getByRole("textbox", { name: "Find symbol" }), "run");
    await user.click(screen.getByRole("button", { name: "Search symbols" }));
    expect(await screen.findAllByText("src/main.ts:4")).toHaveLength(2);
    expect(mocks.invoke).toHaveBeenCalledWith("find_symbols", {
      repositoryId: "repository-1",
      query: "run",
      limit: 50,
    });

    await user.click(screen.getByRole("button", { name: "Open definition" }));
    expect(onOpenSource).toHaveBeenCalledWith("src/main.ts", 4);
  });
});
