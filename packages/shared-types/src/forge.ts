export interface ForgeRepository {
  host: string;
  owner: string;
  name: string;
}

export interface ForgeConnection {
  id: string;
  remoteId: string;
  provider: string;
  apiBase: string;
  accountLogin: string | null;
  credentialRef: string;
}

export interface StoredIssue {
  id: string;
  remoteId: string;
  repositoryId: string;
  providerId: string;
  number: number;
  title: string;
  body: string;
  state: string;
  url: string;
  author: string;
  labels: string[];
  updatedAt: string;
}

export interface PullRequestPlan {
  taskId: string;
  remoteId: string;
  remoteName: string;
  remoteUrl: string;
  pushUrl: string;
  repository: ForgeRepository;
  worktreePath: string;
  head: string;
  base: string;
  headSha: string;
  commitsAhead: number;
  clean: boolean;
  title: string;
  body: string;
  draft: boolean;
  /** Logins to ask for review once the pull request exists. */
  reviewers: string[];
  pushCommand: string;
  fingerprint: string;
}

export interface ReviewComment {
  providerId: string;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  url: string;
  createdAt: string;
}

export interface CheckRun {
  providerId: string;
  name: string;
  status: string;
  conclusion: string | null;
  url: string;
  startedAt: string | null;
  completedAt: string | null;
}

export interface PullRequestSnapshot {
  id: string;
  remoteId: string;
  taskId: string | null;
  issueId: string | null;
  providerId: string;
  number: number;
  title: string;
  body: string;
  state: string;
  url: string;
  head: string;
  base: string;
  draft: boolean;
  mergeable: boolean | null;
  headSha: string;
  updatedAt: string;
  comments: ReviewComment[];
  checks: CheckRun[];
}

export type ForgeMutationStatus = "started" | "completed" | "failed" | "interrupted";

export interface ForgeMutation {
  id: string;
  taskId: string;
  remoteId: string;
  operation: "create_pull_request";
  status: ForgeMutationStatus;
  planFingerprint: string;
  pushApproved: boolean;
  forgeApproved: boolean;
  detailsJson: string;
  error: string | null;
  startedAt: string;
  finishedAt: string | null;
}

/** One commit a push would carry. */
export interface TaskCommit {
  oid: string;
  subject: string;
  author: string;
  date: string;
}

/**
 * A workflow the forge would run, read from the worktree rather than guessed.
 * Nothing is scheduled until the pull request exists.
 */
export interface PlannedCheck {
  /** The workflow file, relative to the repository root. */
  path: string;
  name: string;
  /** True when the file names `pull_request` among its triggers. */
  onPullRequest: boolean;
}
