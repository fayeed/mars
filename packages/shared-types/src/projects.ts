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
