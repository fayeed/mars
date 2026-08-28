import type { FileDocument } from "@mars/shared-types";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { CodeEditor } from "./CodeEditor";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock(import("./monaco"), () => ({}));
vi.mock("@monaco-editor/react", () => ({
  default: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <textarea aria-label="Monaco editor" onChange={(event) => onChange(event.target.value)} value={value} />
  ),
}));

const original: FileDocument = {
  relativePath: "src/main.ts",
  content: "export const value = 1;\n",
  version: "version-1",
  size: 24,
};

function Harness({ onReload = vi.fn() }: { onReload?: () => void }) {
  const [document, setDocument] = useState(original);
  const [value, setValue] = useState(document.content);
  return (
    <CodeEditor
      document={document}
      onChange={setValue}
      onReload={onReload}
      onSaved={(saved) => {
        setDocument(saved);
        setValue(saved.content);
      }}
      repositoryId="repository-1"
      value={value}
    />
  );
}

describe("CodeEditor", () => {
  it("saves the current draft against the loaded version", async () => {
    const user = userEvent.setup();
    const saved = { ...original, content: "export const value = 2;\n", version: "version-2" };
    mocks.invoke.mockResolvedValueOnce(saved);
    render(<Harness />);

    fireEvent.change(screen.getByRole("textbox", { name: "Monaco editor" }), {
      target: { value: saved.content },
    });
    expect(screen.getByText("Unsaved")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(mocks.invoke).toHaveBeenCalledWith("write_file", {
      repositoryId: "repository-1",
      relativePath: original.relativePath,
      content: saved.content,
      expectedVersion: original.version,
    });
    expect(await screen.findByText("Saved")).toBeVisible();
  });

  it("surfaces optimistic-save conflicts and offers a disk reload", async () => {
    const user = userEvent.setup();
    const onReload = vi.fn();
    mocks.invoke.mockRejectedValueOnce({
      code: "file.write_conflict",
      message: "file changed on disk",
    });
    render(<Harness onReload={onReload} />);

    fireEvent.change(screen.getByRole("textbox", { name: "Monaco editor" }), {
      target: { value: "external conflict" },
    });
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("File changed on disk");
    await user.click(screen.getByRole("button", { name: "Reload from disk" }));
    expect(onReload).toHaveBeenCalledOnce();
  });
});
