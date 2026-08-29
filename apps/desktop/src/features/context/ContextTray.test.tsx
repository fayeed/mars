import type { ContextItem } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { ContextTray } from "./ContextTray";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  createObjectUrl: vi.fn(() => "blob:mars-preview"),
  revokeObjectUrl: vi.fn(),
}));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const textItem: ContextItem = {
  id: "context-text",
  repositoryId: "repository-1",
  kind: "text",
  title: "Sync constraint",
  bodyText: "Existing sync behavior must remain unchanged.",
  originalUri: null,
  mimeType: "text/plain",
  contentHash: "text-digest",
  classification: null,
  quarantineReason: null,
  artifactPath: null,
  originalName: null,
  capturedAt: "2026-08-29T00:00:00Z",
};

const imageItem: ContextItem = {
  ...textItem,
  id: "context-image",
  kind: "image",
  title: "settings.png",
  bodyText: null,
  mimeType: "image/png",
  contentHash: "image-digest",
  artifactPath: "/artifacts/sha256/im/image-digest",
  originalName: "settings.png",
};

beforeAll(() => {
  Object.defineProperty(URL, "createObjectURL", { configurable: true, value: mocks.createObjectUrl });
  Object.defineProperty(URL, "revokeObjectURL", { configurable: true, value: mocks.revokeObjectUrl });
});

afterAll(() => {
  Reflect.deleteProperty(URL, "createObjectURL");
  Reflect.deleteProperty(URL, "revokeObjectURL");
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("ContextTray", () => {
  it("captures task text and preserves the original while marking it as a constraint", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "search_context") return Promise.resolve([]);
      if (command === "capture_text_context") return Promise.resolve(textItem);
      if (command === "classify_context") {
        return Promise.resolve({ ...textItem, classification: "constraint" });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<ContextTray repositoryId="repository-1" taskId="task-1" />);

    await user.click(screen.getByRole("button", { name: "Capture text" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Title"), textItem.title);
    await user.type(within(dialog).getByLabelText("Text"), textItem.bodyText ?? "");
    await user.click(within(dialog).getByRole("button", { name: "Capture and attach" }));

    expect(await screen.findByText(textItem.bodyText ?? "missing")).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith("capture_text_context", {
      repositoryId: "repository-1",
      taskId: "task-1",
      title: textItem.title,
      text: textItem.bodyText,
    });

    await user.click(screen.getByRole("button", { name: "Use as constraint" }));
    expect(await screen.findByText("constraint")).toBeVisible();
    expect(screen.getByText(textItem.bodyText ?? "missing")).toBeVisible();
  });

  it("attaches and previews an original image chosen from disk", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "search_context") return Promise.resolve([]);
      if (command === "capture_context_bytes") return Promise.resolve(imageItem);
      if (command === "read_context_artifact") {
        return Promise.resolve({ bytes: [137, 80, 78, 71], mimeType: "image/png", originalName: "settings.png" });
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<ContextTray repositoryId="repository-1" taskId="task-1" />);

    const image = new File([new Uint8Array([137, 80, 78, 71])], "settings.png", {
      type: "image/png",
    });
    await user.upload(screen.getByLabelText("Choose context files"), image);

    expect(await screen.findByRole("img", { name: "settings.png" })).toHaveAttribute(
      "src",
      "blob:mars-preview",
    );
    expect(mocks.invoke).toHaveBeenCalledWith("capture_context_bytes", {
      repositoryId: "repository-1",
      taskId: "task-1",
      title: null,
      originalName: "settings.png",
      bytes: [137, 80, 78, 71],
      mimeType: "image/png",
    });
    expect(mocks.invoke).toHaveBeenCalledWith("read_context_artifact", {
      itemId: imageItem.id,
    });
  });
});
