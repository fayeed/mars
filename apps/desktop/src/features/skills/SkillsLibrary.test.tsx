import type { SkillInstallation } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { SkillsLibrary } from "./SkillsLibrary";

const skill: SkillInstallation = {
  id: "installation-1",
  skillId: "skill-1",
  name: "review",
  scope: "project",
  installPath: "/data/skills/review",
  pinnedHash: "9f8e7d6c5b4a3928",
  skillMarkdown: "Read the diff. Report only defects you can point at.",
  allowClaude: true,
  allowCodex: false,
};

function renderLibrary(props: Partial<Parameters<typeof SkillsLibrary>[0]> = {}) {
  return render(
    <SkillsLibrary
      onRemove={vi.fn()}
      onToggleAdapter={vi.fn()}
      skills={[skill]}
      state="idle"
      {...props}
    />,
  );
}

describe("SkillsLibrary", () => {
  it("shows the full instruction text rather than a description of it", () => {
    renderLibrary();

    expect(
      screen.getByText("Read the diff. Report only defects you can point at."),
    ).toBeVisible();
  });

  it("pins what runs to the content that was inspected", () => {
    renderLibrary();

    expect(screen.getByText("9f8e7d6c5b4a")).toBeVisible();
  });

  it("says when a skill applies, and that Mars does not trigger one itself", () => {
    renderLibrary();

    expect(screen.getByText(/Mars does not trigger a skill on its own/)).toBeVisible();
  });

  it("says what the scope actually means", () => {
    renderLibrary();

    expect(screen.getByText("available to every task in this repository")).toBeVisible();
  });

  it("turns a skill on or off for one agent", async () => {
    const user = userEvent.setup();
    const onToggleAdapter = vi.fn();
    renderLibrary({ onToggleAdapter });

    await user.click(screen.getByRole("button", { name: "codex" }));
    expect(onToggleAdapter).toHaveBeenCalledWith(skill, "codex", true);

    await user.click(screen.getByRole("button", { name: "claude" }));
    expect(onToggleAdapter).toHaveBeenCalledWith(skill, "claude", false);
  });

  it("uninstalls", async () => {
    const user = userEvent.setup();
    const onRemove = vi.fn();
    renderLibrary({ onRemove });

    await user.click(screen.getByRole("button", { name: "Uninstall" }));
    expect(onRemove).toHaveBeenCalledWith(skill);
  });

  it("says nothing installs itself when the list is empty", () => {
    renderLibrary({ skills: [] });

    expect(screen.getByText("No skills installed")).toBeVisible();
    expect(screen.getByText(/nothing installs itself/)).toBeVisible();
  });

  it("collapses a skill on request", async () => {
    const user = userEvent.setup();
    renderLibrary();

    await user.click(screen.getByRole("button", { expanded: true }));
    expect(screen.queryByText(skill.skillMarkdown)).not.toBeInTheDocument();
  });
});
