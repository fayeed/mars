export interface SkillCandidate {
  name: string;
  slug: string;
  source: string;
  installs: number;
}

export interface InspectedSkill {
  id: string;
  name: string;
  source: string;
  slug: string;
  installs: number;
  contentHash: string;
  registryHash: string;
  /** Whether the files hash to the digest the registry published with them. */
  registryVerified: boolean;
  trustState: "inspected" | "trusted" | "blocked";
  skillMarkdown: string;
  files: string[];
  changedFiles: string[];
}

export type SkillScope = "global" | "project" | "task" | "workflow";

export interface SkillInstallation {
  id: string;
  skillId: string;
  name: string;
  source: string;
  /** The registry's path segment, to look for updates with. */
  slug: string;
  scope: SkillScope;
  repositoryId: string | null;
  taskId: string | null;
  installPath: string;
  pinnedHash: string;
  /** The instructions at the pinned version: what an agent is given. */
  skillMarkdown: string;
  /** The one-line description from the skill's front matter. */
  description: string;
  enabled: boolean;
  /** Blocked after it was installed; it reaches no agent. */
  blocked: boolean;
  allowClaude: boolean;
  allowCodex: boolean;
  installedAt: string;
}

/** A newer version than an installation pins, found but not installed. */
export interface SkillUpdate {
  installationId: string;
  pinnedHash: string;
  latestHash: string;
  /** Empty when the installation is current. */
  changedFiles: string[];
}

export type LocalSkillOrigin =
  | "repository_claude"
  | "repository_agents"
  | "repository_shared"
  | "user_claude"
  | "user_codex";

/** A skill already on this machine, which the CLIs load themselves. */
export interface LocalSkill {
  name: string;
  description: string;
  origin: LocalSkillOrigin;
  path: string;
  skillMarkdown: string;
  loadedBy: ("claude" | "codex")[];
}
