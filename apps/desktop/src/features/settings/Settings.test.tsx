import type { AgentAccount, GrantView, IndexStatus } from "@mars/shared-types";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { Settings } from "./Settings";

const grant: GrantView = {
  id: "grant-1",
  taskId: "task-1",
  capability: "run_command",
  scope: { kind: "command", program: "cargo", cwd: "/data/worktrees/x" },
  pattern: "cargo test*",
  decision: "allow_pattern",
  usesRemaining: null,
  grantedBy: "fayeed",
  grantedAt: "2026-09-08T12:00:00Z",
  expiresAt: null,
  revokedAt: null,
  revokedBy: null,
  active: true,
};

const index: IndexStatus = {
  repositoryId: "repository-1",
  rootPath: "/work/mars",
  branch: "main",
  headOid: "511c628",
  indexedHeadOid: "511c628",
  headMatchesIndex: true,
  embeddingModel: "local",
  embeddingDimensions: 384,
  counts: { files: 412, symbols: 5120, edges: 900, chunks: 2400, commits: 300, ignoredPaths: 12 },
  lastRun: null,
  runningRuns: 0,
};

function renderSettings(props: Partial<Parameters<typeof Settings>[0]> = {}) {
  return render(
    <Settings
      agents={[
        {
          id: "claude",
          name: "Claude",
          probe: {
            kind: "claude",
            executable: "/usr/bin/claude",
            version: "1.4.0",
            supportsResume: true,
          },
          error: null,
        },
        { id: "codex", name: "Codex", probe: null, error: "codex is not on PATH" },
      ]}
      agentsState="idle"
      connection={null}
      grants={[grant]}
      grantsState="idle"
      index={index}
      onConnectGitHub={vi.fn()}
      onReindex={vi.fn()}
      onRevokeGrant={vi.fn()}
      repositoryName="mars"
      storage={{
        database: "/data/mars.sqlite3",
        worktrees: "/data/worktrees",
        artifacts: "/data/artifacts",
        skills: "/data/skills",
      }}
      {...props}
    />,
  );
}

describe("Settings agents", () => {
  it("reports the version that answered, and says when one did not", () => {
    renderSettings();

    expect(screen.getByText("1.4.0")).toBeVisible();
    expect(screen.getByText("not found on PATH")).toBeVisible();
    expect(screen.getByText("codex is not on PATH")).toBeVisible();
  });

  it("says who a run bills", () => {
    renderSettings({ agents: [claudeWith(account())] });

    expect(screen.getByText("Runs bill your Claude Team plan.")).toBeVisible();
  });

  it("offers the agent's own sign-in when it is not signed in", async () => {
    const user = userEvent.setup();
    const onSignIn = vi.fn();
    renderSettings({
      agents: [claudeWith(account({ billing: { type: "signedOut" }, signIn: { type: "signedOut" } }))],
      onSignIn,
    });

    expect(screen.getByText("not signed in")).toBeVisible();
    expect(screen.getByText("claude auth login")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(onSignIn).toHaveBeenCalledWith("claude");
  });

  it("names a key that outranks the sign-in, and offers the sign-in instead", async () => {
    const user = userEvent.setup();
    const onUseSignIn = vi.fn();
    renderSettings({
      agents: [
        claudeWith(
          account({
            billing: { type: "apiKey", source: "ANTHROPIC_API_KEY" },
            overrides: ["ANTHROPIC_API_KEY"],
          }),
        ),
      ],
      onUseSignIn,
    });

    expect(screen.getByText("Runs bill the API key in ANTHROPIC_API_KEY.")).toBeVisible();
    expect(screen.getByText(/outranks your sign-in/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use your Claude Team plan instead" }));
    expect(onUseSignIn).toHaveBeenCalledWith("claude", true);
  });

  it("lets a person go back to the key once the sign-in was chosen", async () => {
    const user = userEvent.setup();
    const onUseSignIn = vi.fn();
    renderSettings({
      agents: [claudeWith(account({ overrides: ["ANTHROPIC_API_KEY"], useSignIn: true }))],
      onUseSignIn,
    });

    expect(screen.getByText(/leaves ANTHROPIC_API_KEY out/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Use ANTHROPIC_API_KEY instead" }));
    expect(onUseSignIn).toHaveBeenCalledWith("claude", false);
  });
});

function account(overrides: Partial<AgentAccount> = {}): AgentAccount {
  return {
    kind: "claude",
    billing: { type: "plan", plan: "team" },
    signIn: { type: "plan", plan: "team" },
    overrides: [],
    useSignIn: false,
    signInCommand: "claude auth login",
    ...overrides,
  };
}

function claudeWith(agentAccount: AgentAccount) {
  return {
    id: "claude",
    name: "Claude",
    probe: { kind: "claude" as const, executable: "claude", version: "2.1.292", supportsResume: true },
    account: agentAccount,
    error: null,
  };
}

describe("Settings: the revocation surface screen 01 promised", () => {
  it("lists a standing grant with who gave it and what it covers", () => {
    renderSettings();

    expect(screen.getByText("run_command")).toBeVisible();
    expect(screen.getByText("cargo test*")).toBeVisible();
    expect(screen.getByText(/cargo in \/data\/worktrees\/x · granted by fayeed/)).toBeVisible();
  });

  it("revokes a grant", async () => {
    const user = userEvent.setup();
    const onRevokeGrant = vi.fn();
    renderSettings({ onRevokeGrant });

    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(onRevokeGrant).toHaveBeenCalledWith(grant);
  });

  it("still lists a grant that no longer stands, so the history is not lost", () => {
    renderSettings({
      grants: [{ ...grant, active: false, revokedAt: "2026-09-08T13:00:00Z", revokedBy: "fayeed" }],
    });

    expect(screen.getByText("No longer standing")).toBeVisible();
    expect(screen.getByText(/revoked by fayeed/)).toBeVisible();
  });

  it("says plainly when nothing has ever been granted", () => {
    renderSettings({ grants: [] });

    expect(screen.getByText(/No grant has ever been made/)).toBeVisible();
  });
});

describe("Settings indexing and data", () => {
  it("states what the index has read and whether it still matches", () => {
    renderSettings();

    expect(screen.getByText(/412 files · 5,120 symbols/)).toBeVisible();
    expect(screen.getByText("matches HEAD")).toBeVisible();
  });

  it("says a moved tree is stale rather than broken", () => {
    renderSettings({ index: { ...index, headMatchesIndex: false } });

    expect(screen.getByText("the tree moved since the run")).toBeVisible();
  });

  it("names every path, and says none of them are in the repository", () => {
    renderSettings();

    expect(screen.getByText("/data/mars.sqlite3")).toBeVisible();
    expect(screen.getByText(/All of it is outside the repository/)).toBeVisible();
  });

  it("re-indexes only when asked", async () => {
    const user = userEvent.setup();
    const onReindex = vi.fn();
    renderSettings({ onReindex });

    await user.click(screen.getByRole("button", { name: /Re-index now/ }));
    expect(onReindex).toHaveBeenCalled();
  });
});

describe("Settings credentials", () => {
  it("says where tokens are kept, and offers to connect when none is", () => {
    renderSettings();

    expect(screen.getByText(/operating system's credential store/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Connect GitHub" })).toBeVisible();
  });
});
