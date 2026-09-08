export type RepositoryCapability = "readFiles" | "indexCode" | "observeGit";

export interface RepositoryCandidate {
  name: string;
  rootPath: string;
  gitCommonDir: string;
  capabilities: RepositoryCapability[];
}

export interface ProjectSnapshot {
  projectId: string;
  repositoryId: string;
  name: string;
  rootPath: string;
  gitCommonDir: string;
}

export type OpenRepositoryOutcome =
  | { status: "trustRequired"; repository: RepositoryCandidate }
  | { status: "opened"; repository: ProjectSnapshot };

export interface CommandFailure {
  code: string;
  message: string;
}

/** One class of file trust will not put in front of an agent, and why. */
export interface SkipRule {
  label: string;
  detail: string;
}

/**
 * Where Mars keeps what it learns about a repository. Every path is inside the
 * app data directory; nothing is written into the repository itself.
 */
export interface StoragePaths {
  database: string;
  worktrees: string;
  artifacts: string;
  skills: string;
}

/**
 * What a person is shown before they decide whether to trust a repository.
 *
 * A field Git could not answer is `null` rather than a guess, so the screen can
 * say "could not be read" instead of showing a plausible wrong value.
 */
export interface RepositoryPreview {
  remote: string | null;
  branch: string | null;
  headOid: string | null;
  headSummary: string | null;
  /** Files Git tracks. The size of the repository, not of the index. */
  trackedFiles: number | null;
  /** Files the indexer would actually read, after ignore rules are applied. */
  indexableFiles: number | null;
  skipped: SkipRule[];
  storage: StoragePaths;
}
