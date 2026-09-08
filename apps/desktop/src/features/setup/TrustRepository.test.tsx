import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { TrustRepository } from "./TrustRepository";

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

const preview = {
  remote: "git@github.com:mars/fixture.git",
  branch: "main",
  headOid: "511c628aa9c1d4f0e7b2",
  headSummary: "refactor: replace RepositoryWorkspace",
  trackedFiles: 1204,
  indexableFiles: 412,
  skipped: [{ label: "Secrets", detail: ".env files, private keys, certificates" }],
  storage: {
    database: "/data/mars.sqlite3",
    worktrees: "/data/worktrees",
    artifacts: "/data/artifacts",
    skills: "/data/skills",
  },
};

/** Answer by command name; the preview is fired off without being awaited. */
function respond(overrides: { preview?: () => Promise<unknown> } = {}) {
  mocks.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
    if (command === "preview_repository") {
      return overrides.preview ? overrides.preview() : Promise.resolve(preview);
    }
    if (command === "open_repository") {
      return Promise.resolve(
        args.trust === "trust"
          ? { status: "opened", repository: project }
          : { status: "trustRequired", repository: candidate },
      );
    }
    throw new Error(`unexpected command ${command}`);
  });
}

async function reachTheGate() {
  const user = userEvent.setup();
  mocks.openDialog.mockResolvedValue(candidate.rootPath);
  await user.click(screen.getByRole("button", { name: "Open repository" }));
  return user;
}

afterEach(() => {
  vi.clearAllMocks();
});

describe("TrustRepository disclosure", () => {
  it("shows the remote, branch, commit and what would be indexed before trust is given", async () => {
    respond();
    render(<TrustRepository onOpened={vi.fn()} />);
    await reachTheGate();

    expect(await screen.findByText("git@github.com:mars/fixture.git")).toBeVisible();
    expect(screen.getByText("main")).toBeVisible();
    // The commit is shown short, next to its subject.
    expect(screen.getByText("511c628")).toBeVisible();
    expect(screen.getByText("refactor: replace RepositoryWorkspace")).toBeVisible();
    expect(screen.getByText("412")).toBeVisible();
    expect(screen.getByText("1,204")).toBeVisible();
  });

  it("names where it will store what it learns, and that trust was not given yet", async () => {
    respond();
    render(<TrustRepository onOpened={vi.fn()} />);
    await reachTheGate();

    expect(await screen.findByText("/data/mars.sqlite3")).toBeVisible();
    expect(screen.getByText("/data/worktrees")).toBeVisible();
    // Nothing has been trusted: only the inspection and the preview have run.
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "open_repository",
      expect.objectContaining({ trust: "trust" }),
    );
  });

  it("states the capabilities trust grants and the ones it withholds", async () => {
    respond();
    render(<TrustRepository onOpened={vi.fn()} />);
    await reachTheGate();

    expect(await screen.findByText("Build a local code index")).toBeVisible();
    expect(screen.getByText("Run commands")).toBeVisible();
    expect(screen.getByText(/Each of those stops the agent and asks/)).toBeVisible();
  });

  it("says the preview failed rather than showing an empty repository", async () => {
    respond({ preview: () => Promise.reject(new Error("git is not on PATH")) });
    render(<TrustRepository onOpened={vi.fn()} />);
    await reachTheGate();

    expect(await screen.findByText("The repository could not be read")).toBeVisible();
    // The decision is still available; it is the disclosure that failed.
    expect(screen.getByRole("button", { name: /Trust and index/ })).toBeEnabled();
  });
});

describe("TrustRepository outcomes", () => {
  it("trusts and asks for an index", async () => {
    respond();
    const onOpened = vi.fn();
    render(<TrustRepository onOpened={onOpened} />);
    const user = await reachTheGate();

    await user.click(await screen.findByRole("button", { name: /Trust and index/ }));

    expect(mocks.invoke).toHaveBeenCalledWith("open_repository", {
      path: candidate.rootPath,
      trust: "trust",
    });
    expect(onOpened).toHaveBeenCalledWith({ project, indexing: "requested" });
  });

  it("opens read-only without asking for an index", async () => {
    respond();
    const onOpened = vi.fn();
    render(<TrustRepository onOpened={onOpened} />);
    const user = await reachTheGate();

    await user.click(await screen.findByRole("button", { name: "Open read-only" }));

    expect(onOpened).toHaveBeenCalledWith({ project, indexing: "deferred" });
  });

  it("says where the decision is undone", async () => {
    respond();
    render(<TrustRepository onOpened={vi.fn()} />);
    await reachTheGate();

    expect(await screen.findByText(/Settings › Repository trust/)).toBeVisible();
  });
});
