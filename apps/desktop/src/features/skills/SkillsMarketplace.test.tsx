import type { InspectedSkill, LocalSkill, SkillInstallation } from "@mars/shared-types";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { SkillsMarketplace, formatInstalls } from "./SkillsMarketplace";

const installation: SkillInstallation = {
  id: "inst-1",
  skillId: "skill-1",
  name: "web-design-guidelines",
  source: "vercel-labs/agent-skills",
  slug: "web-design-guidelines",
  scope: "project",
  repositoryId: "repo",
  taskId: null,
  installPath: "/data/skills/store/abc",
  pinnedHash: "abcdef0123456789",
  skillMarkdown: "---\nname: web-design-guidelines\n---\n# Pinned text\n",
  description: "Review UI code",
  enabled: true,
  blocked: false,
  allowClaude: true,
  allowCodex: false,
  installedAt: "2026-10-09T10:00:00Z",
};

const inspected: InspectedSkill = {
  id: "skill-2",
  name: "pdf",
  source: "anthropics/skills",
  slug: "pdf",
  installs: 120_000,
  contentHash: "1234567890abcdef1234",
  registryHash: "ff",
  registryVerified: true,
  trustState: "inspected",
  skillMarkdown: "# PDF skill\nRead the whole thing.",
  files: ["SKILL.md", "scripts/fill.py"],
  changedFiles: ["SKILL.md", "scripts/fill.py"],
};

const local: LocalSkill = {
  name: "lint",
  description: "Run the linters",
  origin: "repository_claude",
  path: "/repo/.claude/skills/lint",
  skillMarkdown: "# lint",
  loadedBy: ["claude"],
};

function routes(overrides: Record<string, unknown> = {}) {
  const table: Record<string, unknown> = {
    repository_skills: [installation],
    local_skills: [local],
    search_skills: [{ name: "pdf", slug: "pdf", source: "anthropics/skills", installs: 120_000 }],
    inspect_skill: inspected,
    install_skill: { ...installation, id: "inst-2", name: "pdf" },
    set_skill_enabled: installation,
    set_skill_adapter: installation,
    check_skill_update: {
      installationId: "inst-1",
      pinnedHash: "abcdef",
      latestHash: "999999",
      changedFiles: ["SKILL.md"],
    },
    remove_skill_installation: null,
    block_skill: null,
    ...overrides,
  };
  invoke.mockImplementation((command: string) =>
    command in table ? Promise.resolve(table[command]) : Promise.reject(new Error(command)),
  );
}

describe("SkillsMarketplace", () => {
  beforeEach(() => {
    invoke.mockReset();
    routes();
  });

  it("searches, shows the whole skill, and installs it at the chosen scope", async () => {
    const user = userEvent.setup();
    const onChanged = vi.fn();
    render(
      <SkillsMarketplace
        onChanged={onChanged}
        repositoryId="repo"
        taskId="task-1"
        taskTitle="Fix forms"
      />,
    );
    await user.click(screen.getByRole("button", { name: "testing" }));
    expect(screen.getByRole("textbox", { name: "Search skills" })).toHaveValue("testing");

    const results = await screen.findByRole("list", { name: "Search results" });
    await user.click(within(results).getByRole("button", { name: /pdf/ }));

    const review = await screen.findByRole("region", { name: "Review pdf" });
    expect(within(review).getByText(/Read the whole thing/)).toBeInTheDocument();
    expect(within(review).getByText("Matches what skills.sh published")).toBeInTheDocument();
    // A task is open, so the narrowest scope is the default.
    expect(within(review).getByRole("radio", { name: "Only Fix forms" })).toBeChecked();
    await user.click(within(review).getByRole("radio", { name: "This repository" }));
    await user.click(within(review).getByRole("checkbox", { name: "Codex" }));
    await user.click(within(review).getByRole("button", { name: /Trust and install/ }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("install_skill", {
        skillId: "skill-2",
        repositoryId: "repo",
        taskId: "task-1",
        scope: "project",
        allowClaude: true,
        allowCodex: false,
      }),
    );
    expect(onChanged).toHaveBeenCalled();
    expect(await screen.findByRole("status")).toHaveTextContent("Installed pdf for this repository");
  });

  it("says so when the files do not match the registry's checksum", async () => {
    routes({ inspect_skill: { ...inspected, registryVerified: false } });
    const user = userEvent.setup();
    render(<SkillsMarketplace repositoryId="repo" taskId={null} />);
    await user.type(screen.getByRole("textbox", { name: "Search skills" }), "pdf");
    const results = await screen.findByRole("list", { name: "Search results" });
    await user.click(within(results).getByRole("button", { name: /pdf/ }));
    expect(await screen.findByText(/Does not match the registry's checksum/)).toBeInTheDocument();
  });

  it("refuses to offer installing a blocked skill", async () => {
    routes({ inspect_skill: { ...inspected, trustState: "blocked" } });
    const user = userEvent.setup();
    render(<SkillsMarketplace repositoryId="repo" taskId={null} />);
    await user.type(screen.getByRole("textbox", { name: "Search skills" }), "pdf");
    const results = await screen.findByRole("list", { name: "Search results" });
    await user.click(within(results).getByRole("button", { name: /pdf/ }));
    expect(await screen.findByText("This skill is blocked")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Trust and install/ })).not.toBeInTheDocument();
  });

  it("manages what is installed: agents, updates, and removal", async () => {
    const user = userEvent.setup();
    render(<SkillsMarketplace repositoryId="repo" taskId={null} />);
    await user.click(screen.getByRole("tab", { name: /Installed/ }));
    const list = await screen.findByRole("list", { name: "Installed skills" });
    expect(within(list).getByText("for this repository")).toBeInTheDocument();

    await user.click(within(list).getByRole("checkbox", { name: "Codex" }));
    expect(invoke).toHaveBeenCalledWith("set_skill_adapter", {
      installationId: "inst-1",
      adapter: "codex",
      allowed: true,
    });

    await user.click(within(list).getByRole("button", { name: "Check for update" }));
    expect(await within(list).findByText(/1 file\(s\) changed: SKILL.md/)).toBeInTheDocument();

    await user.click(within(list).getByRole("button", { name: "Uninstall" }));
    expect(invoke).toHaveBeenCalledWith("remove_skill_installation", { installationId: "inst-1" });
  });

  it("lists the skills the CLIs already load on their own", async () => {
    const user = userEvent.setup();
    render(<SkillsMarketplace repositoryId="repo" taskId={null} />);
    await user.click(screen.getByRole("tab", { name: /On this machine/ }));
    const list = await screen.findByRole("list", { name: "Skills on this machine" });
    expect(within(list).getByText("loaded by claude on its own")).toBeInTheDocument();
    expect(within(list).getByText("Run the linters")).toBeInTheDocument();
  });

  it("formats install counts a person reads at a glance", () => {
    expect(formatInstalls(713_095)).toBe("713k installs");
    expect(formatInstalls(1_250_000)).toBe("1.3M installs");
    expect(formatInstalls(1)).toBe("1 install");
  });
});
