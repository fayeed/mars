/**
 * Source control as the core reports it: the main repository and each task's
 * worktree, read and changed through the person's own Git.
 */

/** Which working directory a source-control command acts on. */
export type GitTarget =
  | { kind: "repository"; repositoryId: string }
  | { kind: "task"; taskId: string };

/** One changed path. Status letters are Git's own: M A D R C T U, or `.` for none. */
export interface GitFileChange {
  path: string;
  originalPath: string | null;
  /** The index against HEAD. */
  staged: string;
  /** The working tree against the index. */
  unstaged: string;
  untracked: boolean;
  conflicted: boolean;
}

export interface RepoStatus {
  root: string;
  /** Null when HEAD is detached. */
  branch: string | null;
  headOid: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  changes: GitFileChange[];
  /** A merge, rebase, cherry-pick or revert part-way through. */
  operation: string | null;
}

export interface CommitInfo {
  oid: string;
  shortOid: string;
  subject: string;
  author: string;
  authoredAt: string;
  refs: string[];
}

export interface BranchInfo {
  name: string;
  current: boolean;
  upstream: string | null;
  oid: string;
  subject: string;
  committedAt: string;
  /** Checked out in another worktree, which Git will not switch to here. */
  checkedOutElsewhere: boolean;
}

export interface StashEntry {
  reference: string;
  message: string;
}

export type IntegrationStrategy = "merge" | "squash" | "fast_forward";

/** What merging a task's branch into its base would do, worked out without touching either. */
export interface IntegrationPreview {
  root: string;
  base: string;
  baseOid: string;
  head: string;
  headOid: string;
  commits: CommitInfo[];
  /** `name-status` lines between the base and the head. */
  filesChanged: string[];
  /** Files that would conflict. Empty means a clean merge. */
  conflicts: string[];
  fastForward: boolean;
  /** Why it cannot be applied as things stand. */
  blockers: string[];
  fingerprint: string;
}

/** The most recent removal of a task's worktree, which can be restored. */
export interface CleanupView {
  id: string;
  removedPath: string;
  branchName: string;
  archivedRef: string | null;
  state: string;
}
