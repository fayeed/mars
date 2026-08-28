import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { OpenRepository } from "./OpenRepository";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openDialog: vi.fn(),
}));

vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));
vi.mock(import("@tauri-apps/plugin-dialog"), () => ({ open: mocks.openDialog }));

const candidate = {
  name: "mars-fixture",
  rootPath: "/work/mars-fixture",
  gitCommonDir: "/work/mars-fixture/.git",
  capabilities: ["readFiles", "indexCode", "observeGit"],
} as const;

const project = {
  projectId: "project-1",
  repositoryId: "repository-1",
  name: candidate.name,
  rootPath: candidate.rootPath,
  gitCommonDir: candidate.gitCommonDir,
};

afterEach(() => {
  vi.clearAllMocks();
});

describe("OpenRepository", () => {
  it("previews capabilities before trusting and opening a repository", async () => {
    const user = userEvent.setup();
    const onOpened = vi.fn();
    mocks.openDialog.mockResolvedValue(candidate.rootPath);
    mocks.invoke
      .mockResolvedValueOnce({ status: "trustRequired", repository: candidate })
      .mockResolvedValueOnce({ status: "opened", repository: project });
    render(<OpenRepository onOpened={onOpened} />);

    await user.click(screen.getByRole("button", { name: "Open repository" }));

    expect(await screen.findByRole("heading", { name: "Trust this repository?" })).toBeVisible();
    expect(screen.getByText(candidate.rootPath)).toBeVisible();
    expect(screen.getByText("Read source files")).toBeVisible();
    expect(screen.getByText("Build local code index")).toBeVisible();
    expect(screen.getByText("Read Git metadata")).toBeVisible();
    expect(mocks.invoke).toHaveBeenNthCalledWith(1, "open_repository", {
      path: candidate.rootPath,
      trust: "requireConfirmation",
    });

    await user.click(screen.getByRole("button", { name: "Trust and open" }));

    expect(mocks.invoke).toHaveBeenNthCalledWith(2, "open_repository", {
      path: candidate.rootPath,
      trust: "trust",
    });
    expect(onOpened).toHaveBeenCalledWith(project);
  });

  it("does not inspect when directory selection is cancelled", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(null);
    render(<OpenRepository onOpened={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Open repository" }));

    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("shows native repository errors without opening the trust dialog", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue("/work/plain");
    mocks.invoke.mockRejectedValue({
      code: "repository.not_git",
      message: "directory is not a Git repository: /work/plain",
    });
    render(<OpenRepository onOpened={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Open repository" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("directory is not a Git repository");
    expect(screen.queryByRole("heading", { name: "Trust this repository?" })).not.toBeInTheDocument();
  });

  it("keeps trust failures visible inside the confirmation dialog", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(candidate.rootPath);
    mocks.invoke
      .mockResolvedValueOnce({ status: "trustRequired", repository: candidate })
      .mockRejectedValueOnce({
        code: "repository.storage",
        message: "local project database is unavailable",
      });
    render(<OpenRepository onOpened={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: "Open repository" }));
    const dialog = await screen.findByRole("dialog");

    await user.click(within(dialog).getByRole("button", { name: "Trust and open" }));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "local project database is unavailable",
    );
  });
});
