import type { InspectedSkill, SkillCandidate, SkillInstallation } from "@mars/shared-types";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SkillBrowser } from "./SkillBrowser";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock(import("@tauri-apps/api/core"), () => ({ invoke: mocks.invoke }));

const candidate: SkillCandidate = {
  name: "react-review",
  slug: "vercel-labs/agent-skills/react-review",
  source: "vercel-labs/agent-skills",
  installs: 1200,
};

const inspected: InspectedSkill = {
  id: "skill-1",
  name: candidate.name,
  slug: candidate.slug,
  source: candidate.source,
  installs: candidate.installs,
  contentHash: "abcdef0123456789abcdef0123456789",
  registryHash: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
  registryVerified: true,
  trustState: "inspected",
  skillMarkdown: "# React review\n\nCheck effects and boundaries.",
  files: ["SKILL.md", "references/react.md"],
  changedFiles: ["SKILL.md", "references/react.md"],
};

const installation: SkillInstallation = {
  id: "installation-1",
  skillId: inspected.id,
  name: inspected.name,
  source: inspected.source,
  slug: "react-review",
  scope: "task",
  repositoryId: "repo",
  taskId: "task",
  installPath: "/skills/store/abcdef012345",
  pinnedHash: inspected.contentHash,
  skillMarkdown: inspected.skillMarkdown,
  description: "Check effects and boundaries",
  enabled: true,
  blocked: false,
  allowClaude: true,
  allowCodex: true,
  installedAt: "2026-10-09T10:00:00Z",
};

afterEach(() => vi.clearAllMocks());

describe("SkillBrowser", () => {
  it("separates discovery, SKILL.md inspection, explicit trust, and removal", async () => {
    const user = userEvent.setup();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_task_skills") return Promise.resolve([]);
      if (command === "search_skills") return Promise.resolve([candidate]);
      if (command === "inspect_skill") return Promise.resolve(inspected);
      if (command === "install_skill") return Promise.resolve(installation);
      if (command === "remove_skill_installation") return Promise.resolve(undefined);
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    });
    render(<SkillBrowser repositoryId="repository-1" taskId="task-1" />);

    await user.click(await screen.findByRole("button", { name: "Find skills" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Search skills.sh"), "react review");
    await user.click(within(dialog).getByRole("button", { name: "Search" }));
    await user.click(await within(dialog).findByRole("button", { name: /react-review/ }));

    expect(await within(dialog).findByText(/Check effects and boundaries/)).toBeVisible();
    expect(within(dialog).getByText("references/react.md")).toBeVisible();
    expect(mocks.invoke).not.toHaveBeenCalledWith("install_skill", expect.anything());

    await user.click(within(dialog).getByRole("button", { name: "Trust, pin, and install" }));
    expect(await screen.findByText("abcdef012345 · task")).toBeVisible();
    expect(mocks.invoke).toHaveBeenCalledWith("install_skill", {
      skillId: inspected.id,
      repositoryId: "repository-1",
      taskId: "task-1",
      scope: "task",
      allowClaude: true,
      allowCodex: true,
    });

    await user.click(screen.getByRole("button", { name: "Remove react-review" }));
    expect(screen.queryByText("abcdef012345 · task")).not.toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("remove_skill_installation", {
      installationId: installation.id,
    });
  });
});
