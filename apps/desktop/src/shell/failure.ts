import type { CommandFailure } from "@mars/shared-types";

import type { ActivityState } from "@/shell/activity-state";

/**
 * Why something did not work, in the few kinds a person can act on.
 *
 * The core reports about a hundred error codes, which is right for a log and
 * useless on a screen: "adapter.probe_failed" and "agent.not_running" are the
 * same sentence to the reader — *the agent is not there* — and want the same
 * next move. These are the classes that differ in what the person should do
 * about them, and nothing finer.
 *
 * `unknown` is not a failure of this list. Most codes genuinely have no better
 * advice attached, and inventing some would be worse than handing over the
 * message the core actually sent.
 */
export type FailureCause =
  | "agentUnavailable"
  | "agentSignedOut"
  | "indexCold"
  | "permissionDenied"
  | "networkOffline"
  | "notFound"
  | "conflict"
  | "unknown";

export interface ClassifiedFailure {
  cause: FailureCause;
  /** The core's code, kept so a report can name the exact condition. */
  code: string | null;
  /** The core's own words. Always shown; never replaced by ours. */
  message: string;
}

/**
 * Code prefixes and exact codes, most specific first.
 *
 * Matched against the code rather than the message: messages are prose meant
 * for a person and will be rewritten, codes are the contract.
 */
const CAUSES: { cause: FailureCause; codes: string[] }[] = [
  {
    cause: "permissionDenied",
    codes: [
      "permission.",
      "skill.blocked",
      "forge.approval_required",
      "plan.blocked",
      "plan.not_approved",
      "context.quarantined",
      "file.ignored",
      "file.outside_repository",
      "index.outside_repository",
      "skill.unsafe_path",
    ],
  },
  {
    cause: "networkOffline",
    codes: ["forge.http", "forge.request", "forge.credential", "skill.network"],
  },
  {
    cause: "agentUnavailable",
    codes: ["adapter.", "agent.not_running", "agent.process", "agent.missing_stream"],
  },
  { cause: "agentSignedOut", codes: ["agent.signed_out"] },
  { cause: "indexCold", codes: ["index.", "brain.symbol_not_found"] },
  {
    cause: "conflict",
    codes: [
      "context_pack.changed",
      "plan.changed",
      "forge.stale_plan",
      "file.write_conflict",
      "task.dirty_worktree",
      "skill.hash_mismatch",
      "context.digest_mismatch",
    ],
  },
  {
    cause: "notFound",
    codes: [
      "task.not_found",
      "plan.not_found",
      "run.not_found",
      "agent.session_not_found",
      "context.not_found",
      "forge.pr_not_found",
      "terminal.not_found",
      "cleanup.not_found",
      "skill.installation_not_found",
    ],
  },
];

function codeOf(error: unknown): string | null {
  if (typeof error === "object" && error !== null && "code" in error) {
    const code = (error as CommandFailure).code;
    return typeof code === "string" && code.length > 0 ? code : null;
  }
  return null;
}

export function failureMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as CommandFailure).message);
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

export function classifyFailure(error: unknown): ClassifiedFailure {
  const code = codeOf(error);
  const message = failureMessage(error);
  if (!code) return { cause: "unknown", code: null, message };
  for (const entry of CAUSES) {
    if (entry.codes.some((prefix) => code === prefix || code.startsWith(prefix))) {
      return { cause: entry.cause, code, message };
    }
  }
  return { cause: "unknown", code, message };
}

export interface FailureCopy {
  state: ActivityState;
  title: string;
  /** What happened, and — the part that matters — what it did not do. */
  description: string;
  /** The move that could resolve it, or `null` when there is no honest one. */
  remedy: string | null;
}

/**
 * How each cause reads, everywhere.
 *
 * Two rules hold across all of them. Each says what was *not* changed, because
 * the most useful thing a failure can tell someone is how much of their work
 * survived it. And none of them offers a retry the shell cannot actually
 * perform — a remedy that does nothing is worse than an honest dead end.
 */
const COPY: Record<FailureCause, Omit<FailureCopy, "state"> & { state: ActivityState }> = {
  agentUnavailable: {
    state: "failed",
    title: "The agent is not available",
    description:
      "Mars runs the agent CLIs that are installed on this machine; it does not bundle them. " +
      "Nothing was started, and nothing in the worktree changed.",
    remedy: "Check installed agents",
  },
  agentSignedOut: {
    state: "blocked",
    title: "The agent is not signed in",
    description:
      "Mars runs the agent CLI with your own account and never sees how you sign in to it. " +
      "Nothing was started, and nothing in the worktree changed.",
    remedy: "Sign in",
  },
  indexCold: {
    state: "stale",
    title: "The index cannot answer this yet",
    description:
      "Search, symbols and Project Brain read from the local index. Anything not yet indexed " +
      "cannot appear in an answer, however well it matches.",
    remedy: "Index this repository",
  },
  permissionDenied: {
    state: "blocked",
    title: "That needs a permission it does not have",
    description:
      "Every side effect passes the permission broker, and agent output cannot grant itself " +
      "anything. Nothing was done.",
    remedy: "Review permissions",
  },
  networkOffline: {
    state: "failed",
    title: "The forge could not be reached",
    description:
      "Only the intent was recorded. No issue, pull request or comment was created, and Mars " +
      "will not claim one was until the provider confirms it.",
    remedy: "Try again",
  },
  notFound: {
    state: "failed",
    title: "That is no longer there",
    description:
      "The record it pointed at is gone or was never written. Nothing was changed looking for it.",
    remedy: "Reload",
  },
  conflict: {
    state: "stale",
    title: "Something moved first",
    description:
      "What was approved is not what is on disk now, so the action was refused rather than " +
      "applied to a different state than the one it was agreed against.",
    remedy: "Rebuild and review it again",
  },
  unknown: {
    state: "failed",
    title: "That did not work",
    description: "Nothing was changed.",
    remedy: null,
  },
};

export function failureCopy(cause: FailureCause): FailureCopy {
  return COPY[cause];
}

/**
 * Whether the index has read enough to be worth asking.
 *
 * A cold index is not an error and must not be drawn as one: nothing failed,
 * the repository simply has not been read yet, and the move is to read it.
 */
export function isIndexCold(status: { counts?: { files?: number } } | null | undefined): boolean {
  return !status || (status.counts?.files ?? 0) === 0;
}
