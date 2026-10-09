import type { RunRecord, WorktreeState } from "@mars/shared-types";

/**
 * What one message in the composer does.
 *
 * The same words do very different things depending on where they are sent,
 * so the decision is made in one place, from facts the screen already holds,
 * and can be read and tested apart from the screen.
 */
export type MessageRoute =
  /** A question about the repository. It never becomes work. */
  | { kind: "ask"; question: string }
  /** A new task's plan: its own branch and worktree, approved before anything runs. */
  | { kind: "new-task" }
  /** An agent is working in this task; the message waits for it to finish. */
  | { kind: "queue" }
  /** The task's plan is not approved yet; the message refines what will be asked. */
  | { kind: "refine-plan" }
  /** No agent has worked here; the message is the assignment, shown as a pack first. */
  | { kind: "first-run" }
  /** The task's agent has a session; the message continues it. */
  | { kind: "continue" };

export interface RouteInput {
  text: string;
  /** "New task" was chosen in the composer. */
  composeNew: boolean;
  /** The task open in the conversation, if any. */
  task: { taskId: string; state: WorktreeState } | null;
  /** Whether an agent is running in the open task right now. */
  working: boolean;
  /** The open task's runs, as the core records them. */
  runs: Pick<RunRecord, "providerSessionId">[];
}

export function routeMessage({ composeNew, runs, task, text, working }: RouteInput): MessageRoute {
  const asked = /^\/ask\s+/.exec(text);
  if (asked) return { kind: "ask", question: text.slice(asked[0].length) };
  if (composeNew || !task) return { kind: "new-task" };
  if (working) return { kind: "queue" };
  if (task.state !== "ready") return { kind: "refine-plan" };
  if (runs.some((run) => Boolean(run.providerSessionId))) return { kind: "continue" };
  return { kind: "first-run" };
}
