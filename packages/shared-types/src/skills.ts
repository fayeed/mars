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
  scope: SkillScope;
  installPath: string;
  pinnedHash: string;
  skillMarkdown: string;
  allowClaude: boolean;
  allowCodex: boolean;
}
